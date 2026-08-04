//! Structured local diagnostics with an opt-in OpenAI reachability check.

use crate::{config::Config, render::ansi, runtime, secret, telemetry};
use serde::Serialize;
use std::io::IsTerminal;
use std::path::Path;
use std::time::Duration;

#[derive(Serialize)]
pub struct Check {
    pub name: &'static str,
    pub status: &'static str,
    pub detail: String,
    pub next: Option<String>,
}

#[derive(Serialize)]
pub struct Report {
    pub supported: bool,
    pub checks: Vec<Check>,
}

pub fn gather(
    config: &Config,
    network: bool,
    all_providers: bool,
    telemetry_policy: &telemetry::Policy,
) -> Report {
    let supported = matches!(std::env::consts::OS, "linux" | "macos")
        && matches!(std::env::consts::ARCH, "x86_64" | "aarch64");
    let host_hint = format!(
        "uhm v{}.{} supports Linux/macOS on x86_64/aarch64",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR")
    );
    let mut checks = vec![
        check(
            "host",
            supported,
            format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH),
            &host_hint,
        ),
        Check {
            name: "terminal",
            status: "ok",
            detail: format!(
                "tty={}, plain={}, color={}, motion={}",
                std::io::stderr().is_terminal(),
                ansi::plain_enabled(),
                ansi::color_enabled(),
                ansi::motion_enabled()
            ),
            next: None,
        },
        path_check("config", &config.paths.config_file, false),
        path_check("data", &config.paths.data_dir, true),
        path_check("cache", &config.paths.cache_dir, true),
        Check {
            name: "context",
            status: "ok",
            detail: format!("{} (inspect: uhm context show)", config.context_mode),
            next: None,
        },
        Check {
            name: "telemetry",
            status: if telemetry_policy.enabled {
                "ok"
            } else {
                "off"
            },
            detail: format!(
                "{} ({})",
                if telemetry_policy.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                telemetry_policy.reason
            ),
            next: Some("inspect: uhm telemetry status; preview: uhm telemetry preview".into()),
        },
        shell_check(),
        python_check(config.program.enabled),
        clipboard_check(),
        environment_check(config),
        containment_check(config),
        repair_recover_check(config),
        undo_restore_check(config),
    ];
    let providers = if all_providers {
        vec![
            crate::provider::ProviderId::Openai,
            crate::provider::ProviderId::Cerebras,
        ]
    } else {
        vec![config.provider]
    };
    for provider in providers {
        checks.push(key_check(provider));
        checks.push(provider_capabilities(provider));
        if network {
            checks.push(network_check(provider));
        } else {
            checks.push(Check {
                name: "provider network",
                status: "skipped",
                detail: format!("{} {}", provider, provider.adapter().endpoint()),
                next: Some(
                    "run `uhm doctor network` for an explicit reachability/authentication check"
                        .into(),
                ),
            });
        }
    }
    Report { supported, checks }
}

fn provider_capabilities(provider: crate::provider::ProviderId) -> Check {
    let adapter = provider.adapter();
    let capabilities = adapter.capabilities();
    Check {
        name: "provider adapter",
        status: "ok",
        detail: format!(
            "{} · {} · stream={} · reasoning={} · strict-bounds={}",
            provider,
            adapter.endpoint(),
            capabilities.streaming,
            capabilities.reasoning_effort,
            capabilities.strict_schema_bounds
        ),
        next: None,
    }
}

fn python_check(enabled: bool) -> Check {
    if !enabled {
        return Check {
            name: "Python runtime",
            status: "off",
            detail: "microprogram execution disabled by configuration".into(),
            next: None,
        };
    }
    let inventory = runtime::inventory();
    Check {
        name: "Python runtime",
        status: if inventory.available { "ok" } else { "missing" },
        detail: match (inventory.resolved_path, inventory.version) {
            (Some(path), Some(version)) => format!("{} ({}, isolated/no-site)", path, version),
            _ => "python3 -I -S unavailable".into(),
        },
        next: (!inventory.available).then(|| {
            "install Python 3 or disable program execution; shell actions remain available".into()
        }),
    }
}

/// Overall health: true only when no check reports a real failure. Benign
/// statuses (`ok`, `off`, `skipped`, `optional`) do not fail — `optional` covers
/// the clipboard helper, which is genuinely optional. Everything else (e.g.
/// `unsupported`, `missing`, `permissions`, `blocked`, `authentication`,
/// `rate_limit`, `api`, or a transport-stage status) is a failure. The host check already
/// emits `unsupported` on an unsupported platform, so this subsumes the old
/// `report.supported` test.
pub fn healthy(report: &Report) -> bool {
    report.checks.iter().all(|check| {
        matches!(
            check.status,
            "ok" | "off" | "skipped" | "optional" | "warning"
        )
    })
}

pub fn render(report: &Report) {
    for line in render_lines(report) {
        println!("{}", line);
    }
}

/// Column widths come from the widest name and status in this report, and
/// the status pad is computed from the unstyled status text, so multi-word
/// names, long statuses, and ANSI styling bytes never push the detail column
/// out of alignment.
fn render_lines(report: &Report) -> Vec<String> {
    let name_width = report
        .checks
        .iter()
        .map(|check| check.name.len())
        .max()
        .unwrap_or(0);
    let status_width = report
        .checks
        .iter()
        .map(|check| check.status.len())
        .max()
        .unwrap_or(0);
    let mut lines = Vec::new();
    for check in &report.checks {
        let state = match check.status {
            "ok" => ansi::success("OK"),
            "off" | "skipped" => ansi::info(check.status),
            "warning" => ansi::warning("warning"),
            _ => ansi::critical(check.status),
        };
        let pad = " ".repeat(status_width.saturating_sub(check.status.len()));
        lines.push(format!(
            "{:<name_width$} {}{} {}",
            check.name, state, pad, check.detail
        ));
        if let Some(next) = &check.next {
            lines.push(format!(
                "{:<indent$} {}",
                "",
                next,
                indent = name_width + status_width + 1
            ));
        }
    }
    lines
}

/// Whether `uhm repair` and `uhm recover` can complete on this install: both
/// seed from the retained intent, which only the full history detail keeps.
fn repair_recover_check(config: &Config) -> Check {
    let usable =
        config.history.enabled && config.history.detail == crate::config::HistoryDetail::Full;
    Check {
        name: "repair/recover",
        status: if usable { "ok" } else { "off" },
        detail: if usable {
            "usable; history retains intents (history.detail: full)".into()
        } else if !config.history.enabled {
            "not usable: history is disabled, so no run retains the intent these commands replay"
                .into()
        } else {
            format!(
                "not usable: history.detail is {} and repair/recover need the retained intent",
                config.history.detail.as_str()
            )
        },
        next: (!usable).then(|| {
            "set history.enabled: true and history.detail: full to make uhm repair/recover work"
                .into()
        }),
    }
}

/// Whether `uhm undo`/`uhm restore` can complete on this install: both need
/// a captured preimage, and capture is off until explicitly enabled.
fn undo_restore_check(config: &Config) -> Check {
    let enabled = crate::recovery::effective_enabled(&config.paths.data_dir, &config.recovery);
    Check {
        name: "undo/restore",
        status: if enabled { "ok" } else { "off" },
        detail: if enabled {
            "usable; eligible managed-file jobs capture preimages for uhm undo/restore".into()
        } else {
            "not usable: recovery snapshot capture is off, so runs retain no preimage".into()
        },
        next: (!enabled)
            .then(|| "run `uhm recovery on`, or pass --recoverable to capture one job".into()),
    }
}

pub fn environment(config: &Config) -> Report {
    Report {
        supported: true,
        checks: vec![environment_check(config), containment_check(config)],
    }
}

fn environment_check(config: &Config) -> Check {
    let exposed = crate::environment::exposed_common_names(
        config.execution.deny_common_env,
        &config.execution.deny_env,
    );
    Check {
        name: "child environment",
        status: if exposed.is_empty() { "ok" } else { "warning" },
        detail: if exposed.is_empty() {
            "no detected common credential names would reach shell children".into()
        } else {
            format!("inherited credential names: {} (values hidden)", exposed.join(", "))
        },
        next: (!exposed.is_empty()).then(|| {
            "set execution.deny_common_env: true, add exact names to execution.deny_env, or invoke uhm with a minimized environment".into()
        }),
    }
}

fn containment_check(config: &Config) -> Check {
    let mode = config.execution.containment;
    let available = crate::containment::executable().is_some();
    Check {
        name: "containment",
        status: match mode {
            crate::containment::Mode::Off => "off",
            crate::containment::Mode::Bubblewrap if cfg!(target_os = "linux") && available => "ok",
            crate::containment::Mode::Bubblewrap => "missing",
        },
        detail: match mode {
            crate::containment::Mode::Off => "disabled; child processes use the caller's OS permissions".into(),
            crate::containment::Mode::Bubblewrap if cfg!(target_os = "linux") && available => "Bubblewrap requested; network and writes outside the working directory are isolated".into(),
            crate::containment::Mode::Bubblewrap => "Bubblewrap requested but unavailable on this host".into(),
        },
        next: (mode == crate::containment::Mode::Bubblewrap && (!cfg!(target_os = "linux") || !available))
            .then(|| "install `bwrap` on Linux or set execution.containment: off".into()),
    }
}

fn check(name: &'static str, ok: bool, detail: String, next: &str) -> Check {
    Check {
        name,
        status: if ok { "ok" } else { "unsupported" },
        detail,
        next: (!ok).then(|| next.into()),
    }
}

fn key_check(provider: crate::provider::ProviderId) -> Check {
    let variable = provider.adapter().credential_env();
    match secret::resolve_key(provider) {
        Ok(_) => Check {
            name: "provider key",
            status: "ok",
            detail: "configured (value hidden)".into(),
            next: None,
        },
        Err(_) => {
            let path = secret::file_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "<data-dir>/uhm/secrets".into());
            Check {
                name: "provider key",
                status: "missing",
                detail: format!("{variable} is not configured"),
                next: Some(format!(
                    "set {variable}, or write {variable}=... to {} and chmod 600 it",
                    path
                )),
            }
        }
    }
}

fn path_check(name: &'static str, path: &Path, directory: bool) -> Check {
    let exists = path.exists();
    let private = if !exists {
        true
    } else {
        private_permissions(path)
    };
    Check {
        name,
        status: if private { "ok" } else { "permissions" },
        detail: format!(
            "{}{}",
            path.display(),
            if exists { "" } else { " (created on use)" }
        ),
        next: (!private).then(|| {
            format!(
                "restrict {} to owner-only permissions",
                if directory { "directory" } else { "file" }
            )
        }),
    }
}

#[cfg(unix)]
fn private_permissions(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o077 == 0)
        .unwrap_or(false)
}
#[cfg(not(unix))]
fn private_permissions(_: &Path) -> bool {
    true
}

fn shell_check() -> Check {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let name = Path::new(&shell)
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or(&shell);
    let supported = matches!(name, "sh" | "bash" | "zsh" | "fish" | "pwsh" | "powershell");
    Check {
        name: "shell",
        status: if supported { "ok" } else { "unsupported" },
        detail: shell,
        next: Some(
            "parent cwd/export/alias changes are generated but not applied by the base binary"
                .into(),
        ),
    }
}

fn clipboard_check() -> Check {
    let mechanisms = ["pbcopy", "wl-copy", "xclip"]
        .into_iter()
        .filter(|name| executable(name))
        .collect::<Vec<_>>();
    Check {
        name: "clipboard",
        status: if mechanisms.is_empty() {
            "optional"
        } else {
            "ok"
        },
        detail: if mechanisms.is_empty() {
            "no supported helper found".into()
        } else {
            mechanisms.join(", ")
        },
        next: mechanisms.is_empty().then(|| {
            "copy review control prints exact command bytes when no helper is integrated".into()
        }),
    }
}

fn executable(name: &str) -> bool {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
        .any(|dir| dir.join(name).is_file())
}

fn network_check(provider: crate::provider::ProviderId) -> Check {
    let Ok(key) = secret::resolve_key(provider) else {
        return Check {
            name: "provider network",
            status: "blocked",
            detail: "API key missing".into(),
            next: Some("configure the key, then rerun `uhm doctor network`".into()),
        };
    };
    let models_endpoint = match provider {
        crate::provider::ProviderId::Openai => "https://api.openai.com/v1/models",
        crate::provider::ProviderId::Cerebras => "https://api.cerebras.ai/v1/models",
    };
    let agent = match crate::http::agent_for(
        models_endpoint,
        crate::http::Timeouts::uniform(Duration::from_secs(3)),
    ) {
        Ok(agent) => agent,
        Err(error) => return transport_check(error),
    };
    match agent
        .get(models_endpoint)
        .set("Authorization", &format!("Bearer {}", key))
        .call()
    {
        Ok(_) => Check {
            name: "provider network",
            status: "ok",
            detail: "TLS, reachability, and authentication succeeded".into(),
            next: None,
        },
        Err(ureq::Error::Status(401, _)) => Check {
            name: "provider network",
            status: "authentication",
            detail: format!("{provider} rejected the API key"),
            next: Some("replace the key in the environment or private secrets file".into()),
        },
        Err(ureq::Error::Status(429, _)) => Check {
            name: "provider network",
            status: "rate_limit",
            detail: format!("{provider} returned HTTP 429"),
            next: Some("check project quota/billing and retry later".into()),
        },
        Err(ureq::Error::Status(code, _)) => Check {
            name: "provider network",
            status: "api",
            detail: format!("{provider} returned HTTP {code}"),
            next: Some("retry with --verbose or check the provider status page".into()),
        },
        Err(error) => transport_check(agent.classify_error(error)),
    }
}

fn transport_check(error: crate::http::HttpError) -> Check {
    Check {
        name: "provider network",
        status: match error.stage {
            crate::http::FailureStage::Configuration => "trust_config",
            crate::http::FailureStage::ProxyConfiguration => "proxy_config",
            crate::http::FailureStage::ProxyConnection => "proxy_connect",
            crate::http::FailureStage::Dns => "dns",
            crate::http::FailureStage::Tcp => "tcp",
            crate::http::FailureStage::TlsCertificate => "tls_certificate",
            crate::http::FailureStage::TlsHandshake => "tls_handshake",
            crate::http::FailureStage::Http => "network",
        },
        detail: error.message,
        next: Some(match error.stage {
            crate::http::FailureStage::Configuration => {
                "fix the named CA bundle or trust-store setting, then rerun `uhm doctor network`"
            }
            crate::http::FailureStage::ProxyConfiguration => {
                "fix the named proxy variable, then rerun `uhm doctor network`"
            }
            crate::http::FailureStage::ProxyConnection => {
                "check proxy reachability, credentials, and CONNECT policy"
            }
            crate::http::FailureStage::Dns => {
                "keep the managed proxy configured when direct DNS is unavailable"
            }
            crate::http::FailureStage::Tcp => "check destination reachability and firewall policy",
            crate::http::FailureStage::TlsCertificate => {
                "configure the private root with UHM_CA_BUNDLE or the standard SSL certificate variables"
            }
            crate::http::FailureStage::TlsHandshake => {
                "check proxy interception and TLS protocol compatibility"
            }
            crate::http::FailureStage::Http => "retry or inspect the provider status",
        }
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_never_contains_a_key_value() {
        std::env::set_var("OPENAI_API_KEY", "doctor-secret-sentinel");
        let root = tempfile::tempdir().unwrap();
        let config = Config::test(crate::dirs::Paths {
            config_file: root.path().join("c"),
            data_dir: root.path().join("d"),
            cache_dir: root.path().join("x"),
        });
        let report = gather(
            &config,
            false,
            false,
            &telemetry::Policy {
                enabled: false,
                reason: "test",
            },
        );
        assert!(!serde_json::to_string(&report)
            .unwrap()
            .contains("doctor-secret-sentinel"));
        std::env::remove_var("OPENAI_API_KEY");
    }

    #[test]
    fn all_provider_mode_reports_both_fixed_adapters() {
        let root = tempfile::tempdir().unwrap();
        let config = Config::test(crate::dirs::Paths {
            config_file: root.path().join("c"),
            data_dir: root.path().join("d"),
            cache_dir: root.path().join("x"),
        });
        let report = gather(
            &config,
            false,
            true,
            &telemetry::Policy {
                enabled: false,
                reason: "test",
            },
        );
        let rendered = serde_json::to_string(&report).unwrap();
        assert!(rendered.contains(crate::provider::openai::ENDPOINT));
        assert!(rendered.contains(crate::provider::cerebras::ENDPOINT));
        assert!(!rendered.contains("Bearer"));
    }

    fn check_status(name: &'static str, status: &'static str) -> Check {
        Check {
            name,
            status,
            detail: String::new(),
            next: None,
        }
    }

    /// Visible text of a rendered line, with ANSI escape sequences removed.
    fn visible(line: &str) -> String {
        let mut cleaned = String::new();
        let mut in_escape = false;
        for character in line.chars() {
            if in_escape {
                in_escape = character != 'm';
            } else if character == '\u{1b}' {
                in_escape = true;
            } else {
                cleaned.push(character);
            }
        }
        cleaned
    }

    #[test]
    fn detail_column_stays_aligned_across_multiword_names_and_statuses() {
        let report = Report {
            supported: true,
            checks: vec![
                Check {
                    name: "host",
                    status: "ok",
                    detail: "detail-anchor a".into(),
                    next: None,
                },
                Check {
                    name: "child environment",
                    status: "warning",
                    detail: "detail-anchor b".into(),
                    next: None,
                },
                Check {
                    name: "provider network",
                    status: "skipped",
                    detail: "detail-anchor c".into(),
                    next: None,
                },
                Check {
                    name: "provider network",
                    status: "authentication",
                    detail: "detail-anchor d".into(),
                    next: None,
                },
            ],
        };
        let lines = render_lines(&report);
        let columns: Vec<usize> = lines
            .iter()
            .map(|line| visible(line).find("detail-anchor").unwrap())
            .collect();
        assert!(
            columns.iter().all(|column| *column == columns[0]),
            "misaligned detail columns {columns:?} in {lines:#?}"
        );
    }

    #[test]
    fn default_install_reports_repair_recover_and_undo_as_not_usable() {
        let root = tempfile::tempdir().unwrap();
        let config = Config::test(crate::dirs::Paths {
            config_file: root.path().join("c"),
            data_dir: root.path().join("d"),
            cache_dir: root.path().join("x"),
        });
        let repair = repair_recover_check(&config);
        assert_eq!(repair.status, "off");
        assert!(repair.detail.contains("not usable"), "{}", repair.detail);
        assert!(repair
            .next
            .as_deref()
            .unwrap()
            .contains("history.detail: full"));
        let undo = undo_restore_check(&config);
        assert_eq!(undo.status, "off");
        assert!(undo.detail.contains("not usable"), "{}", undo.detail);
        assert!(undo.next.as_deref().unwrap().contains("uhm recovery on"));
    }

    #[test]
    fn enabled_retention_reports_repair_recover_and_undo_as_usable() {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::test(crate::dirs::Paths {
            config_file: root.path().join("c"),
            data_dir: root.path().join("d"),
            cache_dir: root.path().join("x"),
        });
        config.history.detail = crate::config::HistoryDetail::Full;
        std::fs::create_dir_all(&config.paths.data_dir).unwrap();
        crate::recovery::enable(&config.paths.data_dir).unwrap();
        assert_eq!(repair_recover_check(&config).status, "ok");
        assert_eq!(undo_restore_check(&config).status, "ok");
    }

    #[test]
    fn healthy_report_has_no_failures() {
        let report = Report {
            supported: true,
            checks: vec![
                check_status("host", "ok"),
                check_status("OpenAI network", "ok"),
            ],
        };
        assert!(healthy(&report));
    }

    #[test]
    fn benign_optional_and_skipped_statuses_are_healthy() {
        let report = Report {
            supported: true,
            checks: vec![
                check_status("host", "ok"),
                check_status("clipboard", "optional"),
                check_status("OpenAI network", "skipped"),
                check_status("Python runtime", "off"),
            ],
        };
        assert!(healthy(&report));
    }

    #[test]
    fn failing_network_check_is_unhealthy() {
        let report = Report {
            supported: true,
            checks: vec![
                check_status("host", "ok"),
                check_status("OpenAI network", "network_tls"),
            ],
        };
        assert!(!healthy(&report));
    }

    #[test]
    fn unsupported_host_is_unhealthy() {
        let report = Report {
            supported: false,
            checks: vec![check_status("host", "unsupported")],
        };
        assert!(!healthy(&report));
    }
}
