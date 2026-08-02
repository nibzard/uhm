//! HTTP layer for the streaming OpenAI request. Other modules (doctor, telemetry)
//! build their own short-lived ureq agents; all of them opt into env-proxy via
//! `try_proxy_from_env`.

use std::io::Read;
use std::time::Duration;

pub struct Response {
    pub reader: Box<dyn Read + Send>,
}

/// Shared short-request agent policy. Every outbound HTTP caller goes through
/// this constructor so proxy discovery and deadlines cannot drift by module.
pub fn agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .try_proxy_from_env(true)
        .timeout(timeout)
        .build()
}

/// POST `body` (JSON) with bearer auth. Returns a streaming reader on 2xx,
/// or an error message (with the server's message if we can parse it) on 4xx/5xx.
pub fn post_stream(url: &str, auth: &str, body: &str) -> Result<Response, String> {
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
        .set("Accept", "text/event-stream")
        .send_string(body);
    match res {
        Ok(resp) => Ok(Response {
            reader: Box::new(resp.into_reader()),
        }),
        Err(ureq::Error::Status(code, resp)) => {
            let body_text = resp.into_string().unwrap_or_default();
            Err(format_http_error(code, &body_text))
        }
        Err(e) => Err(format!("request failed: {}", e)),
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
