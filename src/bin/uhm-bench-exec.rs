//! Private, keyless adapter around production execution primitives.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use uhm_cli::action::{ProposedAction, StdinMode};

const RESULT_PATH: &str = "/tmp/uhm-bench-execution-result.json";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Limits {
    wall_ms: u64,
    stdout_bytes: usize,
    stderr_bytes: usize,
    workspace_bytes: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    action: WireAction,
    stdin: Option<String>,
    limits: Limits,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireAction {
    tool: String,
    arguments: Value,
}

#[derive(Serialize)]
struct ResultEnvelope {
    started: bool,
    exit_code: i32,
    signal: Option<i32>,
    timed_out: bool,
    duration_ms: u128,
    helper_setup_ms: Option<u128>,
    output_overflow: bool,
    artifact_commit_success: Option<bool>,
    parent_state: Option<Value>,
}

fn write_result(result: &ResultEnvelope) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(RESULT_PATH)
        .map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut file, result).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())
}

#[cfg(unix)]
trait PrivateMode {
    fn mode(&mut self, mode: u32) -> &mut Self;
}
#[cfg(unix)]
impl PrivateMode for std::fs::OpenOptions {
    fn mode(&mut self, mode: u32) -> &mut Self {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptionsExt::mode(self, mode)
    }
}
#[cfg(not(unix))]
trait PrivateMode {
    fn mode(&mut self, _mode: u32) -> &mut Self {
        self
    }
}
#[cfg(not(unix))]
impl PrivateMode for std::fs::OpenOptions {}

fn shell_result(result: uhm_cli::shell::Result) -> ResultEnvelope {
    ResultEnvelope {
        started: true,
        exit_code: result.code,
        signal: result.signal,
        timed_out: result.timed_out,
        duration_ms: result.duration.as_millis(),
        helper_setup_ms: None,
        output_overflow: false,
        artifact_commit_success: None,
        parent_state: None,
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("benchmark execution adapter: {error}");
        std::process::exit(125);
    }
}

fn run() -> Result<(), String> {
    let _ = std::fs::remove_file(RESULT_PATH);
    let mut raw = Vec::new();
    std::io::stdin()
        .take(256 * 1024 + 1)
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
    if raw.len() > 256 * 1024 {
        return Err("execution envelope is oversized".into());
    }
    let envelope: Envelope = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
    let action =
        uhm_cli::contract::decode_and_validate(&envelope.action.tool, envelope.action.arguments)?;
    let cwd = Path::new("/work");
    let diagnostic = envelope
        .limits
        .stdout_bytes
        .max(envelope.limits.stderr_bytes);
    let result = match action {
        ProposedAction::Shell {
            command,
            stdin_mode,
            ..
        } => {
            let stdin = (stdin_mode == StdinMode::Original)
                .then(|| envelope.stdin.as_deref().unwrap_or("").as_bytes());
            shell_result(uhm_cli::shell::execute(uhm_cli::shell::Request {
                shell: "/bin/bash",
                command: &command,
                stdin,
                timeout: Duration::from_millis(envelope.limits.wall_ms),
                diagnostic_bytes: diagnostic,
                deny_common_env: false,
                deny_env: &[],
                containment: uhm_cli::containment::Mode::Off,
            })?)
        }
        ProposedAction::Program { program } => {
            let config = uhm_cli::config::ProgramConfig {
                timeout_secs: envelope.limits.wall_ms.div_ceil(1000).max(1),
                output_max_bytes: envelope
                    .limits
                    .stdout_bytes
                    .saturating_add(envelope.limits.stderr_bytes),
                diagnostic_bytes: diagnostic,
                workspace_max_bytes: envelope.limits.workspace_bytes,
                ..Default::default()
            };
            let runtime = uhm_cli::runtime::inventory();
            let diagnostics =
                uhm_cli::program::preflight(&program, &runtime, envelope.stdin.is_some());
            if let Some(diagnostic) = diagnostics.iter().find(|diagnostic| {
                diagnostic.severity != uhm_cli::program::DiagnosticSeverity::Warning
            }) {
                return Err(format!(
                    "program preflight {}: {}",
                    diagnostic.code, diagnostic.message
                ));
            }
            let executed = uhm_cli::program::execute(uhm_cli::program::Request {
                proposal: &program,
                python: &runtime,
                stdin: envelope.stdin.as_deref().map(str::as_bytes),
                cwd,
                config: &config,
                containment: uhm_cli::containment::Mode::Off,
                retain_workspace: false,
                recovery: None,
            })?;
            std::io::stdout()
                .write_all(&executed.stdout)
                .map_err(|e| e.to_string())?;
            std::io::stderr()
                .write_all(&executed.stderr_tail)
                .map_err(|e| e.to_string())?;
            ResultEnvelope {
                started: true,
                exit_code: executed.code,
                signal: executed.signal,
                timed_out: executed.timed_out,
                duration_ms: executed.duration.as_millis(),
                helper_setup_ms: Some(executed.helper_setup_duration.as_millis()),
                output_overflow: executed.output_overflow,
                artifact_commit_success: Some(executed.artifact_commit_success),
                parent_state: None,
            }
        }
        ProposedAction::ParentShell { action, .. } => {
            let rendered = uhm_cli::shell_integration::render(
                &action,
                uhm_cli::shell_integration::ShellFamily::Bash,
            )?;
            let state_path = PathBuf::from("/tmp/uhm-bench-parent-state");
            let _ = std::fs::remove_file(&state_path);
            let script = format!("{{ {rendered}; }}\nstatus=$?\nif [ \"$status\" -eq 0 ]; then {{ printf 'CWD=%s\\0' \"$PWD\"; env -0; }} > '{}'; fi\nexit \"$status\"", state_path.display());
            let mut result = shell_result(uhm_cli::shell::execute(uhm_cli::shell::Request {
                shell: "/bin/bash",
                command: &script,
                stdin: None,
                timeout: Duration::from_millis(envelope.limits.wall_ms),
                diagnostic_bytes: diagnostic,
                deny_common_env: false,
                deny_env: &[],
                containment: uhm_cli::containment::Mode::Off,
            })?);
            if result.exit_code == 0 {
                let bytes = std::fs::read(&state_path).map_err(|e| e.to_string())?;
                let mut cwd_value = None;
                let mut environment = serde_json::Map::new();
                for field in bytes
                    .split(|byte| *byte == 0)
                    .filter(|value| !value.is_empty())
                {
                    let field = String::from_utf8_lossy(field);
                    if let Some(value) = field.strip_prefix("CWD=") {
                        cwd_value = Some(value.to_string());
                    } else if let Some((name, value)) = field.split_once('=') {
                        environment.insert(name.into(), json!(value));
                    }
                }
                result.parent_state = Some(json!({"cwd":cwd_value,"environment":environment}));
            }
            result
        }
        _ => return Err("execution adapter received a non-executable action".into()),
    };
    write_result(&result)
}
