//! Structured local diagnostics with an opt-in OpenAI reachability check.

use crate::{api, config::Config, render::ansi, runtime, secret, telemetry};
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

pub fn gather(config: &Config, network: bool, telemetry_policy: &telemetry::Policy) -> Report {
    let supported = matches!(std::env::consts::OS, "linux" | "macos")
        && matches!(std::env::consts::ARCH, "x86_64" | "aarch64");
    let mut checks = vec![
        check(
            "host",
            supported,
            format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH),
            "uhm v0.1 supports Linux/macOS on x86_64/aarch64",
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
        key_check(),
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
    ];
    if network {
        checks.push(network_check());
    } else {
        checks.push(Check {
            name: "OpenAI network",
            status: "skipped",
            detail: api::ENDPOINT.into(),
            next: Some(
                "run `uhm doctor network` for an explicit reachability/authentication check".into(),
            ),
        });
    }
    Report { supported, checks }
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

pub fn render(report: &Report) {
    for check in &report.checks {
        let state = match check.status {
            "ok" => ansi::success("OK"),
            "off" | "skipped" => ansi::info(check.status),
            _ => ansi::critical(check.status),
        };
        println!("{:<16} {:<9} {}", check.name, state, check.detail);
        if let Some(next) = &check.next {
            println!("{:<27} {}", "", next);
        }
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

fn key_check() -> Check {
    match secret::resolve_key() {
        Ok(_) => Check {
            name: "API key",
            status: "ok",
            detail: "configured (value hidden)".into(),
            next: None,
        },
        Err(_) => {
            let path = secret::file_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "<data-dir>/uhm/secrets".into());
            Check {
                name: "API key",
                status: "missing",
                detail: "OPENAI_API_KEY is not configured".into(),
                next: Some(format!(
                    "set OPENAI_API_KEY, or write OPENAI_API_KEY=... to {} and chmod 600 it",
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

fn network_check() -> Check {
    let Ok(key) = secret::resolve_key() else {
        return Check {
            name: "OpenAI network",
            status: "blocked",
            detail: "API key missing".into(),
            next: Some("configure the key, then rerun `uhm doctor network`".into()),
        };
    };
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(3))
        .build();
    match agent
        .get("https://api.openai.com/v1/models")
        .set("Authorization", &format!("Bearer {}", key))
        .call()
    {
        Ok(_) => Check {
            name: "OpenAI network",
            status: "ok",
            detail: "TLS, reachability, and authentication succeeded".into(),
            next: None,
        },
        Err(ureq::Error::Status(401, _)) => Check {
            name: "OpenAI network",
            status: "authentication",
            detail: "OpenAI rejected the API key".into(),
            next: Some("replace the key in the environment or private secrets file".into()),
        },
        Err(ureq::Error::Status(429, _)) => Check {
            name: "OpenAI network",
            status: "rate_limit",
            detail: "OpenAI returned HTTP 429".into(),
            next: Some("check project quota/billing and retry later".into()),
        },
        Err(ureq::Error::Status(code, _)) => Check {
            name: "OpenAI network",
            status: "api",
            detail: format!("OpenAI returned HTTP {}", code),
            next: Some("retry with --verbose or check https://status.openai.com".into()),
        },
        Err(ureq::Error::Transport(error)) => Check {
            name: "OpenAI network",
            status: "network_tls",
            detail: format!("connection failed ({:?})", error.kind()),
            next: Some("check DNS, proxy, firewall, and TLS certificate settings".into()),
        },
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
}
