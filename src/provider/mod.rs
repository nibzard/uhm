//! Provider-neutral model invocation and decoded tool-call boundary.

pub mod cerebras;
pub mod openai;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

pub const ADAPTER_CONTRACT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Openai,
    Cerebras,
}

impl ProviderId {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "openai" => Ok(Self::Openai),
            "cerebras" => Ok(Self::Cerebras),
            _ => Err("provider must be openai or cerebras".into()),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Cerebras => "cerebras",
        }
    }

    pub const fn adapter(self) -> &'static dyn ProviderAdapter {
        match self {
            Self::Openai => &openai::OpenAiAdapter,
            Self::Cerebras => &cerebras::CerebrasAdapter,
        }
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone)]
pub struct Authorization(String);

impl Authorization {
    pub fn bearer(key: &str) -> Self {
        Self(format!("Bearer {key}"))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderErrorKind {
    Credential,
    Auth,
    RateLimited,
    Transient,
    Timeout,
    RequestRejected,
    Refused,
    Incomplete,
    Malformed,
    UnsupportedCapability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    pub kind: ProviderErrorKind,
    pub message: String,
    pub attempts_consumed: u8,
}

impl ProviderError {
    pub fn before_request(kind: ProviderErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: sanitize(message.into()),
            attempts_consumed: 0,
        }
    }

    pub fn after_request(kind: ProviderErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: sanitize(message.into()),
            attempts_consumed: 1,
        }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProviderError {}

fn sanitize(value: String) -> String {
    crate::render::ansi::sanitize_untrusted(&value)
        .chars()
        .take(512)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct WireCapabilities {
    pub streaming: bool,
    pub reasoning_effort: bool,
    pub strict_schema_bounds: bool,
}

#[derive(Clone)]
pub struct Invocation<'a> {
    pub model: &'a str,
    pub authorization: Authorization,
    pub input: &'a str,
    pub stream: bool,
    pub max_tokens: u32,
    pub reasoning_effort: &'a str,
    pub request_max_bytes: usize,
    pub response_max_bytes: usize,
}

pub struct HttpRequest {
    pub url: &'static str,
    pub authorization: Authorization,
    pub accept: &'static str,
    pub body: String,
}

pub struct HttpResponse {
    pub status: u16,
    pub reader: Box<dyn std::io::Read + Send>,
}

pub trait Transport {
    fn post(&self, request: HttpRequest) -> Result<HttpResponse, ProviderError>;
}

pub struct NetworkTransport;

impl Transport for NetworkTransport {
    fn post(&self, request: HttpRequest) -> Result<HttpResponse, ProviderError> {
        crate::http::post_stream_typed(
            request.url,
            request.authorization.expose(),
            request.accept,
            &request.body,
        )
        .map(|response| HttpResponse {
            status: response.status,
            reader: response.reader,
        })
        .map_err(|error| ProviderError::after_request(error.kind, error.message))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedToolCall {
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderResponse {
    pub call: DecodedToolCall,
    pub provider: ProviderId,
    pub api_family: &'static str,
    pub requested_model: String,
    pub resolved_model: Option<String>,
    pub resolved_fingerprint: Option<String>,
    pub request_id: Option<String>,
    pub finish_reason: Option<String>,
    pub usage: Usage,
    pub adapter_contract_version: u32,
    pub raw: String,
}

pub trait ProviderAdapter: Sync {
    fn id(&self) -> ProviderId;
    fn api_family(&self) -> &'static str;
    fn endpoint(&self) -> &'static str;
    fn credential_env(&self) -> &'static str;
    fn capabilities(&self) -> WireCapabilities;
    fn build_request(&self, invocation: &Invocation<'_>) -> Result<HttpRequest, ProviderError>;
    fn parse_response(
        &self,
        invocation: &Invocation<'_>,
        response: HttpResponse,
    ) -> Result<ProviderResponse, ProviderError>;
    fn parse_cached(&self, model: &str, raw: &str) -> Result<ProviderResponse, ProviderError>;
}

pub fn invoke_with(
    adapter: &dyn ProviderAdapter,
    transport: &dyn Transport,
    invocation: &Invocation<'_>,
) -> Result<ProviderResponse, ProviderError> {
    if invocation.authorization.expose().trim() == "Bearer" {
        return Err(ProviderError::before_request(
            ProviderErrorKind::Credential,
            format!("No API key was provided for {}", adapter.id()),
        ));
    }
    if invocation.input.len() > invocation.request_max_bytes {
        return Err(ProviderError::before_request(
            ProviderErrorKind::RequestRejected,
            format!(
                "model request exceeds configured {} byte limit",
                invocation.request_max_bytes
            ),
        ));
    }
    let request = adapter.build_request(invocation)?;
    if request.body.len() > invocation.request_max_bytes {
        return Err(ProviderError::before_request(
            ProviderErrorKind::RequestRejected,
            "serialized model request exceeds configured byte limit",
        ));
    }
    let response = transport.post(request)?;
    if !(200..300).contains(&response.status) {
        return Err(ProviderError::after_request(
            match response.status {
                401 | 403 => ProviderErrorKind::Auth,
                429 => ProviderErrorKind::RateLimited,
                500..=599 => ProviderErrorKind::Transient,
                _ => ProviderErrorKind::RequestRejected,
            },
            format!("{} returned HTTP {}", adapter.id(), response.status),
        ));
    }
    adapter.parse_response(invocation, response)
}

pub fn validate(response: &ProviderResponse) -> Result<crate::action::ProposedAction, String> {
    crate::contract::decode_and_validate(&response.call.name, response.call.arguments.clone())
}

pub(crate) fn read_bounded(
    mut reader: Box<dyn std::io::Read + Send>,
    max: usize,
    provider: ProviderId,
) -> Result<String, ProviderError> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take((max + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            ProviderError::after_request(
                ProviderErrorKind::Transient,
                format!("read {provider} response: {error}"),
            )
        })?;
    if bytes.len() > max {
        return Err(ProviderError::after_request(
            ProviderErrorKind::Malformed,
            format!("{provider} response exceeded configured {max} byte limit"),
        ));
    }
    String::from_utf8(bytes).map_err(|_| {
        ProviderError::after_request(
            ProviderErrorKind::Malformed,
            format!("{provider} response was not valid UTF-8"),
        )
    })
}

pub(crate) fn arguments(name: &str, value: &Value) -> Result<Value, ProviderError> {
    let raw = value.as_str().ok_or_else(|| {
        ProviderError::after_request(
            ProviderErrorKind::Malformed,
            format!("{name} tool call omitted string arguments"),
        )
    })?;
    if raw.len() > 96 * 1024 {
        return Err(ProviderError::after_request(
            ProviderErrorKind::Malformed,
            "function arguments exceeded 98304 bytes",
        ));
    }
    serde_json::from_str(raw).map_err(|error| {
        ProviderError::after_request(
            ProviderErrorKind::Malformed,
            format!("invalid {name} arguments: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffered_response_limit_is_enforced_after_one_post() {
        let error = read_bounded(
            Box::new(std::io::Cursor::new(b"12345".to_vec())),
            4,
            ProviderId::Cerebras,
        )
        .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::Malformed);
        assert_eq!(error.attempts_consumed, 1);
    }

    #[test]
    fn authorization_is_not_rendered_by_provider_errors() {
        let secret = "provider-secret-sentinel";
        let error = ProviderError::after_request(ProviderErrorKind::Auth, "authentication failed");
        assert!(!format!("{error:?}").contains(secret));
        assert!(!error.to_string().contains(secret));
    }
}
