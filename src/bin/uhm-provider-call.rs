//! Private benchmark bridge that exercises the production provider adapters.

use serde::{Deserialize, Serialize};
use std::io::Read;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    provider: uhm_cli::provider::ProviderId,
    model: String,
    input: String,
    max_tokens: u32,
    reasoning_effort: String,
    request_max_bytes: usize,
    response_max_bytes: usize,
}

#[derive(Serialize)]
struct Output {
    tool: String,
    arguments: serde_json::Value,
    provider: uhm_cli::provider::ProviderId,
    api_family: &'static str,
    requested_model: String,
    resolved_model: Option<String>,
    resolved_fingerprint: Option<String>,
    request_id: Option<String>,
    finish_reason: Option<String>,
    usage: uhm_cli::provider::Usage,
    adapter_contract_version: u32,
}

#[derive(Serialize)]
struct ErrorOutput {
    error: SafeError,
}

#[derive(Serialize)]
struct SafeError {
    kind: uhm_cli::provider::ProviderErrorKind,
    attempts_consumed: u8,
    message: String,
}

fn run() -> Result<(), SafeError> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(512 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|error| SafeError {
            kind: uhm_cli::provider::ProviderErrorKind::RequestRejected,
            attempts_consumed: 0,
            message: format!("read provider bridge input: {error}"),
        })?;
    let envelope: Envelope = serde_json::from_slice(&bytes).map_err(|error| SafeError {
        kind: uhm_cli::provider::ProviderErrorKind::RequestRejected,
        attempts_consumed: 0,
        message: format!("invalid provider bridge input: {error}"),
    })?;
    let variable = envelope.provider.adapter().credential_env();
    let key = std::env::var(variable)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| SafeError {
            kind: uhm_cli::provider::ProviderErrorKind::Credential,
            attempts_consumed: 0,
            message: format!("{variable} is required"),
        })?;
    let invocation = uhm_cli::provider::Invocation {
        model: &envelope.model,
        authorization: uhm_cli::provider::Authorization::bearer(&key),
        input: &envelope.input,
        stream: false,
        max_tokens: envelope.max_tokens,
        reasoning_effort: &envelope.reasoning_effort,
        request_max_bytes: envelope.request_max_bytes,
        response_max_bytes: envelope.response_max_bytes,
    };
    let response = uhm_cli::provider::invoke_with(
        envelope.provider.adapter(),
        &uhm_cli::provider::NetworkTransport,
        &invocation,
    )
    .map_err(|error| SafeError {
        kind: error.kind,
        attempts_consumed: error.attempts_consumed,
        message: error.message,
    })?;
    println!(
        "{}",
        serde_json::to_string(&Output {
            tool: response.call.name,
            arguments: response.call.arguments,
            provider: response.provider,
            api_family: response.api_family,
            requested_model: response.requested_model,
            resolved_model: response.resolved_model,
            resolved_fingerprint: response.resolved_fingerprint,
            request_id: response.request_id,
            finish_reason: response.finish_reason,
            usage: response.usage,
            adapter_contract_version: response.adapter_contract_version,
        })
        .map_err(|error| SafeError {
            kind: uhm_cli::provider::ProviderErrorKind::Malformed,
            attempts_consumed: 1,
            message: error.to_string()
        })?
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        println!(
            "{}",
            serde_json::to_string(&ErrorOutput { error })
                .expect("safe provider error is serializable")
        );
        std::process::exit(1);
    }
}
