//! Versioned, bounded context sent to the model. Environment values never leave this module.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const POLICY_VERSION: u32 = 5;
pub const DISCLOSURE_VERSION: u32 = 4;
pub const TOOL_CATALOG: &[&str] = &[
    "sh", "bash", "zsh", "fish", "git", "rg", "fd", "jq", "yq", "fzf", "gh", "python3", "node",
    "ruby", "go", "cargo", "make", "curl", "wget", "tar", "zip", "docker", "podman", "kubectl",
    "aws", "gcloud", "brew", "apt", "dnf",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Minimal,
    Standard,
    Full,
}
impl Mode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "minimal" => Ok(Self::Minimal),
            "standard" => Ok(Self::Standard),
            "full" => Ok(Self::Full),
            _ => Err("context mode must be minimal, standard, or full".into()),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Standard => "standard",
            Self::Full => "full",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub policy_version: u32,
    pub mode: String,
    pub program_runtime: crate::runtime::PythonInventory,
    pub machine: Value,
}

pub fn disclosure_payload() -> Value {
    json!({
        "version": DISCLOSURE_VERSION,
        "default_mode":"standard",
        "leaves_device":true,
        "groups":["Python 3 runtime path/version/isolated-mode support","OS and architecture","target shell","common tool presence","normalized working directory","bounded Git state","bounded directory entry names","invocation-only parent cwd and previous exit status when shell integration is used","bounded help output of an installed tool your request names, after you allow that tool once"],
        "tool_help":"when a request names an installed executable, uhm asks once per tool before running its help flag and may then include that bounded output; the answer is remembered until the tool changes",
        "shell_history":"off by default; when enabled, exactly one entry is previewed and requires confirmation",
        "local_input":"--local-input keeps piped content out of the model request",
        "inspect":"uhm context show", "minimize":"--context minimal", "minimize_config":"context_mode: minimal"
    })
}

pub fn gather(mode: Mode, shell: &str, timeout_ms: u64) -> Snapshot {
    let program_runtime = crate::runtime::inventory();
    if mode == Mode::Minimal {
        return Snapshot {
            policy_version: POLICY_VERSION,
            mode: mode.as_str().into(),
            program_runtime,
            machine: json!({}),
        };
    }
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let normalized = normalize_cwd(&cwd);
    let tools = tool_presence();
    let entries = entry_names(&cwd, 40, 4096);
    let git = git_summary(deadline);
    let mut machine = json!({
        "os":{"family":std::env::consts::OS,"version":os_version(deadline)},
        "architecture":std::env::consts::ARCH,
        "target_shell":shell,
        "working_directory":normalized,
        "git":git,
        "entries":entries,
        "tools":tools,
        "session":{"ssh":std::env::var_os("SSH_CONNECTION").is_some()||std::env::var_os("SSH_TTY").is_some(),"tmux":std::env::var_os("TMUX").is_some(),"tty":std::io::stderr().is_terminal()},
    });
    if mode == Mode::Full {
        machine["full"] = json!({
            "raw_working_directory":cwd.to_string_lossy(),
            "username":run(&["id","-un"],deadline),
            "hostname":run(&["hostname"],deadline),
            "kernel":run(&["uname","-sr"],deadline),
            "tool_versions":tool_versions(deadline, &tools),
        });
    }
    Snapshot {
        policy_version: POLICY_VERSION,
        mode: mode.as_str().into(),
        program_runtime,
        machine,
    }
}

pub fn add_shell_invocation(
    snapshot: &mut Snapshot,
    session: &crate::shell_integration::Session,
    last_history: Option<&str>,
) {
    if snapshot.mode == "minimal" {
        if let Some(entry) = last_history {
            snapshot.machine["shell_invocation"] = json!({"last_history_entry":entry});
        }
        return;
    }
    snapshot.machine["shell_invocation"] = json!({
        "protocol_version":crate::shell_integration::PROTOCOL_VERSION,
        "shell":session.shell().as_str(),
        "parent_working_directory":normalize_cwd(Path::new(session.parent_cwd())),
        "previous_status":session.previous_status(),
        "last_history_entry":last_history,
    });
    if snapshot.mode == "full" {
        snapshot.machine["shell_invocation"]["raw_parent_working_directory"] =
            json!(session.parent_cwd());
    }
}

/// Attach the observed self-description of tools the request names. Absent under
/// `minimal`, which carries no general machine fields. The help text is a tool's
/// own untrusted output, so it is sanitized here before it can reach a request or
/// a terminal.
pub fn add_tool_surface(snapshot: &mut Snapshot, observed: &[crate::tool_surface::Observed]) {
    if snapshot.mode == "minimal" || observed.is_empty() {
        return;
    }
    snapshot.machine["named_tools"] = observed
        .iter()
        .map(|tool| {
            json!({
                "name": crate::render::ansi::sanitize_untrusted_inline(&tool.name),
                "help": crate::render::ansi::sanitize_untrusted(&tool.help),
            })
        })
        .collect();
}

fn normalize_cwd(cwd: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        if let Ok(relative) = cwd.strip_prefix(&home) {
            return if relative.as_os_str().is_empty() {
                "$HOME".into()
            } else {
                format!("$HOME/{}", relative.display())
            };
        }
    }
    cwd.file_name()
        .map(|v| format!("…/{}", v.to_string_lossy()))
        .unwrap_or_else(|| "…".into())
}
fn entry_names(cwd: &Path, max: usize, max_bytes: usize) -> Vec<String> {
    let mut names = std::fs::read_dir(cwd)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    let mut used = 0;
    names
        .into_iter()
        .take(max)
        .take_while(|n| {
            used += n.len();
            used <= max_bytes
        })
        .collect()
}
pub(crate) fn path_entries() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|v| std::env::split_paths(&v).collect())
        .unwrap_or_default()
}
fn executable(name: &str) -> bool {
    resolve_in(&path_entries(), name).is_some()
}
/// First supplied directory holding an executable regular file with this exact
/// name. Callers pass a bare name; a name containing a separator resolves to
/// nothing. The search list is a parameter so callers are not coupled to the
/// process environment.
pub(crate) fn resolve_in(search: &[PathBuf], name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains(std::path::MAIN_SEPARATOR) {
        return None;
    }
    search.iter().find_map(|directory| {
        let candidate = directory.join(name);
        (candidate.is_file() && is_executable(&candidate)).then_some(candidate)
    })
}
#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}
#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}
fn tool_presence() -> serde_json::Map<String, Value> {
    TOOL_CATALOG
        .iter()
        .map(|n| ((*n).into(), Value::Bool(executable(n))))
        .collect()
}
pub fn missing_requirements(requirements: &[String]) -> Vec<String> {
    requirements
        .iter()
        .filter(|n| !executable(n))
        .cloned()
        .collect()
}
fn tool_versions(
    deadline: Instant,
    tools: &serde_json::Map<String, Value>,
) -> serde_json::Map<String, Value> {
    tools
        .iter()
        .filter(|(_, v)| v == &&Value::Bool(true))
        .take(12)
        .filter_map(|(n, _)| {
            run(&[n, "--version"], deadline).map(|v| {
                (
                    n.clone(),
                    Value::String(v.lines().next().unwrap_or("").chars().take(160).collect()),
                )
            })
        })
        .collect()
}
fn git_summary(deadline: Instant) -> Value {
    let branch = run(&["git", "rev-parse", "--abbrev-ref", "HEAD"], deadline);
    if branch.is_none() {
        return Value::Null;
    }
    let dirty = run(&["git", "status", "--porcelain"], deadline)
        .map(|s| s.lines().take(101).count())
        .unwrap_or(0);
    json!({"branch":branch,"dirty":dirty>0,"changed_count":dirty.min(100)})
}
fn os_version(deadline: Instant) -> Option<String> {
    if cfg!(target_os = "macos") {
        run(&["sw_vers", "-productVersion"], deadline)
    } else {
        run(&["uname", "-r"], deadline)
    }
}
/// Run a bounded, inert probe: direct argv with no shell, stdin closed, stderr
/// discarded, stdout capped, killed at the deadline. Output is returned only for
/// a successful exit.
pub(crate) fn run(argv: &[&str], deadline: Instant) -> Option<String> {
    if argv.is_empty() || Instant::now() >= deadline {
        return None;
    }
    let mut child = Command::new(argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    loop {
        if let Some(status) = child.try_wait().ok()? {
            let mut out = String::new();
            use std::io::Read;
            child
                .stdout
                .take()?
                .take(4096)
                .read_to_string(&mut out)
                .ok()?;
            return status.success().then(|| out.trim().into());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn minimal_has_no_machine_fields() {
        let s = gather(Mode::Minimal, "/bin/sh", 50);
        assert_eq!(s.machine, json!({}));
    }
    #[test]
    fn minimal_carries_no_observed_tool_help() {
        let mut snapshot = gather(Mode::Minimal, "/bin/sh", 50);
        add_tool_surface(
            &mut snapshot,
            &[crate::tool_surface::Observed {
                name: "steel".into(),
                help: "usage: steel".into(),
            }],
        );
        assert_eq!(snapshot.machine, json!({}));
    }

    #[test]
    fn observed_tool_help_is_sanitized_before_it_can_leave() {
        let mut snapshot = gather(Mode::Standard, "/bin/sh", 150);
        add_tool_surface(
            &mut snapshot,
            &[crate::tool_surface::Observed {
                name: "ste\u{1b}[31mel".into(),
                help: "usage: \u{1b}[2Jsteel\nbrowser  start a session".into(),
            }],
        );
        let rendered = snapshot.machine["named_tools"].to_string();
        assert!(!rendered.contains('\u{1b}'), "{rendered}");
        assert!(rendered.contains("browser"), "{rendered}");
    }

    #[test]
    fn an_empty_observation_adds_no_field() {
        let mut snapshot = gather(Mode::Standard, "/bin/sh", 150);
        add_tool_surface(&mut snapshot, &[]);
        assert!(snapshot.machine.get("named_tools").is_none());
    }

    #[test]
    fn standard_tools_are_only_booleans() {
        let s = gather(Mode::Standard, "/bin/sh", 150);
        assert!(s.machine["tools"]
            .as_object()
            .unwrap()
            .values()
            .all(Value::is_boolean));
        assert!(s.machine.get("full").is_none());
    }
    #[test]
    fn disclosure_is_versioned() {
        assert_eq!(disclosure_payload()["version"], DISCLOSURE_VERSION);
        assert_eq!(
            disclosure_payload()["minimize_config"],
            "context_mode: minimal"
        );
        assert!(disclosure_payload().get("config").is_none());
    }
    #[test]
    fn integration_adds_only_the_bounded_invocation_fields() {
        let root = tempfile::tempdir().unwrap();
        let config = crate::config::Config::test(crate::dirs::Paths {
            config_file: root.path().join("config"),
            data_dir: root.path().join("data"),
            cache_dir: root.path().join("cache"),
        });
        let (dir, nonce) = crate::shell_integration::open(
            &config,
            crate::shell_integration::ShellFamily::Bash,
            "/tmp",
            23,
        )
        .unwrap();
        let session = crate::shell_integration::load(&config, &dir, &nonce).unwrap();
        let mut snapshot = gather(Mode::Standard, "bash", 50);
        add_shell_invocation(&mut snapshot, &session, Some("one exact entry"));
        let value = &snapshot.machine["shell_invocation"];
        assert_eq!(value["protocol_version"], 1);
        assert_eq!(value["shell"], "bash");
        assert_eq!(value["previous_status"], 23);
        assert_eq!(value["last_history_entry"], "one exact entry");
        assert_eq!(value.as_object().unwrap().len(), 5);
        let mut minimal = gather(Mode::Minimal, "bash", 50);
        add_shell_invocation(&mut minimal, &session, None);
        assert_eq!(minimal.machine, json!({}));
    }
}
