//! Content-free, identity-free, best-effort aggregate telemetry.

use crate::action::Effect;
use crate::config::Config;
use crate::{dirs, history};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

pub const ENDPOINT: &str = "https://uhm-telemetry.nikola-balic.workers.dev/v1/events";
pub const SCHEMA_VERSION: u8 = 2;
const MAX_EVENT_BYTES: usize = 2048;
const MAX_QUEUE: usize = 20;
const MAX_AGE: Duration = Duration::from_secs(7 * 86_400);

const EVENTS: &[&str] = &["interaction_summary", "feedback_summary"];
const OS: &[&str] = &["linux", "macos", "other"];
const ARCH: &[&str] = &["x86_64", "aarch64", "other"];
const SHELL: &[&str] = &["sh", "bash", "zsh", "fish", "pwsh", "powershell", "other"];
const MODES: &[&str] = &["auto", "run", "ask", "explain"];
const ROUTES: &[&str] = &[
    "unknown",
    "answer",
    "shell",
    "program",
    "parent_shell",
    "clarification",
];
const DECISIONS: &[&str] = &[
    "not_run",
    "ran",
    "returned",
    "dry_run",
    "cancelled",
    "needs_parent",
    "unavailable",
];
const EFFECTS: &[&str] = &[
    "none",
    "read_local",
    "write_local",
    "delete_local",
    "network_read",
    "remote_mutation",
    "privilege_elevation",
    "process_control",
    "shell_state",
    "unknown",
];
const PROPOSALS: &[&str] = &["not_requested", "valid", "invalid", "refused", "incomplete"];
const EXECUTIONS: &[&str] = &[
    "not_run",
    "exit_zero",
    "exit_nonzero",
    "signal",
    "timeout",
    "spawn_error",
    "output_overflow",
];
const FEEDBACK: &[&str] = &["unknown", "good", "bad"];
const LATENCIES: &[&str] = &["lt_1s", "1s_2s", "2s_5s", "gte_5s"];
const CACHES: &[&str] = &["unknown", "miss", "hit", "disabled"];
const PARENT_ACTIONS: &[&str] = &["not_applicable", "unknown", "applied", "failed"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub v: u8,
    pub event: String,
    pub release: String,
    pub os: String,
    pub arch: String,
    pub shell: String,
    pub mode: String,
    pub route: String,
    pub decision: String,
    pub effects: String,
    pub proposal_outcome: String,
    pub execution_outcome: String,
    pub user_feedback: String,
    pub latency: String,
    pub cache: String,
    pub parent_action: String,
    pub interactive: bool,
    pub notice_revision: u8,
}

impl Event {
    pub fn validate(&self) -> Result<(), String> {
        if self.v != SCHEMA_VERSION || self.notice_revision != crate::first_run::NOTICE_REVISION {
            return Err("unsupported telemetry schema or notice revision".into());
        }
        if self.release != release() {
            return Err("telemetry release must be major.minor".into());
        }
        for (label, value, allowed) in [
            ("event", self.event.as_str(), EVENTS),
            ("os", self.os.as_str(), OS),
            ("arch", self.arch.as_str(), ARCH),
            ("shell", self.shell.as_str(), SHELL),
            ("mode", self.mode.as_str(), MODES),
            ("route", self.route.as_str(), ROUTES),
            ("decision", self.decision.as_str(), DECISIONS),
            ("effects", self.effects.as_str(), EFFECTS),
            (
                "proposal_outcome",
                self.proposal_outcome.as_str(),
                PROPOSALS,
            ),
            (
                "execution_outcome",
                self.execution_outcome.as_str(),
                EXECUTIONS,
            ),
            ("user_feedback", self.user_feedback.as_str(), FEEDBACK),
            ("latency", self.latency.as_str(), LATENCIES),
            ("cache", self.cache.as_str(), CACHES),
            ("parent_action", self.parent_action.as_str(), PARENT_ACTIONS),
        ] {
            if !allowed.contains(&value) {
                return Err(format!("unknown telemetry {} enum", label));
            }
        }
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() >= MAX_EVENT_BYTES {
            return Err("telemetry event exceeds 2 KiB".into());
        }
        Ok(())
    }
}

pub struct Interaction {
    pub run_id: String,
    started: Instant,
    event: Option<Event>,
    pub network_bound: bool,
    pub suppress: bool,
}

impl Interaction {
    pub fn new(mode: &str, interactive: bool, enabled: bool) -> Self {
        Self {
            run_id: history::run_id(),
            started: Instant::now(),
            event: enabled.then(|| Event {
                v: SCHEMA_VERSION,
                event: "interaction_summary".into(),
                release: release(),
                os: enum_or(std::env::consts::OS, OS),
                arch: enum_or(std::env::consts::ARCH, ARCH),
                shell: "other".into(),
                mode: enum_or(mode, MODES),
                route: "unknown".into(),
                decision: "not_run".into(),
                effects: "none".into(),
                proposal_outcome: "not_requested".into(),
                execution_outcome: "not_run".into(),
                user_feedback: "unknown".into(),
                latency: "lt_1s".into(),
                cache: "unknown".into(),
                parent_action: "not_applicable".into(),
                interactive,
                notice_revision: crate::first_run::NOTICE_REVISION,
            }),
            network_bound: false,
            suppress: false,
        }
    }

    pub fn shell(&mut self, shell: &str) {
        let name = Path::new(shell)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(shell);
        if let Some(event) = &mut self.event {
            event.shell = enum_or(name, SHELL);
        }
    }
    pub fn proposal(&mut self, valid: bool, cache_hit: bool) {
        if let Some(event) = &mut self.event {
            event.proposal_outcome = if valid { "valid" } else { "invalid" }.into();
            event.cache = if cache_hit { "hit" } else { "miss" }.into();
        }
        self.network_bound |= !cache_hit;
        self.suppress |= cache_hit;
    }
    pub fn route(&mut self, route: &str) {
        if let Some(event) = &mut self.event {
            event.route = enum_or(route, ROUTES);
        }
    }
    pub fn decision(&mut self, decision: &str) {
        if let Some(event) = &mut self.event {
            event.decision = enum_or(decision, DECISIONS);
        }
    }
    pub fn effects(&mut self, effects: &[Effect]) {
        if let Some(event) = &mut self.event {
            event.effects = dominant_effect(effects).into();
        }
    }
    pub fn execution(&mut self, value: &str) {
        if let Some(event) = &mut self.event {
            event.execution_outcome = enum_or(value, EXECUTIONS);
        }
    }
    pub fn suppress(&mut self) {
        self.suppress = true;
    }
    pub fn parent_pending(&mut self) {
        if let Some(event) = &mut self.event {
            event.parent_action = "unknown".into();
        }
    }
    pub fn event(mut self) -> Option<Event> {
        if let Some(event) = &mut self.event {
            event.latency = latency(self.started.elapsed()).into();
        }
        self.event
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub enabled: bool,
    pub reason: &'static str,
}

pub fn policy(config: &Config, flag_off: bool) -> Policy {
    if flag_off {
        return Policy {
            enabled: false,
            reason: "--no-telemetry",
        };
    }
    if std::env::var("DO_NOT_TRACK").is_ok_and(|value| value == "1") {
        return Policy {
            enabled: false,
            reason: "DO_NOT_TRACK=1",
        };
    }
    if std::env::var("UHM_TELEMETRY")
        .is_ok_and(|value| matches!(value.to_ascii_lowercase().as_str(), "off" | "0" | "false"))
    {
        return Policy {
            enabled: false,
            reason: "UHM_TELEMETRY",
        };
    }
    if !config.telemetry.enabled {
        return Policy {
            enabled: false,
            reason: "config",
        };
    }
    if disabled_marker(config).exists() {
        return Policy {
            enabled: false,
            reason: "uhm telemetry off",
        };
    }
    Policy {
        enabled: true,
        reason: "default",
    }
}

pub fn preview(mode: &str, interactive: bool) -> Event {
    Interaction::new(mode, interactive, true)
        .event()
        .expect("preview is enabled")
}

pub fn complete(config: &Config, resolved_policy: &Policy, interaction: Interaction) {
    if !resolved_policy.enabled || interaction.suppress {
        return;
    }
    let network_bound = interaction.network_bound;
    let run_id = interaction.run_id.clone();
    let Some(event) = interaction.event() else {
        return;
    };
    if event.validate().is_err() {
        return;
    }
    if event.parent_action == "unknown" {
        if !policy(config, false).enabled {
            return;
        }
        let _ = enqueue(config, &run_id, &event);
        return;
    }
    let Ok(send_lock) = open_lock(&telemetry_root(config), "send.lock") else {
        return;
    };
    if send_lock.try_lock_exclusive().is_err() {
        let _ = enqueue(config, &run_id, &event);
        return;
    }
    if !policy(config, false).enabled {
        let _ = fs2::FileExt::unlock(&send_lock);
        return;
    }
    if network_bound {
        flush_older(config, Duration::from_millis(200));
    }
    match send(&event, Duration::from_millis(100)) {
        SendResult::Accepted | SendResult::Ambiguous | SendResult::Rejected => {}
        SendResult::PreSend => {
            let _ = enqueue(config, &run_id, &event);
        }
    }
    let _ = fs2::FileExt::unlock(&send_lock);
}

pub fn disable(config: &Config) -> Result<(), String> {
    dirs::ensure_private_dir(&config.paths.data_dir)?;
    write_private_atomic(&disabled_marker(config), b"off")?;
    let root = telemetry_root(config);
    let send_lock = open_lock(&root, "send.lock")?;
    send_lock
        .lock_exclusive()
        .map_err(|e| format!("lock telemetry sender: {}", e))?;
    let queue_lock = open_lock(&root, "queue.lock")?;
    queue_lock
        .lock_exclusive()
        .map_err(|e| format!("lock telemetry queue: {}", e))?;
    if let Ok(entries) = std::fs::read_dir(root.join("queue")) {
        for entry in entries.flatten() {
            if entry.path().is_file() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    fs2::FileExt::unlock(&queue_lock).map_err(|e| e.to_string())?;
    fs2::FileExt::unlock(&send_lock).map_err(|e| e.to_string())
}

pub fn enable(config: &Config) -> Result<(), String> {
    match std::fs::remove_file(disabled_marker(config)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("enable telemetry: {}", e)),
    }
}

pub fn queue_count(config: &Config) -> usize {
    std::fs::read_dir(telemetry_root(config).join("queue"))
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .count()
}

pub fn feedback(config: &Config, resolved_policy: &Policy, receipt: &history::CoarseReceipt) {
    if !resolved_policy.enabled {
        return;
    }
    let root = telemetry_root(config);
    let Ok(send_lock) = open_lock(&root, "send.lock") else {
        return;
    };
    if send_lock.lock_exclusive().is_err() {
        return;
    }
    if !policy(config, false).enabled {
        let _ = fs2::FileExt::unlock(&send_lock);
        return;
    }
    let event = feedback_event(receipt);
    match send(&event, Duration::from_millis(100)) {
        SendResult::PreSend => {
            let _ = enqueue(config, &format!("feedback-{}", history::run_id()), &event);
        }
        SendResult::Accepted | SendResult::Ambiguous | SendResult::Rejected => {}
    }
    let _ = fs2::FileExt::unlock(&send_lock);
}

fn feedback_event(receipt: &history::CoarseReceipt) -> Event {
    Event {
        v: SCHEMA_VERSION,
        event: "feedback_summary".into(),
        release: release(),
        os: enum_or(std::env::consts::OS, OS),
        arch: enum_or(std::env::consts::ARCH, ARCH),
        shell: "other".into(),
        mode: enum_or(&receipt.mode, MODES),
        route: receipt_route(&receipt.route).into(),
        decision: receipt_decision(&receipt.decision).into(),
        effects: receipt_effect(receipt).into(),
        proposal_outcome: "valid".into(),
        execution_outcome: receipt_execution(receipt).into(),
        user_feedback: enum_or(&receipt.user_feedback, FEEDBACK),
        latency: receipt_latency(&receipt.latency_bucket).into(),
        cache: enum_or(&receipt.cache_state, CACHES),
        parent_action: "not_applicable".into(),
        interactive: false,
        notice_revision: crate::first_run::NOTICE_REVISION,
    }
}

pub fn ack_parent(config: &Config, resolved_policy: &Policy, run_id: &str, status: &str) {
    if !resolved_policy.enabled || !matches!(status, "applied" | "failed") {
        return;
    }
    if !update_parent_candidate(config, run_id, status) {
        return;
    }
    flush_older(config, Duration::from_millis(300));
}

fn update_parent_candidate(config: &Config, run_id: &str, status: &str) -> bool {
    let root = telemetry_root(config);
    let Ok(lock) = open_lock(&root, "queue.lock") else {
        return false;
    };
    if lock.lock_exclusive().is_err() {
        return false;
    }
    if !policy(config, false).enabled {
        let _ = fs2::FileExt::unlock(&lock);
        return false;
    }
    let path = root
        .join("queue")
        .join(format!("{}.json", safe_name(run_id)));
    let updated = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Event>(&bytes).ok())
        .and_then(|mut event| {
            if event.parent_action != "unknown" {
                return None;
            }
            event.parent_action = status.into();
            event.validate().ok()?;
            Some(event)
        });
    let changed = updated.is_some();
    if let Some(event) = updated {
        let _ = write_private_atomic(&path, &serde_json::to_vec(&event).unwrap_or_default());
    }
    let _ = fs2::FileExt::unlock(&lock);
    changed
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SendResult {
    Accepted,
    PreSend,
    Ambiguous,
    Rejected,
}

fn send(event: &Event, timeout: Duration) -> SendResult {
    send_to(ENDPOINT, event, timeout)
}

fn send_to(endpoint: &str, event: &Event, timeout: Duration) -> SendResult {
    let body = match serde_json::to_string(event) {
        Ok(value) => value,
        Err(_) => return SendResult::PreSend,
    };
    let agent = ureq::AgentBuilder::new()
        .try_proxy_from_env(true)
        .timeout(timeout)
        .build();
    match agent
        .post(endpoint)
        .set("Content-Type", "application/json")
        .send_string(&body)
    {
        Ok(response) if response.status() == 202 => SendResult::Accepted,
        Ok(_) | Err(ureq::Error::Status(_, _)) => SendResult::Rejected,
        Err(ureq::Error::Transport(error)) => {
            let kind = format!("{:?}", error.kind());
            if kind.contains("Dns") || kind.contains("ConnectionFailed") {
                SendResult::PreSend
            } else {
                SendResult::Ambiguous
            }
        }
    }
}

fn enqueue(config: &Config, run_id: &str, event: &Event) -> Result<(), String> {
    event.validate()?;
    let root = telemetry_root(config);
    let lock = open_lock(&root, "queue.lock")?;
    lock.lock_exclusive()
        .map_err(|e| format!("lock telemetry queue: {}", e))?;
    let queue = root.join("queue");
    dirs::ensure_private_dir(&queue)?;
    recover_and_prune(&queue);
    let path = queue.join(format!("{}.json", safe_name(run_id)));
    let bytes = serde_json::to_vec(event).map_err(|e| e.to_string())?;
    write_private_atomic(&path, &bytes)?;
    prune_count(&queue);
    fs2::FileExt::unlock(&lock).map_err(|e| e.to_string())
}

fn flush_older(config: &Config, budget: Duration) {
    let started = Instant::now();
    let root = telemetry_root(config);
    let Ok(lock) = open_lock(&root, "queue.lock") else {
        return;
    };
    if lock.lock_exclusive().is_err() {
        return;
    }
    let queue = root.join("queue");
    let _ = dirs::ensure_private_dir(&queue);
    recover_and_prune(&queue);
    let mut files = event_files(&queue);
    files.truncate(10);
    let mut claims = Vec::new();
    for path in files {
        let claim = path.with_extension("sending");
        if std::fs::rename(&path, &claim).is_ok() {
            claims.push((path, claim));
        }
    }
    let _ = fs2::FileExt::unlock(&lock);
    for (original, claim) in claims {
        let elapsed = started.elapsed();
        if elapsed >= budget {
            let _ = std::fs::rename(claim, original);
            continue;
        }
        let event = std::fs::read(&claim)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Event>(&bytes).ok());
        match event.map(|event| send(&event, (budget - elapsed).min(Duration::from_millis(100)))) {
            Some(SendResult::PreSend) => {
                let _ = std::fs::rename(claim, original);
            }
            Some(SendResult::Accepted | SendResult::Ambiguous | SendResult::Rejected) | None => {
                let _ = std::fs::remove_file(claim);
            }
        }
    }
}

fn recover_and_prune(queue: &Path) {
    let now = SystemTime::now();
    if let Ok(entries) = std::fs::read_dir(queue) {
        for entry in entries.flatten() {
            let path = entry.path();
            let age = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|m| now.duration_since(m).ok())
                .unwrap_or_default();
            if age > MAX_AGE {
                let _ = std::fs::remove_file(path);
            } else if path.extension().is_some_and(|ext| ext == "sending") {
                let _ = std::fs::rename(&path, path.with_extension("json"));
            }
        }
    }
    prune_count(queue);
}

fn prune_count(queue: &Path) {
    let files = event_files(queue);
    if files.len() > MAX_QUEUE {
        let remove = files.len() - MAX_QUEUE;
        for path in files.into_iter().take(remove) {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn event_files(queue: &Path) -> Vec<PathBuf> {
    let mut files = std::fs::read_dir(queue)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect::<Vec<_>>();
    files.sort_by_key(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok());
    files
}

fn open_lock(root: &Path, name: &str) -> Result<std::fs::File, String> {
    dirs::ensure_private_dir(root)?;
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(root.join(name))
        .map_err(|e| format!("open telemetry lock: {}", e))
}

fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("telemetry path has no parent")?;
    dirs::ensure_private_dir(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.error.to_string())?;
    Ok(())
}

fn telemetry_root(config: &Config) -> PathBuf {
    config.paths.data_dir.join("telemetry")
}
fn disabled_marker(config: &Config) -> PathBuf {
    config.paths.data_dir.join("telemetry.disabled")
}
#[cfg(test)]
fn queue_path(config: &Config, run_id: &str) -> PathBuf {
    telemetry_root(config)
        .join("queue")
        .join(format!("{}.json", safe_name(run_id)))
}
fn safe_name(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(64)
        .collect()
}
fn release() -> String {
    env!("CARGO_PKG_VERSION")
        .split('.')
        .take(2)
        .collect::<Vec<_>>()
        .join(".")
}
fn enum_or(value: &str, allowed: &[&str]) -> String {
    if allowed.contains(&value) {
        value.into()
    } else {
        allowed.last().copied().unwrap_or("unknown").into()
    }
}
fn latency(value: Duration) -> &'static str {
    if value < Duration::from_secs(1) {
        "lt_1s"
    } else if value < Duration::from_secs(2) {
        "1s_2s"
    } else if value < Duration::from_secs(5) {
        "2s_5s"
    } else {
        "gte_5s"
    }
}
fn dominant_effect(effects: &[Effect]) -> &'static str {
    effects
        .iter()
        .map(Effect::label)
        .max_by_key(|v| EFFECTS.iter().position(|e| e == v).unwrap_or(0))
        .unwrap_or("none")
}
fn receipt_effect(receipt: &history::CoarseReceipt) -> &'static str {
    receipt
        .declared_effects
        .iter()
        .chain(&receipt.detected_effects)
        .filter_map(|v| EFFECTS.iter().copied().find(|e| *e == v))
        .max_by_key(|v| EFFECTS.iter().position(|e| e == v).unwrap_or(0))
        .unwrap_or("none")
}
fn receipt_route(value: &str) -> &'static str {
    match value {
        "return_answer" | "answer" | "ask" | "explain" => "answer",
        "run_shell" | "shell" => "shell",
        "require_parent_shell" => "parent_shell",
        "request_clarification" => "clarification",
        _ => "unknown",
    }
}
fn receipt_decision(value: &str) -> &'static str {
    match value {
        "completed" | "failed" | "timed_out" => "ran",
        "answer" => "returned",
        "not_applied" => "needs_parent",
        _ => "not_run",
    }
}
fn receipt_execution(receipt: &history::CoarseReceipt) -> &'static str {
    if !receipt.execution_attempted {
        "not_run"
    } else if receipt.signal.is_some() {
        "signal"
    } else {
        match receipt.exit_category.as_str() {
            "success" => "exit_zero",
            "failure" => "exit_nonzero",
            _ => "not_run",
        }
    }
}
fn receipt_latency(value: &str) -> &'static str {
    match value {
        "lt_1s" => "lt_1s",
        "1_5s" => "2s_5s",
        "gte_5s" => "gte_5s",
        _ => "gte_5s",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dirs::Paths;
    use std::sync::{Mutex, OnceLock};

    fn config(root: &Path) -> Config {
        Config::test(Paths {
            config_file: root.join("config"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
        })
    }

    #[test]
    fn event_contains_only_fixed_enum_fields() {
        let event = preview("auto", true);
        event.validate().unwrap();
        let value = serde_json::to_value(event).unwrap();
        for prohibited in [
            "prompt",
            "command",
            "cwd",
            "path",
            "repository",
            "error",
            "timestamp",
            "user_id",
            "session_id",
            "model",
        ] {
            assert!(value.get(prohibited).is_none());
        }
    }

    #[test]
    fn every_opt_out_precedes_queue_access() {
        let root = tempfile::tempdir().unwrap();
        let mut config = config(root.path());

        assert!(!policy(&config, true).enabled);
        assert!(!telemetry_root(&config).exists());

        config.telemetry.enabled = false;
        assert!(!policy(&config, false).enabled);
        assert!(!telemetry_root(&config).exists());
        config.telemetry.enabled = true;

        dirs::ensure_private_dir(&config.paths.data_dir).unwrap();
        write_private_atomic(&disabled_marker(&config), b"off").unwrap();
        assert!(!policy(&config, false).enabled);
        assert!(!telemetry_root(&config).exists());
        std::fs::remove_file(disabled_marker(&config)).unwrap();

        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        for (name, value) in [("DO_NOT_TRACK", "1"), ("UHM_TELEMETRY", "off")] {
            std::env::set_var(name, value);
            assert!(!policy(&config, false).enabled);
            assert!(!telemetry_root(&config).exists());
            std::env::remove_var(name);
        }
    }

    #[test]
    fn queue_is_private_and_bounded() {
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        let event = preview("auto", false);
        for i in 0..25 {
            enqueue(&config, &format!("run-{i}"), &event).unwrap();
        }
        assert_eq!(queue_count(&config), MAX_QUEUE);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = event_files(&telemetry_root(&config).join("queue"))[0].clone();
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        disable(&config).unwrap();
        assert_eq!(queue_count(&config), 0);
    }

    #[test]
    fn interrupted_claim_is_recovered_without_corrupting_the_event() {
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        let event = preview("auto", false);
        enqueue(&config, "recover-me", &event).unwrap();
        let queued = queue_path(&config, "recover-me");
        let claim = queued.with_extension("sending");
        std::fs::rename(&queued, &claim).unwrap();

        recover_and_prune(&telemetry_root(&config).join("queue"));

        assert!(!claim.exists());
        assert_eq!(
            serde_json::from_slice::<Event>(&std::fs::read(queued).unwrap()).unwrap(),
            event
        );
    }

    #[test]
    fn parent_action_stays_unknown_until_matching_acknowledgement() {
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        let mut event = preview("auto", false);
        event.parent_action = "unknown".into();
        enqueue(&config, "parent-run", &event).unwrap();
        assert!(!update_parent_candidate(
            &config,
            "different-run",
            "applied"
        ));
        assert!(update_parent_candidate(&config, "parent-run", "failed"));
        let updated: Event =
            serde_json::from_slice(&std::fs::read(queue_path(&config, "parent-run")).unwrap())
                .unwrap();
        assert_eq!(updated.parent_action, "failed");
        assert!(!update_parent_candidate(&config, "parent-run", "applied"));
    }

    #[test]
    fn unknown_values_are_rejected() {
        let mut event = preview("auto", false);
        event.route = "/private/path".into();
        assert!(event.validate().is_err());
        assert!(serde_json::from_value::<Event>(serde_json::json!({"v":1,"unknown":"x"})).is_err());
    }

    #[test]
    fn unreachable_sender_obeys_the_explicit_deadline() {
        let event = preview("auto", false);
        let started = Instant::now();
        let result = send_to(
            "http://127.0.0.1:9/v1/events",
            &event,
            Duration::from_millis(100),
        );
        assert!(matches!(
            result,
            SendResult::PreSend | SendResult::Ambiguous
        ));
        assert!(started.elapsed() < Duration::from_millis(250));
    }
}
