//! HTTP layer for the streaming OpenAI request. Other modules (doctor, telemetry)
//! build their own short-lived ureq agents; all of them opt into env-proxy via
//! `try_proxy_from_env`.

use std::io::Read;
use std::time::Duration;

pub struct Response {
    pub status: u16,
    pub reader: Box<dyn Read + Send>,
}

#[derive(Debug, Clone)]
pub struct HttpError {
    pub kind: crate::provider::ProviderErrorKind,
    pub message: String,
}

/// Shared short-request agent policy. Every outbound HTTP caller goes through
/// this constructor so proxy discovery and deadlines cannot drift by module.
pub fn agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .try_proxy_from_env(true)
        .timeout(timeout)
        .build()
}

pub fn post_stream_typed(
    url: &str,
    auth: &str,
    accept: &str,
    body: &str,
) -> Result<Response, HttpError> {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    let agent = AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .try_proxy_from_env(true)
            .timeout_connect(Duration::from_secs(10))
            .timeout_write(Duration::from_secs(30))
            .timeout_read(Duration::from_secs(120))
            .build()
    });
    let res = agent
        .post(url)
        .set("Authorization", auth)
        .set("Content-Type", "application/json")
        .set("Accept", accept)
        .send_string(body);
    match res {
        Ok(resp) => Ok(Response {
            status: resp.status(),
            reader: Box::new(resp.into_reader()),
        }),
        Err(ureq::Error::Status(code, resp)) => {
            let body_text = resp.into_string().unwrap_or_default();
            Err(HttpError {
                kind: error_kind_for_status(code),
                message: format_http_error(code, &body_text),
            })
        }
        Err(ureq::Error::Transport(error)) => Err(HttpError {
            kind: if error.to_string().to_ascii_lowercase().contains("timed out") {
                crate::provider::ProviderErrorKind::Timeout
            } else {
                crate::provider::ProviderErrorKind::Transient
            },
            message: format!("request failed: {error}"),
        }),
    }
}

fn error_kind_for_status(status: u16) -> crate::provider::ProviderErrorKind {
    match status {
        401 | 403 => crate::provider::ProviderErrorKind::Auth,
        429 => crate::provider::ProviderErrorKind::RateLimited,
        500..=599 => crate::provider::ProviderErrorKind::Transient,
        _ => crate::provider::ProviderErrorKind::RequestRejected,
    }
}

fn format_http_error(status: u16, body: &str) -> String {
    if let Ok(j) = serde_json::from_str::<serde_json::Value>(body.trim()) {
        if let Some(msg) = j
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            return format!(
                "HTTP {}: {}",
                status,
                crate::render::ansi::sanitize_untrusted(msg)
            );
        }
    }
    let short: String = body.chars().take(240).collect();
    format!(
        "HTTP {}: {}",
        status,
        crate::render::ansi::sanitize_untrusted(short.trim())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_errors_are_typed_without_string_matching() {
        assert_eq!(
            error_kind_for_status(401),
            crate::provider::ProviderErrorKind::Auth
        );
        assert_eq!(
            error_kind_for_status(429),
            crate::provider::ProviderErrorKind::RateLimited
        );
        assert_eq!(
            error_kind_for_status(503),
            crate::provider::ProviderErrorKind::Transient
        );
        assert_eq!(
            error_kind_for_status(400),
            crate::provider::ProviderErrorKind::RequestRejected
        );
    }
}
