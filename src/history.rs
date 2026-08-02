//! Private, append-only local decision history.
//!
//! The JSONL journal is authoritative. Content-bearing values live only in a
//! run directory and only when the user explicitly selects a richer detail
//! level; telemetry consumes `CoarseReceipt`, which cannot carry content.

use crate::action::ProposedAction;
use crate::config::{HistoryConfig, HistoryDetail};
use crate::dirs;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 1;
const JOURNAL: &str = "history.v1.jsonl";
const LEGACY: &str = "history.jsonl";
const LOCK: &str = "history.lock";
const RUNS: &str = "runs";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub schema_version: u32,
    pub run_id: String,
    pub sequence: u64,
    pub timestamp: u64,
    pub app_version: String,
    pub model: String,
    pub prompt_schema_version: u32,
    pub route: String,
    pub mode: String,
    pub context_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_run_id: Option<String>,
    pub kind: EventKind,
    #[serde(default)]
    pub data: Value,
    pub checksum: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    RequestCreated,
    ContextSelected,
    ProposalReceived,
    ClarificationRequested,
    UserFeedbackReceived,
    WarningShown,
    UserDecision,
    ExecutionStarted,
    ExecutionFinished,
    ArtifactRecorded,
    JobFinished,
    MigratedReceipt,
    ParentActionAcknowledged,
    RecoveryClassified,
    RecoveryPrepared,
    RecoveryCommitted,
    RecoveryUnavailable,
    UndoStarted,
    UndoItemFinished,
    UndoFinished,
    ForcedRestoreFinished,
    RecoveryExpired,
    BestEffortInverseRequested,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub run_id: String,
    pub detail: String,
    pub created_at: u64,
    pub artifacts: BTreeMap<String, Artifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub file: String,
    pub bytes: u64,
    pub checksum: String,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct Journal {
    pub events: Vec<Event>,
    pub truncated_final_line: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub enabled: bool,
    pub detail: String,
    pub capture_output: bool,
    pub redact_paths: bool,
    pub events: usize,
    pub runs: usize,
    pub bytes: u64,
    pub max_records: usize,
    pub max_age_days: u64,
    pub max_bytes: u64,
    pub journal: PathBuf,
    pub truncated_final_line: bool,
    pub last_write_ok: bool,
}

/// Legacy Plan 2 shape, retained only for migration and command integration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub schema_version: u32,
    pub run_id: String,
    pub timestamp: u64,
    pub app_version: String,
    pub mode: String,
    pub context_mode: String,
    pub route: String,
    #[serde(default = "no_runtime")]
    pub runtime: String,
    pub prompt_schema_version: u32,
    pub declared_effects: Vec<String>,
    pub detected_effects: Vec<String>,
    pub decision: String,
    pub execution_attempted: bool,
    pub exit_category: String,
    pub signal: Option<i32>,
    pub latency_bucket: String,
    pub cache_state: String,
    pub second_turn_used: bool,
    #[serde(default = "unknown_feedback")]
    pub user_feedback: String,
}

/// Strict allowlist used by telemetry. It has no ID, text, paths, or output.
#[derive(Debug, Clone)]
pub struct CoarseReceipt {
    pub mode: String,
    pub route: String,
    pub declared_effects: Vec<String>,
    pub detected_effects: Vec<String>,
    pub decision: String,
    pub execution_attempted: bool,
    pub exit_category: String,
    pub signal: Option<i32>,
    pub latency_bucket: String,
    pub cache_state: String,
    pub user_feedback: String,
}

fn unknown_feedback() -> String {
    "unknown".into()
}
fn no_runtime() -> String {
    "none".into()
}

pub fn run_id() -> String {
    let seed = format!(
        "{}:{}:{:?}",
        now_secs(),
        std::process::id(),
        std::thread::current().id()
    );
    blake3::hash(seed.as_bytes()).to_hex()[..20].to_string()
}

pub fn now_secs() -> u64 {
    use crate::clock::{Clock as _, SystemClock};
    SystemClock.unix_seconds()
}

fn journal_path(data: &Path) -> PathBuf {
    data.join(JOURNAL)
}
fn runs_path(data: &Path) -> PathBuf {
    data.join(RUNS)
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.len() < 8 || id.len() > 64 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("invalid history run ID".into());
    }
    Ok(())
}

fn private_file(path: &Path, create_new: bool) -> Result<std::fs::File, String> {
    let mut options = std::fs::OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(!create_new)
        .create_new(create_new);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|e| format!("open {}: {}", path.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    Ok(file)
}

fn lock(data: &Path) -> Result<std::fs::File, String> {
    if !data.is_absolute() {
        return Err("history data directory must be absolute".into());
    }
    dirs::ensure_private_dir(data)?;
    let file = private_file(&data.join(LOCK), false)?;
    file.lock_exclusive()
        .map_err(|e| format!("lock history: {}", e))?;
    migrate_locked(data)?;
    Ok(file)
}

fn checksum(event: &Event) -> Result<String, String> {
    let mut unsigned = event.clone();
    unsigned.checksum.clear();
    let bytes = serde_json::to_vec(&unsigned).map_err(|e| e.to_string())?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn verify(event: &Event) -> Result<(), String> {
    if event.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "unsupported history schema {}",
            event.schema_version
        ));
    }
    if event.checksum != checksum(event)? {
        return Err("history checksum mismatch".into());
    }
    validate_id(&event.run_id)
}

fn read_unlocked(path: &Path) -> Result<Journal, String> {
    let mut bytes = Vec::new();
    match std::fs::File::open(path) {
        Ok(mut file) => file.read_to_end(&mut bytes).map_err(|e| e.to_string())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Journal {
                events: vec![],
                truncated_final_line: false,
            })
        }
        Err(e) => return Err(format!("read history: {}", e)),
    };
    let truncated = !bytes.is_empty() && !bytes.ends_with(b"\n");
    let lines: Vec<&[u8]> = bytes.split(|b| *b == b'\n').collect();
    let complete = if truncated {
        lines.len().saturating_sub(1)
    } else {
        lines.len()
    };
    let mut events = Vec::new();
    for (index, line) in lines[..complete].iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let event: Event = serde_json::from_slice(line).map_err(|e| {
            format!(
                "history corruption at line {}: {}; export or restore the journal before writing",
                index + 1,
                e
            )
        })?;
        verify(&event).map_err(|e| format!("history corruption at line {}: {}", index + 1, e))?;
        events.push(event);
    }
    Ok(Journal {
        events,
        truncated_final_line: truncated,
    })
}

pub fn read(data: &Path) -> Result<Journal, String> {
    let _guard = lock(data)?;
    read_unlocked(&journal_path(data))
}

fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("history path has no parent")?;
    dirs::ensure_private_dir(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    tmp.write_all(bytes).map_err(|e| e.to_string())?;
    tmp.as_file().sync_all().map_err(|e| e.to_string())?;
    tmp.persist(path).map_err(|e| e.error.to_string())?;
    Ok(())
}

fn append_locked(data: &Path, mut event: Event) -> Result<(), String> {
    let path = journal_path(data);
    let journal = read_unlocked(&path)?;
    if journal.truncated_final_line {
        let mut bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        if let Some(pos) = bytes.iter().rposition(|b| *b == b'\n') {
            bytes.truncate(pos + 1);
        } else {
            bytes.clear();
        }
        write_private_atomic(&path, &bytes)?;
        eprintln!("uhm: history: ignored a truncated final journal line");
    }
    event.sequence = journal
        .events
        .iter()
        .filter(|e| e.run_id == event.run_id)
        .map(|e| e.sequence)
        .max()
        .unwrap_or(0)
        + 1;
    event.checksum = checksum(&event)?;
    let mut file = private_file(&path, false)?;
    file.seek(std::io::SeekFrom::End(0))
        .map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut file, &event).map_err(|e| e.to_string())?;
    file.write_all(b"\n").map_err(|e| e.to_string())?;
    file.sync_data()
        .map_err(|e| format!("flush history: {}", e))
}

use std::io::Seek;

fn base_event(
    run: &str,
    route: &str,
    mode: &str,
    context_mode: &str,
    kind: EventKind,
    data: Value,
    related: Option<&str>,
) -> Event {
    Event {
        schema_version: SCHEMA_VERSION,
        run_id: run.into(),
        sequence: 0,
        timestamp: now_secs(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        model: "unknown".into(),
        prompt_schema_version: crate::prompt::PROMPT_VERSION,
        route: route.into(),
        mode: mode.into(),
        context_mode: context_mode.into(),
        related_run_id: related.map(str::to_owned),
        kind,
        data,
        checksum: String::new(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn record_request(
    data: &Path,
    cfg: &HistoryConfig,
    run: &str,
    route: &str,
    mode: &str,
    context_mode: &str,
    intent: &str,
    related: Option<&str>,
) -> Result<(), String> {
    if !cfg.enabled {
        return Ok(());
    }
    validate_id(run)?;
    let value = if cfg.detail == HistoryDetail::Full {
        let redacted = redact(intent, cfg.redact_paths);
        let (retained, truncated) = truncate_utf8(&redacted, cfg.artifact_max_bytes);
        json!({"intent": retained, "intent_truncated": truncated, "intent_bytes": intent.len(), "retained_bytes": retained.len()})
    } else {
        json!({"intent_hash": blake3::hash(intent.as_bytes()).to_hex().to_string(), "bytes": intent.len()})
    };
    let _guard = lock(data)?;
    append_locked(
        data,
        base_event(
            run,
            route,
            mode,
            context_mode,
            EventKind::RequestCreated,
            value,
            related,
        ),
    )
}

pub fn record_context(
    data: &Path,
    cfg: &HistoryConfig,
    run: &str,
    route: &str,
    mode: &str,
    context_mode: &str,
    related: Option<&str>,
) -> Result<(), String> {
    if !cfg.enabled {
        return Ok(());
    }
    let _guard = lock(data)?;
    append_locked(
        data,
        base_event(
            run,
            route,
            mode,
            context_mode,
            EventKind::ContextSelected,
            json!({"level":context_mode}),
            related,
        ),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn record_proposal(
    data: &Path,
    cfg: &HistoryConfig,
    run: &str,
    route: &str,
    mode: &str,
    context_mode: &str,
    proposal: &ProposedAction,
    related: Option<&str>,
) -> Result<(), String> {
    if !cfg.enabled {
        return Ok(());
    }
    let serialized = serde_json::to_vec_pretty(proposal).map_err(|e| e.to_string())?;
    let kind = match proposal {
        ProposedAction::Answer { .. } => "answer",
        ProposedAction::Shell { .. } => "shell",
        ProposedAction::ParentShell { .. } => "parent_shell",
        ProposedAction::Program { .. } => "program",
        ProposedAction::Clarification { .. } => "clarification",
    };
    let mut value = json!({"proposal_kind":kind,"proposal_hash":blake3::hash(&serialized).to_hex().to_string(),"bytes":serialized.len(),"retained":false});
    let _guard = lock(data)?;
    if cfg.detail.retains_proposals() {
        let artifact = write_artifact_locked(data, cfg, run, "proposal.json", &serialized)?;
        value["retained"] = Value::Bool(true);
        value["artifact"] = Value::String(artifact.file.clone());
        append_locked(
            data,
            base_event(
                run,
                route,
                mode,
                context_mode,
                EventKind::ArtifactRecorded,
                serde_json::to_value(&artifact).unwrap_or_default(),
                related,
            ),
        )?;
    }
    append_locked(
        data,
        base_event(
            run,
            route,
            mode,
            context_mode,
            EventKind::ProposalReceived,
            value,
            related,
        ),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn record_output(
    data: &Path,
    cfg: &HistoryConfig,
    run: &str,
    route: &str,
    context_mode: &str,
    stdout: Option<&[u8]>,
    stderr: Option<&[u8]>,
    failed: bool,
) -> Result<(), String> {
    if !cfg.enabled || cfg.detail == HistoryDetail::Metadata {
        return Ok(());
    }
    let _guard = lock(data)?;
    let mut recorded = Vec::new();
    if cfg.capture_output {
        if let Some(bytes) = stdout.filter(|v| !v.is_empty()) {
            recorded.push(write_artifact_locked(data, cfg, run, "stdout.tail", bytes)?);
        }
    }
    if failed || cfg.capture_output {
        if let Some(bytes) = stderr.filter(|v| !v.is_empty()) {
            recorded.push(write_artifact_locked(data, cfg, run, "stderr.tail", bytes)?);
        }
    }
    for artifact in recorded {
        append_locked(
            data,
            base_event(
                run,
                route,
                route,
                context_mode,
                EventKind::ArtifactRecorded,
                serde_json::to_value(artifact).unwrap_or_default(),
                None,
            ),
        )?;
    }
    Ok(())
}

fn write_artifact_locked(
    data: &Path,
    cfg: &HistoryConfig,
    run: &str,
    name: &str,
    bytes: &[u8],
) -> Result<Artifact, String> {
    validate_id(run)?;
    if Path::new(name)
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("invalid history artifact name".into());
    }
    let root = runs_path(data);
    dirs::ensure_private_dir(&root)?;
    let dir = root.join(run);
    if !dir.exists() {
        std::fs::create_dir(&dir).map_err(|e| format!("create run directory: {}", e))?;
    }
    dirs::ensure_private_dir(&dir)?;
    let kept = bytes.len().min(cfg.artifact_max_bytes);
    let payload = &bytes[..kept];
    write_private_atomic(&dir.join(name), payload)?;
    let artifact = Artifact {
        file: name.into(),
        bytes: payload.len() as u64,
        checksum: blake3::hash(payload).to_hex().to_string(),
        truncated: kept < bytes.len(),
    };
    let manifest_path = dir.join("manifest.json");
    let mut manifest = std::fs::read(&manifest_path)
        .ok()
        .and_then(|v| serde_json::from_slice::<Manifest>(&v).ok())
        .unwrap_or(Manifest {
            schema_version: SCHEMA_VERSION,
            run_id: run.into(),
            detail: cfg.detail.as_str().into(),
            created_at: now_secs(),
            artifacts: BTreeMap::new(),
        });
    manifest.artifacts.insert(name.into(), artifact.clone());
    write_private_atomic(
        &manifest_path,
        &serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
    )?;
    Ok(artifact)
}

fn redact(value: &str, redact_paths: bool) -> String {
    if !redact_paths {
        return value.into();
    }
    value
        .split_whitespace()
        .map(|part| {
            if part.starts_with('/') || part.starts_with("~/") {
                "<path>"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate_utf8(value: &str, max_bytes: usize) -> (&str, bool) {
    if value.len() <= max_bytes {
        return (value, false);
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (&value[..end], true)
}

pub fn append_receipt(data: &Path, cfg: &HistoryConfig, receipt: &Receipt) -> Result<(), String> {
    if !cfg.enabled {
        return Ok(());
    }
    let payload = json!({
        "runtime":receipt.runtime,"declared_effects":receipt.declared_effects,"detected_effects":receipt.detected_effects,
        "decision":receipt.decision,"execution_attempted":receipt.execution_attempted,"exit_category":receipt.exit_category,
        "signal":receipt.signal,"latency_bucket":receipt.latency_bucket,"cache_state":receipt.cache_state,
        "second_turn_used":receipt.second_turn_used,"user_feedback":receipt.user_feedback
    });
    let kind = if receipt.execution_attempted {
        EventKind::ExecutionFinished
    } else {
        EventKind::JobFinished
    };
    let _guard = lock(data)?;
    let related = read_unlocked(&journal_path(data))?
        .events
        .iter()
        .find(|event| event.run_id == receipt.run_id)
        .and_then(|event| event.related_run_id.clone());
    append_locked(
        data,
        base_event(
            &receipt.run_id,
            &receipt.route,
            &receipt.mode,
            &receipt.context_mode,
            EventKind::UserDecision,
            json!({"decision":receipt.decision}),
            related.as_deref(),
        ),
    )?;
    if receipt.execution_attempted {
        append_locked(
            data,
            base_event(
                &receipt.run_id,
                &receipt.route,
                &receipt.mode,
                &receipt.context_mode,
                EventKind::ExecutionStarted,
                json!({}),
                related.as_deref(),
            ),
        )?;
    }
    let mut event = base_event(
        &receipt.run_id,
        &receipt.route,
        &receipt.mode,
        &receipt.context_mode,
        kind,
        payload,
        related.as_deref(),
    );
    event.timestamp = receipt.timestamp;
    event.app_version = receipt.app_version.clone();
    event.prompt_schema_version = receipt.prompt_schema_version;
    append_locked(data, event)?;
    if kind == EventKind::ExecutionFinished {
        append_locked(
            data,
            base_event(
                &receipt.run_id,
                &receipt.route,
                &receipt.mode,
                &receipt.context_mode,
                EventKind::JobFinished,
                json!({"decision":receipt.decision,"exit_category":receipt.exit_category}),
                related.as_deref(),
            ),
        )?;
    }
    prune_locked(data, cfg, false).map(|_| ())
}

pub fn resolve_run(data: &Path, id: &str) -> Result<String, String> {
    let journal = read(data)?;
    if id == "last" {
        return journal
            .events
            .last()
            .map(|e| e.run_id.clone())
            .ok_or("no local history is available".into());
    }
    validate_id(id)?;
    if journal.events.iter().any(|e| e.run_id == id) {
        Ok(id.into())
    } else {
        Err(format!("history run '{}' was not found", id))
    }
}

pub fn events_for(data: &Path, id: &str) -> Result<Vec<Event>, String> {
    let id = resolve_run(data, id)?;
    Ok(read(data)?
        .events
        .into_iter()
        .filter(|e| e.run_id == id)
        .collect())
}

pub fn load_proposal(data: &Path, id: &str) -> Result<(String, ProposedAction), String> {
    let id = resolve_run(data, id)?;
    let path = runs_path(data).join(&id).join("proposal.json");
    let bytes = std::fs::read(&path).map_err(|_| "replay unavailable: this run did not retain an exact proposal; set history.detail to diagnostic or full for future runs".to_string())?;
    let action = serde_json::from_slice::<ProposedAction>(&bytes)
        .map_err(|e| format!("stored proposal is invalid: {}", e))?
        .validate()?;
    Ok((id, action))
}

pub fn repair_seed(
    data: &Path,
    id: &str,
    feedback: Option<&str>,
) -> Result<(String, String), String> {
    let id = resolve_run(data, id)?;
    let events = events_for(data, &id)?;
    if events.iter().any(|event| {
        event.kind == EventKind::RequestCreated
            && event.data.get("intent_truncated").and_then(Value::as_bool) == Some(true)
    }) {
        return Err("repair unavailable: the retained intent was truncated".into());
    }
    let intent = events.iter().find(|e| e.kind == EventKind::RequestCreated).and_then(|e| e.data.get("intent")).and_then(Value::as_str)
        .ok_or("repair unavailable: the original intent was not retained; set history.detail to full for future runs")?;
    let (_, action) = load_proposal(data, &id)?;
    let outcome = events
        .iter()
        .rev()
        .find(|e| {
            matches!(
                e.kind,
                EventKind::ExecutionFinished | EventKind::JobFinished
            )
        })
        .map(|e| e.data.clone())
        .unwrap_or_default();
    let seed = format!("Repair this prior bounded terminal job. Original intent: {}\nPrior typed proposal: {}\nObserved coarse outcome: {}{}", intent, serde_json::to_string(&action).unwrap_or_default(), outcome, feedback.map(|v| format!("\nUser feedback: {}", v)).unwrap_or_default());
    Ok((id, seed))
}

pub fn recovery_seed(
    data: &Path,
    id: &str,
    guidance: Option<&str>,
) -> Result<(String, String), String> {
    let id = resolve_run(data, id)?;
    let events = events_for(data, &id)?;
    if events.iter().any(|event| {
        event.kind == EventKind::RequestCreated
            && event.data.get("intent_truncated").and_then(Value::as_bool) == Some(true)
    }) {
        return Err("best-effort recovery unavailable: the retained intent was truncated".into());
    }
    if events.iter().any(|event| event.route == "recover")
        || events
            .first()
            .is_some_and(|event| event.related_run_id.is_some())
    {
        return Err("best-effort recovery cannot be chained or recursively applied to a linked recovery job".into());
    }
    let intent = events
        .iter()
        .find(|event| event.kind == EventKind::RequestCreated)
        .and_then(|event| event.data.get("intent"))
        .and_then(Value::as_str)
        .ok_or("best-effort recovery unavailable: the original intent was not retained; set history.detail to full for future runs")?;
    let (_, action) = load_proposal(data, &id)?;
    if matches!(
        action,
        ProposedAction::Answer { .. } | ProposedAction::Clarification { .. }
    ) {
        return Err(
            "best-effort recovery unavailable: the retained proposal did not perform an action"
                .into(),
        );
    }
    let outcome = events
        .iter()
        .rev()
        .find(|event| {
            matches!(
                event.kind,
                EventKind::ExecutionFinished | EventKind::JobFinished
            )
        })
        .map(|event| event.data.clone())
        .unwrap_or_else(|| json!({"result":"unknown"}));
    let guidance = guidance.map(|value| {
        value
            .chars()
            .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
            .take(4096)
            .collect::<String>()
    });
    let subset = json!({
        "schema": "uhm.best_effort_inverse.v1",
        "label": "best_effort_inverse",
        "source_run_id": id,
        "original_intent": intent,
        "typed_proposal": action,
        "coarse_outcome": outcome,
        "guidance": guidance,
        "constraint": "Propose one bounded action only. Do not claim that executing it restores the original state."
    });
    let seed = serde_json::to_string_pretty(&subset).map_err(|error| error.to_string())?;
    if seed.len() > 96 * 1024 {
        return Err("best-effort recovery subset exceeds the bounded model-request limit".into());
    }
    Ok((id, seed))
}

pub fn list(
    data: &Path,
    limit: usize,
    failed: bool,
    route: Option<&str>,
) -> Result<Vec<Value>, String> {
    let journal = read(data)?;
    let mut grouped: BTreeMap<String, Vec<Event>> = BTreeMap::new();
    for event in journal.events {
        grouped.entry(event.run_id.clone()).or_default().push(event);
    }
    let mut rows: Vec<Value> = grouped.into_values().filter_map(|events| {
        let last = events.last()?;
        let failed_run = events.iter().any(|e| e.data.get("exit_category").and_then(Value::as_str).is_some_and(|v| matches!(v,"failure"|"signal"|"timed_out")));
        if failed && !failed_run { return None; }
        if route.is_some_and(|r| r != last.route) { return None; }
        Some(json!({"run_id":last.run_id,"timestamp":last.timestamp,"route":last.route,"mode":last.mode,"events":events.len(),"failed":failed_run,"outcome":last.data}))
    }).collect();
    rows.sort_by_key(|v| v["timestamp"].as_u64().unwrap_or(0));
    Ok(rows.into_iter().rev().take(limit).collect())
}

pub fn search(data: &Path, needle: &str) -> Result<Vec<Event>, String> {
    let needle = needle.to_lowercase();
    Ok(read(data)?
        .events
        .into_iter()
        .filter(|e| {
            serde_json::to_string(e)
                .unwrap_or_default()
                .to_lowercase()
                .contains(&needle)
        })
        .take(100)
        .collect())
}

pub fn status(data: &Path, cfg: &HistoryConfig) -> Result<Status, String> {
    if !cfg.enabled && !data.exists() {
        return Ok(Status {
            enabled: false,
            detail: cfg.detail.as_str().into(),
            capture_output: cfg.capture_output,
            redact_paths: cfg.redact_paths,
            events: 0,
            runs: 0,
            bytes: 0,
            max_records: cfg.max_records,
            max_age_days: cfg.max_age_days,
            max_bytes: cfg.max_bytes,
            journal: journal_path(data),
            truncated_final_line: false,
            last_write_ok: true,
        });
    }
    let journal = read(data)?;
    let ids: BTreeSet<_> = journal.events.iter().map(|e| &e.run_id).collect();
    let bytes = tree_bytes(data).unwrap_or(0);
    Ok(Status {
        enabled: cfg.enabled,
        detail: cfg.detail.as_str().into(),
        capture_output: cfg.capture_output,
        redact_paths: cfg.redact_paths,
        events: journal.events.len(),
        runs: ids.len(),
        bytes,
        max_records: cfg.max_records,
        max_age_days: cfg.max_age_days,
        max_bytes: cfg.max_bytes,
        journal: journal_path(data),
        truncated_final_line: journal.truncated_final_line,
        last_write_ok: !journal.truncated_final_line,
    })
}

fn tree_bytes(root: &Path) -> Result<u64, String> {
    let mut total = 0;
    if !root.exists() {
        return Ok(0);
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let meta = std::fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                pending.push(entry.path());
            } else {
                total += meta.len();
            }
        }
    }
    Ok(total)
}

fn encode_events(events: &[Event]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for event in events {
        serde_json::to_writer(&mut out, event).map_err(|e| e.to_string())?;
        out.push(b'\n');
    }
    Ok(out)
}

fn prune_locked(data: &Path, cfg: &HistoryConfig, dry_run: bool) -> Result<(usize, u64), String> {
    let journal = read_unlocked(&journal_path(data))?;
    let cutoff = now_secs().saturating_sub(cfg.max_age_days.saturating_mul(86_400));
    let mut keep = journal.events;
    keep.retain(|e| e.timestamp >= cutoff);
    if keep.len() > cfg.max_records {
        keep.drain(..keep.len() - cfg.max_records);
    }
    while encode_events(&keep)?.len() as u64 > cfg.max_bytes && !keep.is_empty() {
        keep.remove(0);
    }
    let kept_ids: BTreeSet<String> = keep.iter().map(|e| e.run_id.clone()).collect();
    let original = read_unlocked(&journal_path(data))?.events;
    let removed = original.len().saturating_sub(keep.len());
    let before = tree_bytes(data).unwrap_or(0);
    if !dry_run {
        write_private_atomic(&journal_path(data), &encode_events(&keep)?)?;
        let root = runs_path(data);
        if root.exists() {
            for entry in std::fs::read_dir(&root).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                if entry.file_type().map_err(|e| e.to_string())?.is_symlink() {
                    return Err("refusing to prune a symlink under the history run root".into());
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let recovery_owned = entry
                    .path()
                    .join("recovery.json")
                    .symlink_metadata()
                    .is_ok();
                if !kept_ids.contains(&name) && !recovery_owned {
                    std::fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?;
                }
            }
        }
    }
    Ok((
        removed,
        before.saturating_sub(if dry_run {
            before
        } else {
            tree_bytes(data).unwrap_or(before)
        }),
    ))
}

pub fn prune(data: &Path, cfg: &HistoryConfig, dry_run: bool) -> Result<(usize, u64), String> {
    let _guard = lock(data)?;
    prune_locked(data, cfg, dry_run)
}

pub fn clear(data: &Path) -> Result<usize, String> {
    if !data.is_absolute() || data.parent().is_none() {
        return Err("refusing to clear an unsafe history root".into());
    }
    let _guard = lock(data)?;
    let root = runs_path(data);
    if root.exists() {
        if root
            .symlink_metadata()
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("refusing to clear a symlinked history run root".into());
        }
        let mut preserved = 0usize;
        for entry in std::fs::read_dir(&root).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().map_err(|e| e.to_string())?.is_symlink() {
                return Err(
                    "refusing to clear through a symlink under the history run root".into(),
                );
            }
            if entry
                .path()
                .join("recovery.json")
                .symlink_metadata()
                .is_ok()
            {
                preserved += 1;
            } else if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                std::fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?;
            } else {
                std::fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
            }
        }
        write_private_atomic(&journal_path(data), b"")?;
        return Ok(preserved);
    }
    write_private_atomic(&journal_path(data), b"")?;
    Ok(0)
}

pub fn clear_before(data: &Path, cutoff: u64) -> Result<usize, String> {
    let _guard = lock(data)?;
    let journal = read_unlocked(&journal_path(data))?;
    let original = journal.events.len();
    let keep: Vec<Event> = journal
        .events
        .into_iter()
        .filter(|event| event.timestamp >= cutoff)
        .collect();
    let kept_ids: BTreeSet<String> = keep.iter().map(|event| event.run_id.clone()).collect();
    write_private_atomic(&journal_path(data), &encode_events(&keep)?)?;
    let root = runs_path(data);
    if root.exists() {
        for entry in std::fs::read_dir(&root).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().map_err(|e| e.to_string())?.is_symlink() {
                return Err(
                    "refusing to clear through a symlink under the history run root".into(),
                );
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            let recovery_owned = entry
                .path()
                .join("recovery.json")
                .symlink_metadata()
                .is_ok();
            if !kept_ids.contains(&id) && !recovery_owned {
                std::fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(original.saturating_sub(keep.len()))
}

pub fn export(data: &Path, output: &Path, include_content: bool) -> Result<usize, String> {
    if !output.is_absolute() {
        return Err("history export output must be an absolute path".into());
    }
    let journal = read(data)?;
    let values: Vec<Value> = journal
        .events
        .iter()
        .map(|e| {
            if include_content {
                serde_json::to_value(e).unwrap_or_default()
            } else {
                redacted_export_event(e)
            }
        })
        .collect();
    let mut bytes = Vec::new();
    for value in &values {
        serde_json::to_writer(&mut bytes, value).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
    }
    write_private_atomic(output, &bytes)?;
    Ok(values.len())
}

fn redacted_export_event(event: &Event) -> Value {
    const DATA_KEYS: &[&str] = &[
        "bytes",
        "retained_bytes",
        "level",
        "proposal_kind",
        "runtime",
        "declared_effects",
        "detected_effects",
        "decision",
        "execution_attempted",
        "exit_category",
        "signal",
        "latency_bucket",
        "cache_state",
        "second_turn_used",
        "user_feedback",
        "result",
        "state",
        "item_count",
        "attempt",
    ];
    let mut data = serde_json::Map::new();
    if let Some(source) = event.data.as_object() {
        for key in DATA_KEYS {
            if let Some(value) = source.get(*key) {
                data.insert((*key).into(), value.clone());
            }
        }
    }
    json!({
        "export_schema_version": 1,
        "schema_version": event.schema_version,
        "timestamp": event.timestamp,
        "app_version": event.app_version,
        "prompt_schema_version": event.prompt_schema_version,
        "route": event.route,
        "mode": event.mode,
        "context_mode": event.context_mode,
        "kind": event.kind,
        "data": data,
    })
}

pub fn set_feedback(
    data: &Path,
    feedback: &str,
    selected: Option<&str>,
) -> Result<CoarseReceipt, String> {
    if !matches!(feedback, "good" | "bad") {
        return Err("feedback must be good or bad".into());
    }
    let id = resolve_run(data, selected.unwrap_or("last"))?;
    let events = events_for(data, &id)?;
    let final_event = events
        .last()
        .ok_or("no local interaction receipt is available")?;
    let mut event = base_event(
        &id,
        &final_event.route,
        &final_event.mode,
        &final_event.context_mode,
        EventKind::UserFeedbackReceived,
        json!({"feedback":feedback}),
        final_event.related_run_id.as_deref(),
    );
    event.model = final_event.model.clone();
    let _guard = lock(data)?;
    append_locked(data, event)?;
    Ok(coarse_from_event(final_event, feedback))
}

pub fn record_parent_ack(
    data: &Path,
    cfg: &HistoryConfig,
    run: &str,
    status: &str,
) -> Result<(), String> {
    if !cfg.enabled {
        return Ok(());
    }
    if !matches!(status, "applied" | "failed") {
        return Err("parent acknowledgement must be applied or failed".into());
    }
    let events = events_for(data, run)?;
    let last = events.last().ok_or("history run is unavailable")?;
    let event = base_event(
        run,
        &last.route,
        &last.mode,
        &last.context_mode,
        EventKind::ParentActionAcknowledged,
        json!({"status":status}),
        last.related_run_id.as_deref(),
    );
    let _guard = lock(data)?;
    append_locked(data, event)
}

#[allow(clippy::too_many_arguments)]
pub fn record_recovery_event(
    data: &Path,
    cfg: &HistoryConfig,
    run: &str,
    route: &str,
    context_mode: &str,
    kind: EventKind,
    state: &str,
    reason: Option<&str>,
    item_count: usize,
    related: Option<&str>,
) -> Result<(), String> {
    if !cfg.enabled {
        return Ok(());
    }
    validate_id(run)?;
    if !matches!(
        kind,
        EventKind::RecoveryClassified
            | EventKind::RecoveryPrepared
            | EventKind::RecoveryCommitted
            | EventKind::RecoveryUnavailable
            | EventKind::UndoStarted
            | EventKind::UndoItemFinished
            | EventKind::UndoFinished
            | EventKind::ForcedRestoreFinished
            | EventKind::RecoveryExpired
            | EventKind::BestEffortInverseRequested
    ) {
        return Err("invalid recovery history event kind".into());
    }
    if state.len() > 64
        || !state
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        || item_count > 16
    {
        return Err("invalid bounded recovery event".into());
    }
    let bounded_reason = reason.map(|value| {
        value
            .chars()
            .filter(|character| !character.is_control())
            .take(512)
            .collect::<String>()
    });
    let _guard = lock(data)?;
    append_locked(
        data,
        base_event(
            run,
            route,
            route,
            context_mode,
            kind,
            json!({"state":state,"reason":bounded_reason,"item_count":item_count}),
            related,
        ),
    )
}

fn coarse_from_event(event: &Event, feedback: &str) -> CoarseReceipt {
    CoarseReceipt {
        mode: event.mode.clone(),
        route: event.route.clone(),
        declared_effects: event
            .data
            .get("declared_effects")
            .and_then(Value::as_array)
            .map(|v| {
                v.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        detected_effects: event
            .data
            .get("detected_effects")
            .and_then(Value::as_array)
            .map(|v| {
                v.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        decision: event
            .data
            .get("decision")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .into(),
        execution_attempted: event
            .data
            .get("execution_attempted")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        exit_category: event
            .data
            .get("exit_category")
            .and_then(Value::as_str)
            .unwrap_or("not_attempted")
            .into(),
        signal: event
            .data
            .get("signal")
            .and_then(Value::as_i64)
            .map(|v| v as i32),
        latency_bucket: event
            .data
            .get("latency_bucket")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .into(),
        cache_state: event
            .data
            .get("cache_state")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .into(),
        user_feedback: feedback.into(),
    }
}

fn migrate_locked(data: &Path) -> Result<(), String> {
    let legacy = data.join(LEGACY);
    let target = journal_path(data);
    if target.exists() || !legacy.exists() {
        return Ok(());
    }
    let bytes = std::fs::read(&legacy).map_err(|e| format!("read legacy history: {}", e))?;
    let mut events = Vec::new();
    for (i, line) in bytes
        .split(|b| *b == b'\n')
        .filter(|v| !v.is_empty())
        .enumerate()
    {
        let receipt: Receipt = serde_json::from_slice(line)
            .map_err(|e| format!("legacy history corruption at line {}: {}", i + 1, e))?;
        let mut event = base_event(
            &receipt.run_id,
            &receipt.route,
            &receipt.mode,
            &receipt.context_mode,
            EventKind::MigratedReceipt,
            json!({"runtime":receipt.runtime,"declared_effects":receipt.declared_effects,"detected_effects":receipt.detected_effects,"decision":receipt.decision,"execution_attempted":receipt.execution_attempted,"exit_category":receipt.exit_category,"signal":receipt.signal,"latency_bucket":receipt.latency_bucket,"cache_state":receipt.cache_state,"second_turn_used":receipt.second_turn_used,"user_feedback":receipt.user_feedback}),
            None,
        );
        event.timestamp = receipt.timestamp;
        event.sequence = 1;
        event.app_version = receipt.app_version;
        event.prompt_schema_version = receipt.prompt_schema_version;
        event.checksum = checksum(&event)?;
        events.push(event);
    }
    write_private_atomic(&target, &encode_events(&events)?)?;
    let validated = read_unlocked(&target)?;
    if validated.events.len() != events.len() {
        return Err("legacy history migration validation failed".into());
    }
    std::fs::rename(
        &legacy,
        data.join(format!("history.jsonl.migrated-{}.bak", now_secs())),
    )
    .map_err(|e| format!("retain legacy history backup: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cfg(detail: HistoryDetail) -> HistoryConfig {
        HistoryConfig {
            detail,
            max_records: 20,
            ..HistoryConfig::default()
        }
    }
    fn receipt(id: &str) -> Receipt {
        Receipt {
            schema_version: 1,
            run_id: id.into(),
            timestamp: now_secs(),
            app_version: env!("CARGO_PKG_VERSION").into(),
            mode: "auto".into(),
            context_mode: "minimal".into(),
            route: "run_shell".into(),
            runtime: "none".into(),
            prompt_schema_version: 1,
            declared_effects: vec![],
            detected_effects: vec![],
            decision: "completed".into(),
            execution_attempted: true,
            exit_category: "success".into(),
            signal: None,
            latency_bucket: "lt_1s".into(),
            cache_state: "miss".into(),
            second_turn_used: false,
            user_feedback: "unknown".into(),
        }
    }
    #[test]
    fn schema_round_trip_and_checksum() {
        let d = tempfile::tempdir().unwrap();
        append_receipt(
            d.path(),
            &cfg(HistoryDetail::Metadata),
            &receipt("abcdefgh1234"),
        )
        .unwrap();
        let j = read(d.path()).unwrap();
        assert_eq!(j.events.len(), 4);
        assert!(verify(&j.events[0]).is_ok());
    }
    #[test]
    fn metadata_does_not_retain_proposal() {
        let d = tempfile::tempdir().unwrap();
        let action = ProposedAction::Answer {
            text: "secret".into(),
        };
        record_proposal(
            d.path(),
            &cfg(HistoryDetail::Metadata),
            "abcdefgh1234",
            "answer",
            "auto",
            "minimal",
            &action,
            None,
        )
        .unwrap();
        assert!(!runs_path(d.path()).exists());
        assert!(load_proposal(d.path(), "abcdefgh1234").is_err());
    }
    #[test]
    fn diagnostic_replay_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let action = ProposedAction::Answer {
            text: "hello".into(),
        };
        record_proposal(
            d.path(),
            &cfg(HistoryDetail::Diagnostic),
            "abcdefgh1234",
            "answer",
            "auto",
            "minimal",
            &action,
            None,
        )
        .unwrap();
        assert_eq!(load_proposal(d.path(), "last").unwrap().1, action);
    }
    #[test]
    fn interrupted_final_line_is_reported_but_earlier_corruption_fails() {
        let d = tempfile::tempdir().unwrap();
        append_receipt(
            d.path(),
            &cfg(HistoryDetail::Metadata),
            &receipt("abcdefgh1234"),
        )
        .unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(journal_path(d.path()))
            .unwrap()
            .write_all(b"{broken")
            .unwrap();
        assert!(read(d.path()).unwrap().truncated_final_line);
        std::fs::write(journal_path(d.path()), b"{broken}\n").unwrap();
        assert!(read(d.path()).unwrap_err().contains("line 1"));
    }
    #[test]
    fn concurrent_append_sequences_are_monotonic() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().to_path_buf();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let root = root.clone();
                std::thread::spawn(move || {
                    append_receipt(
                        &root,
                        &cfg(HistoryDetail::Metadata),
                        &receipt("abcdefgh1234"),
                    )
                    .unwrap()
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let events = events_for(&root, "abcdefgh1234").unwrap();
        for pair in events.windows(2) {
            assert!(pair[0].sequence < pair[1].sequence);
        }
    }
    #[cfg(unix)]
    #[test]
    fn permissions_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        append_receipt(
            d.path(),
            &cfg(HistoryDetail::Metadata),
            &receipt("abcdefgh1234"),
        )
        .unwrap();
        assert_eq!(
            std::fs::metadata(journal_path(d.path()))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    #[test]
    fn redacted_export_removes_ids_and_content() {
        let d = tempfile::tempdir().unwrap();
        record_request(
            d.path(),
            &cfg(HistoryDetail::Full),
            "abcdefgh1234",
            "run_shell",
            "auto",
            "minimal",
            "read /secret",
            None,
        )
        .unwrap();
        let out = d.path().join("export.jsonl");
        export(d.path(), &out, false).unwrap();
        let text = std::fs::read_to_string(out).unwrap();
        assert!(!text.contains("abcdefgh1234"));
        assert!(!text.contains("intent"));
        assert!(!text.contains("/secret"));
        assert!(!text.contains("checksum"));
        assert!(!text.contains("intent_hash"));
    }
    #[test]
    fn full_intent_is_utf8_bounded_and_truncated_seed_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let mut config = cfg(HistoryDetail::Full);
        config.artifact_max_bytes = 5;
        record_request(
            d.path(),
            &config,
            "bounded00001",
            "run_shell",
            "auto",
            "minimal",
            "éééé",
            None,
        )
        .unwrap();
        let events = events_for(d.path(), "bounded00001").unwrap();
        let intent = events[0].data["intent"].as_str().unwrap();
        assert!(intent.len() <= 5);
        assert!(events[0].data["intent_truncated"].as_bool().unwrap());
        assert!(repair_seed(d.path(), "bounded00001", None).is_err());
    }

    #[test]
    fn clear_preserves_recovery_owned_run_directories() {
        let d = tempfile::tempdir().unwrap();
        let recovery = runs_path(d.path()).join("recover00001");
        std::fs::create_dir_all(&recovery).unwrap();
        std::fs::write(recovery.join("recovery.json"), b"evidence").unwrap();
        let ordinary = runs_path(d.path()).join("ordinary001");
        std::fs::create_dir_all(&ordinary).unwrap();
        std::fs::write(ordinary.join("proposal.json"), b"content").unwrap();
        assert_eq!(clear(d.path()).unwrap(), 1);
        assert!(recovery.join("recovery.json").exists());
        assert!(!ordinary.exists());
    }
    #[test]
    fn feedback_is_immutable_event() {
        let d = tempfile::tempdir().unwrap();
        append_receipt(
            d.path(),
            &cfg(HistoryDetail::Metadata),
            &receipt("abcdefgh1234"),
        )
        .unwrap();
        set_feedback(d.path(), "good", Some("abcdefgh1234")).unwrap();
        assert_eq!(
            events_for(d.path(), "last").unwrap().last().unwrap().kind,
            EventKind::UserFeedbackReceived
        );
    }

    #[test]
    fn best_effort_seed_is_bounded_exact_and_rejects_linked_jobs() {
        let d = tempfile::tempdir().unwrap();
        let full = cfg(HistoryDetail::Full);
        let action = ProposedAction::Shell {
            command: "git reset --soft HEAD~1".into(),
            metadata: crate::action::ProposalMetadata {
                summary: "move the branch pointer".into(),
                assumptions: vec![],
                effects: vec![crate::action::Effect::WriteLocal],
                requirements: vec!["git".into()],
            },
            stdin_mode: crate::action::StdinMode::None,
        };
        record_request(
            d.path(),
            &full,
            "original-0001",
            "run",
            "run",
            "minimal",
            "undo my latest local commit",
            None,
        )
        .unwrap();
        record_proposal(
            d.path(),
            &full,
            "original-0001",
            "run",
            "run",
            "minimal",
            &action,
            None,
        )
        .unwrap();
        let (_, seed) = recovery_seed(d.path(), "original-0001", Some("keep changes")).unwrap();
        assert!(seed.contains("best_effort_inverse"));
        assert!(seed.contains("keep changes"));
        assert!(!seed.contains("snapshots"));
        record_request(
            d.path(),
            &full,
            "linked-000001",
            "recover",
            "recover",
            "minimal",
            "linked inverse",
            Some("original-0001"),
        )
        .unwrap();
        record_proposal(
            d.path(),
            &full,
            "linked-000001",
            "recover",
            "recover",
            "minimal",
            &action,
            Some("original-0001"),
        )
        .unwrap();
        assert!(recovery_seed(d.path(), "linked-000001", None).is_err());
    }

    #[test]
    fn generic_pruning_preserves_recovery_owned_run_directories() {
        let d = tempfile::tempdir().unwrap();
        let config = HistoryConfig {
            max_records: 0,
            ..cfg(HistoryDetail::Metadata)
        };
        record_request(
            d.path(),
            &config,
            "recover-00001",
            "run",
            "run",
            "minimal",
            "test",
            None,
        )
        .unwrap();
        let run = runs_path(d.path()).join("recover-00001");
        dirs::ensure_private_dir(&run).unwrap();
        write_private_atomic(&run.join("recovery.json"), b"retained").unwrap();
        prune(d.path(), &config, false).unwrap();
        assert!(run.join("recovery.json").exists());
    }
}
