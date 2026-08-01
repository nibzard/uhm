//! OpenAI Chat Completions client. Builds the JSON body, posts via http.rs,
//! and either streams prose or accumulates the structured-output envelope.

use crate::action::{ProposedAction, WireProposal};
use crate::http;
use crate::{prompt, sse};
use serde_json::{json, Value};

pub struct ApiConfig {
    pub base_url: String,
    pub model: String,
    pub key: String,
    pub max_tokens: u32,
    pub reasoning_effort: String,
}

pub trait Transport {
    fn post(&self, url: &str, authorization: &str, body: &str) -> Result<http::Response, String>;
}

struct NetworkTransport;

impl Transport for NetworkTransport {
    fn post(&self, url: &str, authorization: &str, body: &str) -> Result<http::Response, String> {
        http::post_stream(url, authorization, body)
    }
}

fn endpoint(cfg: &ApiConfig) -> String {
    format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'))
}

fn build_body(
    cfg: &ApiConfig,
    system: &str,
    user: &str,
    stream: bool,
    response_format: Option<Value>,
) -> String {
    let mut body = json!({
        "model": cfg.model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user}
        ],
        "max_completion_tokens": cfg.max_tokens,
        "reasoning_effort": cfg.reasoning_effort,
        "stream": stream
    });
    if let Some(rf) = response_format {
        body["response_format"] = rf;
    }
    if stream {
        body["stream_options"] = json!({"include_usage": true});
    }
    serde_json::to_string(&body).expect("API request body is serializable")
}

fn post(cfg: &ApiConfig, body: &str) -> Result<http::Response, String> {
    post_with(&NetworkTransport, cfg, body)
}

fn post_with(
    transport: &dyn Transport,
    cfg: &ApiConfig,
    body: &str,
) -> Result<http::Response, String> {
    if cfg.key.trim().is_empty() {
        return Err("No API key was provided to the OpenAI transport".into());
    }
    let auth = format!("Bearer {}", cfg.key);
    transport.post(&endpoint(cfg), &auth, body)
}

/// Stream prose tokens (ask / explain modes).
pub fn stream_answer<F: FnMut(&str)>(
    cfg: &ApiConfig,
    system: &str,
    user: &str,
    on_token: F,
) -> Result<(), String> {
    let body = build_body(cfg, system, user, true, None);
    let resp = post(cfg, &body)?;
    sse::read_stream(resp.reader, on_token)
}

/// Accumulate the full streamed content (used for the structured envelope).
pub fn collect_answer(
    cfg: &ApiConfig,
    system: &str,
    user: &str,
    response_format: Option<Value>,
    stream: bool,
    on_progress: impl FnMut(&str),
) -> Result<String, String> {
    let body = build_body(cfg, system, user, stream, response_format);
    let resp = post(cfg, &body)?;
    if !stream {
        return read_buffered_response(resp.reader);
    }
    let mut full = String::new();
    let mut f = on_progress;
    sse::read_stream(resp.reader, |t| {
        f(t);
        full.push_str(t);
    })?;
    Ok(full)
}

fn read_buffered_response(mut reader: Box<dyn std::io::Read + Send>) -> Result<String, String> {
    use std::io::Read as _;

    let mut raw = String::new();
    reader
        .read_to_string(&mut raw)
        .map_err(|e| format!("read response: {}", e))?;
    let value: Value =
        serde_json::from_str(raw.trim()).map_err(|e| format!("invalid API response: {}", e))?;
    if let Some(message) = value["error"]["message"].as_str() {
        return Err(format!(
            "API error: {}",
            crate::render::ansi::sanitize_untrusted(message)
        ));
    }
    value
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .map(str::to_string)
        .ok_or_else(|| "API response did not contain message content".into())
}

/// Request the structured-output envelope as raw JSON (for caching).
pub fn request_envelope_raw(
    cfg: &ApiConfig,
    system: &str,
    user: &str,
    stream: bool,
) -> Result<String, String> {
    collect_answer(
        cfg,
        system,
        user,
        Some(prompt::proposal_response_format()),
        stream,
        |_| {},
    )
}

pub fn parse_proposal(raw: &str) -> Result<ProposedAction, String> {
    let wire: WireProposal = serde_json::from_str(raw.trim())
        .map_err(|e| format!("could not parse structured model response: {}", e))?;
    ProposedAction::try_from(wire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffered_response_extracts_message_content() {
        let body = br#"{"choices":[{"message":{"content":"hello"}}]}"#.to_vec();
        let reader: Box<dyn std::io::Read + Send> = Box::new(std::io::Cursor::new(body));
        assert_eq!(read_buffered_response(reader).unwrap(), "hello");
    }

    #[test]
    fn buffered_response_surfaces_api_errors() {
        let body = br#"{"error":{"message":"bad request"}}"#.to_vec();
        let reader: Box<dyn std::io::Read + Send> = Box::new(std::io::Cursor::new(body));
        assert!(read_buffered_response(reader)
            .unwrap_err()
            .contains("bad request"));
    }

    struct FakeTransport;

    impl Transport for FakeTransport {
        fn post(
            &self,
            url: &str,
            authorization: &str,
            _body: &str,
        ) -> Result<http::Response, String> {
            assert_eq!(url, "https://example.test/v1/chat/completions");
            assert_eq!(authorization, "Bearer test-key");
            Ok(http::Response {
                reader: Box::new(std::io::Cursor::new(Vec::<u8>::new())),
            })
        }
    }

    #[test]
    fn model_transport_is_replaceable_without_network() {
        let config = ApiConfig {
            base_url: "https://example.test/v1".into(),
            model: "test".into(),
            key: "test-key".into(),
            max_tokens: 10,
            reasoning_effort: "low".into(),
        };
        assert!(post_with(&FakeTransport, &config, "{}").is_ok());
    }
}
