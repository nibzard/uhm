//! Shared HTTP transport policy.
//!
//! Every outbound caller uses this module so trust loading, proxy selection,
//! bypass rules, deadlines, and failure classification remain identical.

use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read};
use std::net::IpAddr;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

pub struct Response {
    pub status: u16,
    pub reader: Box<dyn Read + Send>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureStage {
    Configuration,
    ProxyConfiguration,
    ProxyConnection,
    Dns,
    Tcp,
    TlsCertificate,
    TlsHandshake,
    Http,
}

impl FailureStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Configuration => "trust configuration",
            Self::ProxyConfiguration => "proxy configuration",
            Self::ProxyConnection => "proxy connection or CONNECT tunnel",
            Self::Dns => "DNS resolution",
            Self::Tcp => "TCP connection",
            Self::TlsCertificate => "TLS certificate verification",
            Self::TlsHandshake => "TLS handshake",
            Self::Http => "HTTP exchange",
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpError {
    pub kind: crate::provider::ProviderErrorKind,
    pub stage: FailureStage,
    pub message: String,
    pub request_started: bool,
}

impl fmt::Display for HttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for HttpError {}

#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub connect: Duration,
    pub write: Duration,
    pub read: Duration,
}

pub struct Agent {
    inner: ureq::Agent,
    via_proxy: bool,
}

impl Agent {
    pub fn get(&self, url: &str) -> ureq::Request {
        self.inner.get(url)
    }

    pub fn post(&self, url: &str) -> ureq::Request {
        self.inner.post(url)
    }

    pub fn classify_error(&self, error: ureq::Error) -> HttpError {
        classify_error(error, self.via_proxy)
    }
}

impl Timeouts {
    pub fn uniform(timeout: Duration) -> Self {
        Self {
            connect: timeout,
            write: timeout,
            read: timeout,
        }
    }

    const fn provider() -> Self {
        Self {
            connect: Duration::from_secs(10),
            write: Duration::from_secs(30),
            read: Duration::from_secs(120),
        }
    }
}

/// Build an agent for a specific destination. Proxy bypass is destination
/// dependent, so the URL is deliberately part of construction.
pub fn agent_for(url: &str, timeouts: Timeouts) -> Result<Agent, HttpError> {
    let tls = tls_config()?;
    let mut builder = ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .tls_config(tls)
        .timeout_connect(timeouts.connect)
        .timeout_write(timeouts.write)
        .timeout_read(timeouts.read);
    let proxy = proxy_for(url)?;
    let via_proxy = proxy.is_some();
    if let Some(proxy) = proxy {
        builder = builder.proxy(proxy);
    }
    Ok(Agent {
        inner: builder.build(),
        via_proxy,
    })
}

pub fn post_stream_typed(
    url: &str,
    auth: &str,
    accept: &str,
    body: &str,
) -> Result<Response, HttpError> {
    let agent = agent_for(url, Timeouts::provider())?;
    let result = agent
        .post(url)
        .set("Authorization", auth)
        .set("Content-Type", "application/json")
        .set("Accept", accept)
        .send_string(body);
    match result {
        Ok(response) => Ok(Response {
            status: response.status(),
            reader: Box::new(response.into_reader()),
        }),
        Err(ureq::Error::Status(code, response)) => {
            let body_text = response.into_string().unwrap_or_default();
            Err(HttpError {
                kind: error_kind_for_status(code),
                stage: FailureStage::Http,
                message: format_http_error(code, &body_text),
                request_started: true,
            })
        }
        Err(error) => Err(agent.classify_error(error)),
    }
}

fn classify_error(error: ureq::Error, via_proxy: bool) -> HttpError {
    match error {
        ureq::Error::Status(code, _) => HttpError {
            kind: error_kind_for_status(code),
            stage: FailureStage::Http,
            message: format!("HTTP {code}"),
            request_started: true,
        },
        ureq::Error::Transport(error) => classify_transport(&error, via_proxy),
    }
}

fn tls_config() -> Result<Arc<rustls::ClientConfig>, HttpError> {
    static CONFIG: OnceLock<Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    CONFIG
        .get_or_init(build_tls_config)
        .clone()
        .map_err(configuration_error)
}

fn build_tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
    let native = rustls_native_certs::load_native_certs();
    if native.certs.is_empty() {
        let source = if std::env::var_os("SSL_CERT_FILE").is_some()
            || std::env::var_os("SSL_CERT_DIR").is_some()
        {
            "SSL_CERT_FILE/SSL_CERT_DIR"
        } else {
            "platform trust store"
        };
        return Err(format!("{source} did not provide any valid certificates"));
    }
    if !native.errors.is_empty()
        && (std::env::var_os("SSL_CERT_FILE").is_some()
            || std::env::var_os("SSL_CERT_DIR").is_some())
    {
        return Err(format!(
            "SSL_CERT_FILE/SSL_CERT_DIR contained unreadable or malformed certificate data ({} error(s))",
            native.errors.len()
        ));
    }

    let mut roots = rustls::RootCertStore::empty();
    let (accepted, _) = roots.add_parsable_certificates(native.certs);
    if accepted == 0 {
        return Err("configured trust sources contained no usable root certificates".into());
    }

    if let Some(path) = std::env::var_os("UHM_CA_BUNDLE") {
        append_ca_bundle(&mut roots, Path::new(&path))?;
    }

    let config = rustls::ClientConfig::builder_with_provider(
        rustls::crypto::ring::default_provider().into(),
    )
    .with_protocol_versions(&[&rustls::version::TLS12, &rustls::version::TLS13])
    .map_err(|error| format!("configure TLS protocol versions: {error}"))?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(Arc::new(config))
}

fn append_ca_bundle(store: &mut rustls::RootCertStore, path: &Path) -> Result<(), String> {
    let file = File::open(path)
        .map_err(|error| format!("read UHM_CA_BUNDLE {}: {error}", path.display()))?;
    let certificates = rustls_pemfile::certs(&mut BufReader::new(file))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("parse UHM_CA_BUNDLE {}: {error}", path.display()))?;
    if certificates.is_empty() {
        return Err(format!(
            "UHM_CA_BUNDLE {} contained no certificates",
            path.display()
        ));
    }
    let (accepted, rejected) = store.add_parsable_certificates(certificates);
    if accepted == 0 || rejected > 0 {
        return Err(format!(
            "UHM_CA_BUNDLE {} contained unusable certificate data",
            path.display()
        ));
    }
    Ok(())
}

fn configuration_error(message: String) -> HttpError {
    HttpError {
        kind: crate::provider::ProviderErrorKind::Trust,
        stage: FailureStage::Configuration,
        message: format!("request blocked by trust configuration: {message}"),
        request_started: false,
    }
}

fn proxy_for(url: &str) -> Result<Option<ureq::Proxy>, HttpError> {
    let parsed = url::Url::parse(url).map_err(|_| HttpError {
        kind: crate::provider::ProviderErrorKind::RequestRejected,
        stage: FailureStage::Configuration,
        message: "request URL is invalid".into(),
        request_started: false,
    })?;
    let host = parsed.host_str().ok_or_else(|| HttpError {
        kind: crate::provider::ProviderErrorKind::RequestRejected,
        stage: FailureStage::Configuration,
        message: "request URL does not contain a host".into(),
        request_started: false,
    })?;
    let port = parsed.port_or_known_default().unwrap_or(443);
    if no_proxy_matches(host, port) {
        return Ok(None);
    }

    let names: &[&str] = match parsed.scheme() {
        "https" => &[
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
            "HTTP_PROXY",
            "http_proxy",
        ],
        _ => &["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"],
    };
    let Some((name, value)) = names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| (*name, value))
    }) else {
        return Ok(None);
    };
    ureq::Proxy::new(&value).map(Some).map_err(|_| HttpError {
        kind: crate::provider::ProviderErrorKind::Proxy,
        stage: FailureStage::ProxyConfiguration,
        message: format!("request blocked by malformed {name}"),
        request_started: false,
    })
}

fn no_proxy_matches(host: &str, port: u16) -> bool {
    let value = std::env::var("NO_PROXY")
        .ok()
        .or_else(|| std::env::var("no_proxy").ok());
    value.is_some_and(|value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .any(|entry| no_proxy_entry_matches(entry, host, port))
    })
}

fn no_proxy_entry_matches(entry: &str, host: &str, port: u16) -> bool {
    if entry == "*" {
        return true;
    }
    let (entry_host, entry_port) = split_no_proxy_entry(entry);
    if entry_port.is_some_and(|expected| expected != port) {
        return false;
    }
    let host = host.trim_matches(['[', ']']).trim_end_matches('.');
    let entry_host = entry_host
        .trim_matches(['[', ']'])
        .trim_start_matches('.')
        .trim_end_matches('.');
    match (host.parse::<IpAddr>(), entry_host.parse::<IpAddr>()) {
        (Ok(actual), Ok(expected)) => actual == expected,
        (Ok(_), Err(_)) | (Err(_), Ok(_)) => false,
        (Err(_), Err(_)) => {
            host.eq_ignore_ascii_case(entry_host)
                || host
                    .to_ascii_lowercase()
                    .ends_with(&format!(".{}", entry_host.to_ascii_lowercase()))
        }
    }
}

fn split_no_proxy_entry(entry: &str) -> (&str, Option<u16>) {
    if let Some(close) = entry.find(']').filter(|_| entry.starts_with('[')) {
        let host = &entry[..=close];
        let port = entry[close + 1..]
            .strip_prefix(':')
            .and_then(|value| value.parse().ok());
        return (host, port);
    }
    if entry.matches(':').count() == 1 {
        if let Some((host, port)) = entry.rsplit_once(':') {
            if let Ok(port) = port.parse() {
                return (host, Some(port));
            }
        }
    }
    (entry, None)
}

fn classify_transport(error: &ureq::Transport, via_proxy: bool) -> HttpError {
    let detail = error.to_string();
    let lowercase = detail.to_ascii_lowercase();
    let stage = match error.kind() {
        ureq::ErrorKind::InvalidProxyUrl => FailureStage::ProxyConfiguration,
        ureq::ErrorKind::ProxyConnect | ureq::ErrorKind::ProxyUnauthorized => {
            FailureStage::ProxyConnection
        }
        ureq::ErrorKind::Dns => FailureStage::Dns,
        ureq::ErrorKind::ConnectionFailed | ureq::ErrorKind::Io
            if contains_certificate_error(&lowercase) =>
        {
            FailureStage::TlsCertificate
        }
        ureq::ErrorKind::ConnectionFailed | ureq::ErrorKind::Io
            if lowercase.contains("tls") || lowercase.contains("handshake") =>
        {
            FailureStage::TlsHandshake
        }
        ureq::ErrorKind::ConnectionFailed if via_proxy => FailureStage::ProxyConnection,
        ureq::ErrorKind::ConnectionFailed => FailureStage::Tcp,
        _ => FailureStage::Http,
    };
    let timed_out = lowercase.contains("timed out") || lowercase.contains("timeout");
    let certificate_kind = certificate_error_name(&lowercase);
    let suffix = certificate_kind
        .map(|name| format!(" ({name})"))
        .unwrap_or_default();
    let kind = if timed_out {
        crate::provider::ProviderErrorKind::Timeout
    } else {
        match stage {
            FailureStage::Configuration | FailureStage::TlsCertificate => {
                crate::provider::ProviderErrorKind::Trust
            }
            FailureStage::ProxyConfiguration | FailureStage::ProxyConnection => {
                crate::provider::ProviderErrorKind::Proxy
            }
            FailureStage::Dns => crate::provider::ProviderErrorKind::Dns,
            FailureStage::TlsHandshake => crate::provider::ProviderErrorKind::Tls,
            FailureStage::Tcp | FailureStage::Http => crate::provider::ProviderErrorKind::Network,
        }
    };
    HttpError {
        kind,
        stage,
        message: format!("request failed during {}{suffix}", stage.label()),
        request_started: true,
    }
}

fn contains_certificate_error(detail: &str) -> bool {
    [
        "unknownissuer",
        "unknown issuer",
        "certificate",
        "certnotvalid",
        "invalidcertificate",
    ]
    .iter()
    .any(|needle| detail.contains(needle))
}

fn certificate_error_name(detail: &str) -> Option<&'static str> {
    if detail.contains("unknownissuer") || detail.contains("unknown issuer") {
        Some("UnknownIssuer")
    } else if detail.contains("notvalidforname") || detail.contains("not valid for name") {
        Some("NotValidForName")
    } else if detail.contains("expired") {
        Some("Expired")
    } else if detail.contains("notvalidyet") || detail.contains("not valid yet") {
        Some("NotValidYet")
    } else {
        None
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
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body.trim()) {
        if let Some(message) = json
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(|message| message.as_str())
        {
            return format!(
                "HTTP {}: {}",
                status,
                crate::render::ansi::sanitize_untrusted(message)
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
    }

    #[test]
    fn no_proxy_matches_domains_ips_ipv6_and_ports() {
        assert!(no_proxy_entry_matches(
            "example.com",
            "api.example.com",
            443
        ));
        assert!(no_proxy_entry_matches(".example.com", "example.com", 443));
        assert!(no_proxy_entry_matches("127.0.0.1:8443", "127.0.0.1", 8443));
        assert!(!no_proxy_entry_matches("127.0.0.1:8443", "127.0.0.1", 443));
        assert!(no_proxy_entry_matches("[::1]:443", "::1", 443));
        assert!(!no_proxy_entry_matches("10.0.0.1", "10.0.0.2", 443));
        assert!(no_proxy_entry_matches("*", "anything.invalid", 443));
    }

    #[test]
    fn certificate_names_are_safe_and_specific() {
        assert_eq!(
            certificate_error_name("invalidcertificate(unknownissuer)"),
            Some("UnknownIssuer")
        );
        assert_eq!(certificate_error_name("connection reset"), None);
    }
}
