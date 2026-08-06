//! Hash-verified restoration for Plan 4 managed file outputs.
//!
//! This is deliberately not a general rollback engine. Snapshot capture is a
//! separate opt-in, and only descriptor-validated sibling-staged regular files
//! can acquire a verified restore manifest.

use crate::config::RecoveryConfig;
use crate::dirs;
use serde::{Deserialize, Serialize};
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 1;
const MANIFEST: &str = "recovery.json";
const SNAPSHOTS: &str = "snapshots";
const ENABLED_MARKER: &str = "recovery-enabled-v1";
const DISABLED_MARKER: &str = "recovery-disabled-v1";
const LOCK: &str = "recovery.lock";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryState {
    None,
    Preparing,
    Available,
    CommitPartial,
    UndoPreflight,
    UndoInProgress,
    Restored,
    Conflicted,
    Expired,
    Corrupt,
}

impl RecoveryState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Preparing => "preparing",
            Self::Available => "available",
            Self::CommitPartial => "commit_partial",
            Self::UndoPreflight => "undo_preflight",
            Self::UndoInProgress => "undo_in_progress",
            Self::Restored => "restored",
            Self::Conflicted => "conflicted",
            Self::Expired => "expired",
            Self::Corrupt => "corrupt",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    Preparing,
    SnapshotReady,
    Staged,
    Committed,
    UndoPending,
    Restored,
    Removed,
    Conflicted,
    Expired,
    Corrupt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryItem {
    pub id: String,
    pub destination: PathBuf,
    pub staging: PathBuf,
    pub existed: bool,
    pub snapshot_file: Option<String>,
    pub preimage_hash: Option<String>,
    pub staged_hash: Option<String>,
    pub postimage_hash: Option<String>,
    pub preimage_bytes: u64,
    pub preimage_mode: Option<u32>,
    pub postimage_mode: Option<u32>,
    pub device: u64,
    pub inode: u64,
    pub modified_seconds: i64,
    pub modified_nanoseconds: i64,
    pub state: ItemState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryManifest {
    pub schema_version: u32,
    pub run_id: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub state: RecoveryState,
    pub pinned: bool,
    pub forced_restore: bool,
    /// Immutable ordering key allocated under the recovery lock. Legacy
    /// manifests use zero and fall back to `created_at` with tie rejection.
    #[serde(default)]
    pub selection_sequence: u64,
    /// Logical evidence deadline. Pinning suppresses enforcement but does not
    /// move this deadline. Legacy manifests derive it from `created_at`.
    #[serde(default)]
    pub expires_at: u64,
    /// Terminal cleanup is durably authorized. Management prune sets this only
    /// after `RecoveryExpired`; automatic retention needs no event, and a
    /// completed restore uses its already-durable completion event.
    #[serde(default)]
    pub retirement_acknowledged: bool,
    /// A management prune started this retirement and therefore requires one
    /// durable `RecoveryExpired` event before terminal finalization. This is
    /// persisted while pruning is partial so automatic retention cannot later
    /// downgrade the required crash ordering.
    #[serde(default)]
    pub retirement_event_required: bool,
    /// Unix deadline after which a `preparing` capture can no longer belong to
    /// a live bounded program execution. Zero on legacy manifests is expired.
    #[serde(default)]
    pub preparation_lease_until: u64,
    pub items: Vec<RecoveryItem>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryClass {
    VerifiedRestoreEligible,
    BestEffortOnly,
    Unavailable,
}

impl RecoveryClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::VerifiedRestoreEligible => "verified_restore_eligible",
            Self::BestEffortOnly => "best_effort_only",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EligibilityItem {
    pub destination: PathBuf,
    pub class: RecoveryClass,
    pub reason: String,
    pub existed: bool,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Classification {
    pub requested: bool,
    pub class: RecoveryClass,
    pub reason: String,
    pub items: Vec<EligibilityItem>,
}

impl Classification {
    pub fn all_eligible(&self) -> bool {
        self.requested
            && !self.items.is_empty()
            && self
                .items
                .iter()
                .all(|item| item.class == RecoveryClass::VerifiedRestoreEligible)
    }
}

#[derive(Debug)]
struct Identity {
    device: u64,
    inode: u64,
    len: u64,
    mode: u32,
    modified_seconds: i64,
    modified_nanoseconds: i64,
}

#[derive(Debug)]
struct PreparedItem {
    parent_path: PathBuf,
    parent: File,
    destination_name: CString,
    staging_name: CString,
    preimage: Option<Identity>,
    preimage_file: Option<File>,
}

#[derive(Debug)]
pub struct Coordinator {
    data_dir: PathBuf,
    run_dir: PathBuf,
    manifest: RecoveryManifest,
    prepared: Vec<PreparedItem>,
    // A live coordinator owns the recovery inventory until commit or Drop.
    // The wall-clock lease is only stale-process recovery; it must never let a
    // concurrent prune retire evidence that an in-process coordinator owns.
    ownership_guard: Option<File>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewItem {
    pub destination: PathBuf,
    pub operation: &'static str,
    pub snapshot_bytes: u64,
    pub conflict: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RestorePreview {
    pub run_id: String,
    pub state: String,
    pub forced: bool,
    /// Set when the `last` alias skipped a newer non-restorable manifest, so
    /// the selection is named instead of silently landing on an older run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias_note: Option<String>,
    pub items: Vec<PreviewItem>,
    pub concurrent_writer_warning: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct OperationReport {
    pub source_run_id: String,
    pub operation_run_id: String,
    pub outcome: String,
    pub restored: usize,
    pub removed: usize,
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    pub enabled: bool,
    pub state: String,
    pub reason: String,
    pub run_id: Option<String>,
    pub manifests: usize,
    pub snapshots: usize,
    pub snapshot_bytes: u64,
    pub pinned: usize,
    pub max_age_days: u64,
    pub max_total_bytes: u64,
    pub max_file_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PruneReport {
    pub dry_run: bool,
    pub manifests_scanned: usize,
    pub snapshots_removed: usize,
    pub bytes_removed: u64,
    pub retained_pinned: usize,
    /// Manifests with removable snapshots that a plain prune kept because they
    /// are inside the age and total-byte caps; `--all` removes them.
    pub retained_within_limits: usize,
    /// Terminal recovery manifests finalized during this pass.
    pub manifests_removed: usize,
    pub expired_runs: Vec<String>,
}

fn runs_dir(data: &Path) -> PathBuf {
    data.join("runs")
}

fn run_dir(data: &Path, run: &str) -> PathBuf {
    runs_dir(data).join(run)
}

fn manifest_path(data: &Path, run: &str) -> PathBuf {
    run_dir(data, run).join(MANIFEST)
}

fn validate_run_id(run: &str) -> Result<(), String> {
    if !(8..=64).contains(&run.len())
        || !run
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("invalid recovery run ID".into());
    }
    Ok(())
}

fn lock(data: &Path) -> Result<File, String> {
    if !data.is_absolute() {
        return Err("recovery data directory must be absolute".into());
    }
    dirs::ensure_private_dir(data)?;
    let path = data.join(LOCK);
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).mode(0o600);
    let file = options
        .open(path)
        .map_err(|error| format!("open recovery lock: {error}"))?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;
    file.lock()
        .map_err(|error| format!("lock recovery state: {error}"))?;
    Ok(file)
}

pub fn exclusive_guard(data: &Path) -> Result<File, String> {
    lock(data)
}

fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("private file has no parent")?;
    dirs::ensure_private_dir(parent)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".uhm-recovery-")
        .tempfile_in(parent)
        .map_err(|error| format!("create recovery temporary file: {error}"))?;
    temporary
        .as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;
    temporary
        .write_all(bytes)
        .map_err(|error| error.to_string())?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temporary
        .persist(path)
        .map_err(|error| format!("publish recovery file: {}", error.error))?;
    sync_parent(parent)
}

fn sync_parent(parent: &Path) -> Result<(), String> {
    File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("sync recovery directory {}: {error}", parent.display()))
}

fn write_manifest(data: &Path, manifest: &RecoveryManifest) -> Result<(), String> {
    validate_manifest_shape(data, manifest)?;
    write_private_atomic(
        &manifest_path(data, &manifest.run_id),
        &serde_json::to_vec_pretty(manifest).map_err(|error| error.to_string())?,
    )
}

fn read_manifest(data: &Path, run: &str) -> Result<RecoveryManifest, String> {
    validate_run_id(run)?;
    validate_private_directory(&run_dir(data, run))?;
    let path = manifest_path(data, run);
    validate_private_regular(&path, 1)?;
    let bytes = std::fs::read(&path).map_err(|error| format!("read recovery manifest: {error}"))?;
    if bytes.len() > 256 * 1024 {
        return Err("recovery manifest is oversized".into());
    }
    let manifest: RecoveryManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("recovery manifest is corrupt: {error}"))?;
    validate_manifest_shape(data, &manifest)?;
    if manifest.run_id != run {
        return Err("recovery manifest/run linkage mismatch".into());
    }
    Ok(manifest)
}

fn validate_manifest_shape(data: &Path, manifest: &RecoveryManifest) -> Result<(), String> {
    validate_run_id(&manifest.run_id)?;
    if manifest.schema_version != SCHEMA_VERSION
        || manifest.items.is_empty()
        || manifest.items.len() > 16
    {
        return Err("unsupported or invalid recovery manifest".into());
    }
    for (index, item) in manifest.items.iter().enumerate() {
        if item.id != format!("output-{index:03}") || !item.destination.is_absolute() {
            return Err("invalid recovery item linkage or destination".into());
        }
        if unsafe_components(&item.destination) || unsafe_components(&item.staging) {
            return Err("recovery paths contain traversal".into());
        }
        if let Some(name) = &item.snapshot_file {
            if name != &format!("{}.preimage", item.id) {
                return Err("invalid recovery snapshot linkage".into());
            }
            let expected = run_dir(data, &manifest.run_id).join(SNAPSHOTS).join(name);
            if !expected.starts_with(run_dir(data, &manifest.run_id)) {
                return Err("recovery snapshot escapes its run".into());
            }
        }
    }
    Ok(())
}

fn unsafe_components(path: &Path) -> bool {
    path.components()
        .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
}

pub fn effective_enabled(data: &Path, config: &RecoveryConfig) -> bool {
    let _ = config;
    if data.join(DISABLED_MARKER).exists() {
        return false;
    }
    data.join(ENABLED_MARKER).exists()
}

pub fn capture_requested(data: &Path, config: &RecoveryConfig, one_job: bool) -> bool {
    one_job
        || effective_enabled(data, config)
        || (config.enabled && !data.join(DISABLED_MARKER).exists())
}

pub fn enable(data: &Path) -> Result<(), String> {
    let _guard = lock(data)?;
    let _ = std::fs::remove_file(data.join(DISABLED_MARKER));
    write_private_atomic(&data.join(ENABLED_MARKER), b"recovery-consent-v1\n")
}

pub fn disable(data: &Path) -> Result<(), String> {
    let _guard = lock(data)?;
    let _ = std::fs::remove_file(data.join(ENABLED_MARKER));
    write_private_atomic(&data.join(DISABLED_MARKER), b"recovery-disabled-v1\n")
}

pub fn classify(
    data: &Path,
    cwd: &Path,
    outputs: &[String],
    config: &RecoveryConfig,
    history_enabled: bool,
    one_job: bool,
) -> Classification {
    let consented = effective_enabled(data, config);
    let configured_request = config.enabled && !data.join(DISABLED_MARKER).exists();
    let requested = capture_requested(data, config, one_job);
    if !requested {
        return Classification {
            requested: false,
            class: RecoveryClass::Unavailable,
            reason: "snapshot capture is off; use --recoverable once or `uhm recovery on`".into(),
            items: Vec::new(),
        };
    }
    if configured_request && !one_job && !consented {
        return Classification {
            requested: true,
            class: RecoveryClass::Unavailable,
            reason: "recovery.enabled requests capture, but `uhm recovery on` must record the separate snapshot disclosure first".into(),
            items: Vec::new(),
        };
    }
    if !history_enabled {
        return Classification {
            requested: true,
            class: RecoveryClass::Unavailable,
            reason: "metadata history is disabled, so durable snapshot linkage is unavailable"
                .into(),
            items: Vec::new(),
        };
    }
    let mut items = Vec::new();
    for output in outputs {
        let destination = absolute(cwd, output);
        let (class, reason, existed, bytes) = match inspect_eligibility(&destination, config) {
            Ok((existed, bytes)) => (
                RecoveryClass::VerifiedRestoreEligible,
                if existed {
                    "owned single-link regular file with supported metadata and filesystem"
                } else {
                    "absent path with a supported atomic no-replace sibling commit"
                }
                .into(),
                existed,
                bytes,
            ),
            Err(reason) => (RecoveryClass::Unavailable, reason, destination.exists(), 0),
        };
        items.push(EligibilityItem {
            destination,
            class,
            reason,
            existed,
            bytes,
        });
    }
    let all = !items.is_empty()
        && items
            .iter()
            .all(|item| item.class == RecoveryClass::VerifiedRestoreEligible);
    Classification {
        requested: true,
        class: if all {
            RecoveryClass::VerifiedRestoreEligible
        } else {
            RecoveryClass::Unavailable
        },
        reason: if all {
            "every declared output is eligible for a bounded verified restore".into()
        } else {
            "one or more declared outputs cannot acquire verified restore evidence".into()
        },
        items,
    }
}

fn absolute(cwd: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn inspect_eligibility(destination: &Path, config: &RecoveryConfig) -> Result<(bool, u64), String> {
    if !destination.is_absolute() || unsafe_components(destination) {
        return Err("destination is not an absolute traversal-free managed path".into());
    }
    let parent = destination.parent().ok_or("destination has no parent")?;
    let parent_file = open_parent(parent)?;
    supported_filesystem(&parent_file)?;
    let name = leaf_name(destination)?;
    match openat_read(&parent_file, &name) {
        Ok(file) => {
            let identity = validate_eligible_file(&file, config.max_file_bytes)?;
            Ok((true, identity.len))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let meta = parent_file.metadata().map_err(|error| error.to_string())?;
            if meta.uid() != unsafe { libc::geteuid() } {
                return Err(
                    "new-file recovery requires a current-user-owned parent directory".into(),
                );
            }
            if !atomic_no_replace_supported() {
                return Err("this platform lacks the tested atomic no-replace primitive".into());
            }
            Ok((false, 0))
        }
        Err(error) => Err(format!(
            "destination cannot be opened without following links: {error}"
        )),
    }
}

#[cfg(test)]
fn prepare(
    data: &Path,
    run: &str,
    config: &RecoveryConfig,
    outputs: &[(PathBuf, PathBuf)],
) -> Result<Coordinator, String> {
    prepare_with_lease(data, run, config, outputs, 60)
}

pub fn prepare_with_lease(
    data: &Path,
    run: &str,
    config: &RecoveryConfig,
    outputs: &[(PathBuf, PathBuf)],
    lease_secs: u64,
) -> Result<Coordinator, String> {
    validate_run_id(run)?;
    if outputs.is_empty() || outputs.len() > 16 {
        return Err("recovery capture requires 1..16 managed outputs".into());
    }
    // Enforce age before admitting another capture. The subsequent locked
    // usage calculation still fails closed if another process changes usage
    // between this prune and our lock acquisition.
    prune_impl(data, config, false, false, false)?;
    let ownership_guard = lock(data)?;
    let manifests = scan_manifests(data, None)?;
    // Pending expiry tombstones are intentionally not finalized here: the
    // management path must first durably record (or deduplicate) their
    // RecoveryExpired event. They no longer consume active capture capacity,
    // but their sequence remains part of the allocation high-water mark.
    let active_manifest_count = manifests
        .iter()
        .filter(|manifest| {
            manifest.state != RecoveryState::Expired && !prune_intent_started(manifest)
        })
        .count();
    if active_manifest_count >= config.scan_limit {
        return Err("recovery manifest capacity is full; run `uhm recovery prune --all` before capturing another recovery run".into());
    }
    let retained_before = retained_snapshot_bytes(&manifests);
    let selection_sequence = manifests
        .iter()
        .map(|manifest| manifest.selection_sequence)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let run_path = run_dir(data, run);
    dirs::ensure_private_dir(&run_path)?;
    let snapshots = run_path.join(SNAPSHOTS);
    dirs::ensure_private_dir(&snapshots)?;
    if manifest_path(data, run).exists() {
        return Err("a recovery manifest already exists for this run".into());
    }
    let now = crate::history::now_secs();
    let mut manifest = RecoveryManifest {
        schema_version: SCHEMA_VERSION,
        run_id: run.into(),
        created_at: now,
        updated_at: now,
        state: RecoveryState::Preparing,
        pinned: false,
        forced_restore: false,
        selection_sequence,
        expires_at: now.saturating_add(config.max_age_days.saturating_mul(86_400)),
        retirement_acknowledged: false,
        retirement_event_required: false,
        preparation_lease_until: now.saturating_add(lease_secs),
        items: Vec::new(),
        reason: None,
    };
    let mut prepared = Vec::new();
    let mut total_snapshot_bytes = retained_before;
    for (index, (destination, staging)) in outputs.iter().enumerate() {
        inspect_eligibility(destination, config)?;
        let parent_path = destination
            .parent()
            .ok_or("destination has no parent")?
            .to_path_buf();
        let parent = open_parent(&parent_path)?;
        supported_filesystem(&parent)?;
        let destination_name = leaf_name(destination)?;
        let staging_name = leaf_name(staging)?;
        if staging.parent().and_then(|path| path.canonicalize().ok())
            != Some(
                parent_path
                    .canonicalize()
                    .map_err(|error| error.to_string())?,
            )
        {
            return Err("recovery staging path is not a sibling of its destination".into());
        }
        let id = format!("output-{index:03}");
        let (existing, preimage_file) = match openat_read(&parent, &destination_name) {
            Ok(file) => {
                let identity = validate_eligible_file(&file, config.max_file_bytes)?;
                let snapshot_name = format!("{id}.preimage");
                total_snapshot_bytes = total_snapshot_bytes.saturating_add(identity.len);
                if total_snapshot_bytes > config.max_total_bytes {
                    return Err("global retained preimages would exceed recovery.max_total_bytes; prune unpinned recovery data or raise the bound".into());
                }
                manifest.items.push(RecoveryItem {
                    id,
                    destination: destination.clone(),
                    staging: staging.clone(),
                    existed: true,
                    snapshot_file: Some(snapshot_name),
                    preimage_hash: None,
                    staged_hash: None,
                    postimage_hash: None,
                    preimage_bytes: identity.len,
                    preimage_mode: Some(identity.mode),
                    postimage_mode: None,
                    device: identity.device,
                    inode: identity.inode,
                    modified_seconds: identity.modified_seconds,
                    modified_nanoseconds: identity.modified_nanoseconds,
                    state: ItemState::Preparing,
                });
                (Some(identity), Some(file))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                manifest.items.push(RecoveryItem {
                    id,
                    destination: destination.clone(),
                    staging: staging.clone(),
                    existed: false,
                    snapshot_file: None,
                    preimage_hash: None,
                    staged_hash: None,
                    postimage_hash: None,
                    preimage_bytes: 0,
                    preimage_mode: None,
                    postimage_mode: None,
                    device: 0,
                    inode: 0,
                    modified_seconds: 0,
                    modified_nanoseconds: 0,
                    state: ItemState::Preparing,
                });
                (None, None)
            }
            Err(error) => return Err(format!("snapshot destination: {error}")),
        };
        prepared.push(PreparedItem {
            parent_path,
            parent,
            destination_name,
            staging_name,
            preimage: existing,
            preimage_file,
        });
    }
    // The preparing manifest is durable before any content copy. A crash can
    // therefore be reported honestly instead of leaving anonymous snapshots.
    write_manifest(data, &manifest)?;
    for (index, item) in prepared.iter_mut().enumerate() {
        if let Some(file) = item.preimage_file.as_mut() {
            let snapshot_name = manifest.items[index]
                .snapshot_file
                .as_deref()
                .ok_or("snapshot linkage is missing")?;
            let snapshot_path = snapshots.join(snapshot_name);
            file.seek(SeekFrom::Start(0))
                .map_err(|error| error.to_string())?;
            let hash = copy_snapshot(file, &snapshot_path, config.max_file_bytes)?;
            let verified = hash_file_path(&snapshot_path, config.max_file_bytes)?;
            if hash != verified {
                return Err("snapshot hash verification failed".into());
            }
            manifest.items[index].preimage_hash = Some(hash);
        }
        manifest.items[index].state = ItemState::SnapshotReady;
        manifest.updated_at = crate::history::now_secs();
        write_manifest(data, &manifest)?;
    }
    // Snapshot capture can itself consume a material part of the caller's
    // execution budget. Start the stale-process lease only after every
    // preimage is durable and verified; the live lock above remains the
    // authoritative ownership signal until this Coordinator is dropped.
    let ready_at = crate::history::now_secs();
    manifest.preparation_lease_until = ready_at.saturating_add(lease_secs);
    manifest.updated_at = ready_at;
    write_manifest(data, &manifest)?;
    Ok(Coordinator {
        data_dir: data.into(),
        run_dir: run_path,
        manifest,
        prepared,
        ownership_guard: Some(ownership_guard),
    })
}

fn scan_manifests(
    data: &Path,
    manifest_limit: Option<usize>,
) -> Result<Vec<RecoveryManifest>, String> {
    let entries = match std::fs::read_dir(runs_dir(data)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("scan recovery manifests: {error}")),
    };
    let mut manifests = Vec::new();
    let mut active_manifests = 0usize;
    for entry in entries {
        let entry = entry.map_err(|error| format!("scan recovery manifests: {error}"))?;
        let entry_path = entry.path();
        let path = entry_path.join(MANIFEST);
        match path.symlink_metadata() {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "inspect recovery manifest {}: {error}",
                    path.display()
                ))
            }
        }
        let entry_type = entry
            .file_type()
            .map_err(|error| format!("inspect recovery run directory: {error}"))?;
        if !entry_type.is_dir() || entry_type.is_symlink() {
            return Err("recovery manifest belongs to an unsafe run directory".into());
        }
        let Some(run) = entry.file_name().to_str().map(str::to_owned) else {
            return Err("cannot prove recovery state: non-UTF-8 recovery run directory".into());
        };
        let manifest = read_manifest(data, &run)?;
        if manifest.state != RecoveryState::Expired && !prune_intent_started(&manifest) {
            if manifest_limit.is_some_and(|limit| active_manifests >= limit) {
                return Err("recovery manifest count exceeds recovery.scan_limit; selection is unavailable until `uhm recovery prune --all` makes the inventory complete".into());
            }
            active_manifests += 1;
        }
        manifests.push(manifest);
    }
    Ok(manifests)
}

fn retained_snapshot_bytes(manifests: &[RecoveryManifest]) -> u64 {
    manifests
        .iter()
        .flat_map(|manifest| &manifest.items)
        .filter(|item| item.snapshot_file.is_some() && item.state != ItemState::Expired)
        .map(|item| item.preimage_bytes)
        .sum()
}

fn expiry_deadline(manifest: &RecoveryManifest, config: &RecoveryConfig) -> u64 {
    let configured = manifest
        .created_at
        .saturating_add(config.max_age_days.saturating_mul(86_400));
    if manifest.expires_at == 0 {
        configured
    } else {
        manifest.expires_at.min(configured)
    }
}

fn logically_expired(manifest: &RecoveryManifest, config: &RecoveryConfig, now: u64) -> bool {
    !manifest.pinned
        && matches!(
            manifest.state,
            RecoveryState::Available | RecoveryState::Conflicted
        )
        && now >= expiry_deadline(manifest, config)
}

impl Coordinator {
    pub fn state(&self) -> &'static str {
        self.manifest.state.as_str()
    }

    pub fn commit(&mut self, max_total: u64) -> Result<Vec<PathBuf>, String> {
        // Move ownership into this call so every success and error path releases
        // the cross-process lock only after commit has finished.
        let _ownership_guard = self
            .ownership_guard
            .take()
            .ok_or("recovery coordinator no longer owns its preparing capture")?;
        self.revalidate_durable_capture()?;
        let mut total = 0u64;
        for (index, prepared) in self.prepared.iter().enumerate() {
            revalidate_parent(prepared)?;
            let staged = openat_read(&prepared.parent, &prepared.staging_name)
                .map_err(|error| format!("declared staged artifact was not produced: {error}"))?;
            let identity = validate_staged_file(&staged)?;
            total = total.saturating_add(identity.len);
            if total > max_total {
                self.fail(
                    RecoveryState::Corrupt,
                    "staged artifacts exceed the workspace byte limit",
                )?;
                return Err("staged artifacts exceed the workspace byte limit".into());
            }
            let hash = hash_file(staged, max_total)?;
            self.manifest.items[index].staged_hash = Some(hash);
            self.manifest.items[index].postimage_mode = Some(identity.mode);
            self.manifest.items[index].state = ItemState::Staged;
        }
        for (index, prepared) in self.prepared.iter().enumerate() {
            if let Err(error) = precommit_matches(prepared, &self.manifest.items[index]) {
                self.fail(RecoveryState::Conflicted, &error)?;
                return Err(error);
            }
        }
        transition(&mut self.manifest, RecoveryState::CommitPartial)?;
        write_manifest(&self.data_dir, &self.manifest)?;
        let mut committed = Vec::new();
        for index in 0..self.prepared.len() {
            let prepared = &self.prepared[index];
            let item = &self.manifest.items[index];
            let result = if item.existed {
                rename_replace(
                    &prepared.parent,
                    &prepared.staging_name,
                    &prepared.destination_name,
                )
            } else {
                rename_no_replace(
                    &prepared.parent,
                    &prepared.staging_name,
                    &prepared.destination_name,
                )
            };
            if let Err(error) = result {
                self.manifest.reason = Some(format!("commit item {index}: {error}"));
                write_manifest(&self.data_dir, &self.manifest)?;
                return Err(format!(
                    "commit managed output {}: {error}",
                    item.destination.display()
                ));
            }
            let current = openat_read(&prepared.parent, &prepared.destination_name)
                .map_err(|error| format!("reopen committed output: {error}"))?;
            let identity = validate_staged_file(&current)?;
            let observed = hash_file(current, max_total)?;
            if Some(&observed) != self.manifest.items[index].staged_hash.as_ref() {
                self.manifest.items[index].state = ItemState::Conflicted;
                self.fail(
                    RecoveryState::Conflicted,
                    "committed postimage hash mismatch",
                )?;
                return Err("committed postimage hash mismatch".into());
            }
            self.manifest.items[index].postimage_hash = Some(observed);
            self.manifest.items[index].postimage_mode = Some(identity.mode);
            self.manifest.items[index].state = ItemState::Committed;
            self.manifest.updated_at = crate::history::now_secs();
            sync_directory_handle(&prepared.parent)?;
            write_manifest(&self.data_dir, &self.manifest)?;
            committed.push(self.manifest.items[index].destination.clone());
        }
        transition(&mut self.manifest, RecoveryState::Available)?;
        self.manifest.reason = None;
        write_manifest(&self.data_dir, &self.manifest)?;
        let _ = sync_parent(&self.run_dir);
        Ok(committed)
    }

    fn revalidate_durable_capture(&self) -> Result<(), String> {
        let durable = read_manifest(&self.data_dir, &self.manifest.run_id)?;
        if durable != self.manifest || durable.state != RecoveryState::Preparing {
            return Err(
                "durable recovery capture changed while its coordinator was live; commit refused"
                    .into(),
            );
        }
        for item in &durable.items {
            if item.state != ItemState::SnapshotReady {
                return Err(
                    "durable recovery capture is not fully prepared; commit refused".into(),
                );
            }
            if item.existed {
                snapshot_path(&self.data_dir, &durable, item).map_err(|error| {
                    format!("durable recovery preimage evidence is unavailable: {error}")
                })?;
            } else if item.snapshot_file.is_some() || item.preimage_hash.is_some() {
                return Err("new-file recovery item has unexpected preimage evidence".into());
            }
        }
        Ok(())
    }

    fn fail(&mut self, state: RecoveryState, reason: &str) -> Result<(), String> {
        transition(&mut self.manifest, state)?;
        self.manifest.reason = Some(reason.into());
        self.manifest.updated_at = crate::history::now_secs();
        write_manifest(&self.data_dir, &self.manifest)
    }
}

pub fn cleanup_incomplete_capture(data: &Path, run: &str) {
    if validate_run_id(run).is_err() {
        return;
    }
    let Ok(_guard) = lock(data) else {
        return;
    };
    let directory = run_dir(data, run);
    if directory.join(MANIFEST).exists()
        && !read_manifest(data, run)
            .is_ok_and(|manifest| manifest.state == RecoveryState::Preparing)
    {
        return;
    }
    let snapshots = directory.join(SNAPSHOTS);
    if snapshots
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    {
        let _ = std::fs::remove_dir_all(snapshots);
    }
    let _ = std::fs::remove_file(directory.join(MANIFEST));
}

fn manifest_order(manifest: &RecoveryManifest) -> (u8, u64) {
    if manifest.selection_sequence == 0 {
        (0, manifest.created_at)
    } else {
        (1, manifest.selection_sequence)
    }
}

fn newest_manifest(
    mut manifests: Vec<RecoveryManifest>,
) -> Result<Option<RecoveryManifest>, String> {
    manifests.sort_by_key(manifest_order);
    let newest = manifests.pop();
    if let (Some(candidate), Some(previous)) = (&newest, manifests.last()) {
        if manifest_order(candidate) == manifest_order(previous) {
            return Err(format!(
                "recovery ordering is ambiguous between runs {} and {}; specify a run ID",
                previous.run_id, candidate.run_id
            ));
        }
    }
    Ok(newest)
}

fn resolve_manifest_run(
    data: &Path,
    selected: &str,
    config: &RecoveryConfig,
) -> Result<String, String> {
    if selected != "last" {
        validate_run_id(selected)?;
        return Ok(selected.into());
    }
    newest_manifest(scan_manifests(data, Some(config.scan_limit))?)?
        .map(|manifest| manifest.run_id)
        .ok_or_else(|| "no retained recovery manifest is available".into())
}

/// The manifest states a verified undo or forced restore can act on.
fn restorable_state(state: RecoveryState) -> bool {
    matches!(
        state,
        RecoveryState::Available
            | RecoveryState::Conflicted
            | RecoveryState::UndoPreflight
            | RecoveryState::UndoInProgress
    )
}

/// An expired item under a non-terminal manifest is durable prune intent. It
/// is written before unlinking the corresponding snapshot, so the whole
/// manifest must stop participating in restore selection immediately.
fn prune_intent_started(manifest: &RecoveryManifest) -> bool {
    manifest.state != RecoveryState::Expired
        && manifest
            .items
            .iter()
            .any(|item| item.state == ItemState::Expired)
}

fn restorable_manifest(manifest: &RecoveryManifest) -> bool {
    restorable_state(manifest.state) && !prune_intent_started(manifest)
}

fn selection_state(manifest: &RecoveryManifest) -> &'static str {
    if prune_intent_started(manifest) {
        "expiring"
    } else {
        manifest.state.as_str()
    }
}

/// Resolves a run selection for undo and restore. The `last` alias picks the
/// most recent manifest in a restorable state, so a newer restored or corrupt
/// manifest never shadows the run the user can act on; when one was skipped,
/// the returned note names the choice.
fn resolve_restorable_run(
    data: &Path,
    selected: &str,
    config: &RecoveryConfig,
) -> Result<(String, Option<String>), String> {
    if selected != "last" {
        validate_run_id(selected)?;
        return Ok((selected.into(), None));
    }
    let now = crate::history::now_secs();
    let manifests = scan_manifests(data, Some(config.scan_limit))?;
    let newest_any = newest_manifest(manifests.clone())?;
    let newest_restorable = newest_manifest(
        manifests
            .into_iter()
            .filter(|manifest| {
                restorable_manifest(manifest) && !logically_expired(manifest, config, now)
            })
            .collect(),
    )?;
    match (newest_restorable, newest_any) {
        (Some(run), Some(newest)) if newest.run_id != run.run_id => {
            let note = format!(
                "selected run {}, the most recent restorable manifest; skipped newer run {} because its state is {}",
                run.run_id,
                newest.run_id,
                if logically_expired(&newest, config, now) {
                    "expired"
                } else {
                    selection_state(&newest)
                }
            );
            Ok((run.run_id, Some(note)))
        }
        (Some(run), _) => Ok((run.run_id, None)),
        (None, Some(newest)) => Err(format!(
            "no restorable recovery manifest is available; the most recent manifest {} is {}",
            newest.run_id,
            if logically_expired(&newest, config, now) {
                "expired"
            } else {
                selection_state(&newest)
            }
        )),
        (None, None) => Err("no retained recovery manifest is available".into()),
    }
}

fn snapshot_path(
    data: &Path,
    manifest: &RecoveryManifest,
    item: &RecoveryItem,
) -> Result<PathBuf, String> {
    let path = linked_snapshot_path(data, manifest, item)?;
    validate_private_directory(&run_dir(data, &manifest.run_id).join(SNAPSHOTS))?;
    validate_private_regular(&path, 1)?;
    let observed = hash_file_path(&path, item.preimage_bytes)?;
    if item.preimage_hash.as_deref() != Some(&observed) {
        return Err("retained snapshot hash does not match its manifest".into());
    }
    Ok(path)
}

fn linked_snapshot_path(
    data: &Path,
    manifest: &RecoveryManifest,
    item: &RecoveryItem,
) -> Result<PathBuf, String> {
    let name = item
        .snapshot_file
        .as_deref()
        .ok_or("replacement recovery item has no retained snapshot")?;
    let path = run_dir(data, &manifest.run_id).join(SNAPSHOTS).join(name);
    let expected = run_dir(data, &manifest.run_id)
        .join(SNAPSHOTS)
        .join(format!("{}.preimage", item.id));
    if path != expected {
        return Err("snapshot path does not match its manifest item".into());
    }
    Ok(path)
}

fn owned_snapshot_exists(path: &Path) -> Result<bool, String> {
    match path.symlink_metadata() {
        Ok(_) => {
            validate_private_regular(path, 1)?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("inspect recovery snapshot for pruning: {error}")),
    }
}

fn current_hash(item: &RecoveryItem, max: u64) -> Result<Option<String>, String> {
    let parent_path = item
        .destination
        .parent()
        .ok_or("destination has no parent")?;
    let parent = open_parent(parent_path)?;
    supported_filesystem(&parent)?;
    let name = leaf_name(&item.destination)?;
    match openat_read(&parent, &name) {
        Ok(file) => {
            let identity = validate_staged_file(&file)?;
            if identity.len > max {
                return Err("current destination exceeds the bounded recovery hash limit".into());
            }
            Ok(Some(hash_file(file, max)?))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("inspect current recovery destination: {error}")),
    }
}

fn current_mode(item: &RecoveryItem, max: u64) -> Result<Option<u32>, String> {
    let parent_path = item
        .destination
        .parent()
        .ok_or("destination has no parent")?;
    let parent = open_parent(parent_path)?;
    let name = leaf_name(&item.destination)?;
    match openat_read(&parent, &name) {
        Ok(file) => {
            let identity = validate_staged_file(&file)?;
            if identity.len > max {
                return Err("current destination exceeds the bounded recovery limit".into());
            }
            Ok(Some(identity.mode))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("inspect current recovery mode: {error}")),
    }
}

fn item_conflict(
    data: &Path,
    manifest: &RecoveryManifest,
    item: &RecoveryItem,
    max: u64,
) -> Option<String> {
    if item.existed {
        if let Err(error) = snapshot_path(data, manifest, item) {
            return Some(error);
        }
    }
    let expected = item.postimage_hash.as_deref();
    if expected.is_none() {
        return Some("managed commit lacks a verified postimage hash".into());
    }
    match current_hash(item, max) {
        Ok(Some(hash)) if Some(hash.as_str()) == expected => match current_mode(item, max) {
            Ok(Some(mode)) if Some(mode) == item.postimage_mode => None,
            Ok(Some(mode)) => Some(format!(
                "current mode {mode:o} does not match the recorded postimage mode"
            )),
            Ok(None) => Some("managed destination disappeared while checking its mode".into()),
            Err(error) => Some(error),
        },
        Ok(Some(hash)) => Some(format!(
            "current hash {} does not match recorded postimage",
            &hash[..12]
        )),
        Ok(None) => Some("managed destination is missing".into()),
        Err(error) => Some(error),
    }
}

pub fn preview_restore(
    data: &Path,
    selected: &str,
    config: &RecoveryConfig,
    forced: bool,
) -> Result<RestorePreview, String> {
    let (run, alias_note) = resolve_restorable_run(data, selected, config)?;
    let manifest = read_manifest(data, &run)?;
    if logically_expired(&manifest, config, crate::history::now_secs()) {
        return Err(format!(
            "recovery manifest is expired as of Unix time {}",
            expiry_deadline(&manifest, config)
        ));
    }
    if !restorable_manifest(&manifest) {
        return Err(format!(
            "recovery manifest is {}, not restorable",
            selection_state(&manifest)
        ));
    }
    let items = manifest
        .items
        .iter()
        .map(|item| PreviewItem {
            destination: item.destination.clone(),
            operation: if item.existed {
                "replace_with_preimage"
            } else {
                "remove_created_file"
            },
            snapshot_bytes: item.preimage_bytes,
            conflict: item_conflict(data, &manifest, item, config.max_file_bytes),
        })
        .collect();
    Ok(RestorePreview {
        run_id: run,
        state: manifest.state.as_str().into(),
        forced,
        alias_note,
        items,
        concurrent_writer_warning: "Each rename is atomic, but the collection is not a transaction; another writer can race the final hash check and rename.",
    })
}

fn create_restore_temporary(
    parent: &File,
    item: &RecoveryItem,
    snapshot: &Path,
) -> Result<CString, String> {
    let name = CString::new(format!(".uhm-restore-{}-{}", item.id, std::process::id()))
        .map_err(|_| "invalid restore temporary name")?;
    let descriptor = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC,
            0o600,
        )
    };
    if descriptor < 0 {
        return Err(format!(
            "create sibling restore file: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut output = unsafe { File::from_raw_fd(descriptor) };
    let result = (|| {
        let mut input = File::open(snapshot).map_err(|error| error.to_string())?;
        std::io::copy(&mut input, &mut output).map_err(|error| error.to_string())?;
        output
            .set_permissions(std::fs::Permissions::from_mode(
                item.preimage_mode.ok_or("snapshot mode is missing")?,
            ))
            .map_err(|error| error.to_string())?;
        output.sync_all().map_err(|error| error.to_string())
    })();
    if let Err(error) = result {
        drop(output);
        let _ = unlinkat(parent, &name);
        return Err(error);
    }
    Ok(name)
}

fn unlinkat(parent: &File, name: &CString) -> Result<(), String> {
    if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

pub fn restore(
    data: &Path,
    selected: &str,
    operation_run_id: &str,
    config: &RecoveryConfig,
    forced: bool,
) -> Result<OperationReport, String> {
    validate_run_id(operation_run_id)?;
    let _guard = lock(data)?;
    let (source, _alias_note) = resolve_restorable_run(data, selected, config)?;
    let mut manifest = read_manifest(data, &source)?;
    if logically_expired(&manifest, config, crate::history::now_secs()) {
        return Err(format!(
            "recovery manifest is expired as of Unix time {}",
            expiry_deadline(&manifest, config)
        ));
    }
    // Force is irreversible provenance: a resumed ordinary `undo` must not
    // downgrade an operation that previously crossed the force boundary.
    let forced = forced || manifest.forced_restore;
    if !restorable_manifest(&manifest) {
        return Err(format!(
            "recovery manifest is {}, not restorable",
            selection_state(&manifest)
        ));
    }
    if forced && !manifest.forced_restore {
        manifest.forced_restore = true;
        write_manifest(data, &manifest)?;
    }

    let mut conflicts = Vec::new();
    for item in &manifest.items {
        if matches!(item.state, ItemState::Restored | ItemState::Removed) {
            let complete = if item.state == ItemState::Restored {
                current_hash(item, config.max_file_bytes)?.as_deref()
                    == item.preimage_hash.as_deref()
            } else {
                current_hash(item, config.max_file_bytes)?.is_none()
            };
            if !complete {
                return Err(format!(
                    "previously completed restore item {} no longer matches its recorded final state",
                    item.destination.display()
                ));
            }
            continue;
        }
        if forced {
            current_hash(item, config.max_file_bytes).map_err(|error| {
                format!(
                    "forced restore still rejects unsupported destination types at {}: {error}",
                    item.destination.display()
                )
            })?;
        }
        if let Some(conflict) = item_conflict(data, &manifest, item, config.max_file_bytes) {
            conflicts.push(format!("{}: {conflict}", item.destination.display()));
        }
    }
    if !conflicts.is_empty() && !forced {
        if manifest.state == RecoveryState::Available {
            transition(&mut manifest, RecoveryState::UndoPreflight)?;
        }
        if manifest.state != RecoveryState::Conflicted {
            transition(&mut manifest, RecoveryState::Conflicted)?;
        }
        manifest.reason = Some(conflicts.join("; "));
        write_manifest(data, &manifest)?;
        return Err(format!(
            "verified undo refused: {}. Inspect the file or use `uhm restore {} --force`.",
            conflicts.join("; "),
            source
        ));
    }
    if matches!(
        manifest.state,
        RecoveryState::Available | RecoveryState::Conflicted
    ) {
        transition(&mut manifest, RecoveryState::UndoPreflight)?;
        manifest.forced_restore = forced;
        manifest.reason = None;
        for item in &mut manifest.items {
            if !matches!(item.state, ItemState::Restored | ItemState::Removed) {
                item.state = ItemState::UndoPending;
            }
        }
        write_manifest(data, &manifest)?;
        transition(&mut manifest, RecoveryState::UndoInProgress)?;
        write_manifest(data, &manifest)?;
    } else if manifest.state == RecoveryState::UndoPreflight {
        transition(&mut manifest, RecoveryState::UndoInProgress)?;
        write_manifest(data, &manifest)?;
    }

    let mut restored = 0usize;
    let mut removed = 0usize;
    for index in 0..manifest.items.len() {
        if matches!(
            manifest.items[index].state,
            ItemState::Restored | ItemState::Removed
        ) {
            continue;
        }
        let item = manifest.items[index].clone();
        let parent = open_parent(
            item.destination
                .parent()
                .ok_or("destination has no parent")?,
        )?;
        supported_filesystem(&parent)?;
        let destination = leaf_name(&item.destination)?;
        if item.existed {
            let snapshot = snapshot_path(data, &manifest, &item)?;
            let temporary = create_restore_temporary(&parent, &item, &snapshot)?;
            if !forced {
                let observed = current_hash(&item, config.max_file_bytes)?;
                if observed.as_deref() != item.postimage_hash.as_deref() {
                    let _ = unlinkat(&parent, &temporary);
                    manifest.items[index].state = ItemState::Conflicted;
                    transition(&mut manifest, RecoveryState::Conflicted)?;
                    manifest.reason = Some(format!(
                        "{} changed during restore",
                        item.destination.display()
                    ));
                    write_manifest(data, &manifest)?;
                    return Err(manifest.reason.clone().unwrap());
                }
            }
            if let Err(error) = rename_replace(&parent, &temporary, &destination) {
                let _ = unlinkat(&parent, &temporary);
                return Err(error);
            }
            sync_directory_handle(&parent)?;
            let final_hash = current_hash(&item, config.max_file_bytes)?;
            let final_mode = current_mode(&item, config.max_file_bytes)?;
            if final_hash.as_deref() != item.preimage_hash.as_deref()
                || final_mode != item.preimage_mode
            {
                return Err("restored preimage failed its completion hash or mode check".into());
            }
            manifest.items[index].state = ItemState::Restored;
            restored += 1;
        } else {
            match openat_read(&parent, &destination) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    manifest.items[index].state = ItemState::Removed;
                    removed += 1;
                }
                Err(error) => return Err(format!("open created output for removal: {error}")),
                Ok(_) => {
                    let quarantine = CString::new(format!(
                        ".uhm-quarantine-{}-{}",
                        item.id,
                        std::process::id()
                    ))
                    .map_err(|_| "invalid quarantine name")?;
                    rename_no_replace(&parent, &destination, &quarantine)?;
                    let quarantined = openat_read(&parent, &quarantine)
                        .map_err(|error| format!("open quarantined output: {error}"))?;
                    let hash = hash_file(quarantined, config.max_file_bytes)?;
                    if !forced && Some(hash.as_str()) != item.postimage_hash.as_deref() {
                        let restore_result = rename_no_replace(&parent, &quarantine, &destination);
                        manifest.items[index].state = ItemState::Conflicted;
                        transition(&mut manifest, RecoveryState::Conflicted)?;
                        manifest.reason = Some(format!(
                            "{} changed while quarantined{}",
                            item.destination.display(),
                            restore_result
                                .err()
                                .map(|e| format!(
                                    "; quarantine retained because restore failed: {e}"
                                ))
                                .unwrap_or_default()
                        ));
                        write_manifest(data, &manifest)?;
                        return Err(manifest.reason.clone().unwrap());
                    }
                    unlinkat(&parent, &quarantine)?;
                    sync_directory_handle(&parent)?;
                    if openat_read(&parent, &destination).is_ok() {
                        return Err("created output still exists after recovery removal".into());
                    }
                    manifest.items[index].state = ItemState::Removed;
                    removed += 1;
                }
            }
        }
        manifest.updated_at = crate::history::now_secs();
        write_manifest(data, &manifest)?;
    }
    transition(&mut manifest, RecoveryState::Restored)?;
    manifest.reason = if forced {
        Some(
            "retained snapshots were applied under explicit force; this is not verified undo"
                .into(),
        )
    } else {
        None
    };
    write_manifest(data, &manifest)?;
    Ok(OperationReport {
        source_run_id: source,
        operation_run_id: operation_run_id.into(),
        outcome: if forced {
            "forced_restore"
        } else {
            "verified_restore"
        }
        .into(),
        restored,
        removed,
        conflicts,
    })
}

pub fn status(
    data: &Path,
    selected: Option<&str>,
    config: &RecoveryConfig,
) -> Result<StatusReport, String> {
    let mut report = StatusReport {
        enabled: effective_enabled(data, config),
        state: if effective_enabled(data, config) {
            "enabled"
        } else {
            "disabled"
        }
        .into(),
        reason: if effective_enabled(data, config) {
            "new eligible managed outputs may be snapshotted"
        } else {
            "new recovery snapshots are not captured"
        }
        .into(),
        run_id: None,
        manifests: 0,
        snapshots: 0,
        snapshot_bytes: 0,
        pinned: 0,
        max_age_days: config.max_age_days,
        max_total_bytes: config.max_total_bytes,
        max_file_bytes: config.max_file_bytes,
    };
    let selected_run = selected
        .map(|value| resolve_manifest_run(data, value, config))
        .transpose()?;
    let manifests = scan_manifests(data, Some(config.scan_limit))?;
    let now = crate::history::now_secs();
    for manifest in manifests {
        report.manifests += 1;
        if manifest.pinned {
            report.pinned += 1;
        }
        for item in &manifest.items {
            if item.snapshot_file.is_some() && !matches!(item.state, ItemState::Expired) {
                report.snapshots += 1;
                report.snapshot_bytes = report.snapshot_bytes.saturating_add(item.preimage_bytes);
            }
        }
        if selected_run.as_deref() == Some(manifest.run_id.as_str()) {
            report.run_id = Some(manifest.run_id.clone());
            if logically_expired(&manifest, config, now) {
                report.state = "expired".into();
                report.reason = format!(
                    "recovery evidence expired at Unix time {}",
                    expiry_deadline(&manifest, config)
                );
            } else {
                report.state = selection_state(&manifest).into();
                report.reason = manifest
                    .reason
                    .unwrap_or_else(|| "recovery manifest validated".into());
            }
        }
    }
    Ok(report)
}

pub fn startup_check(data: &Path, config: &RecoveryConfig) -> usize {
    let Ok(manifests) = scan_manifests(data, Some(config.scan_limit.min(32))) else {
        return 0;
    };
    manifests
        .into_iter()
        .filter(|manifest| {
            matches!(
                manifest.state,
                RecoveryState::Preparing
                    | RecoveryState::CommitPartial
                    | RecoveryState::UndoPreflight
                    | RecoveryState::UndoInProgress
            )
        })
        .count()
}

pub fn pin(
    data: &Path,
    selected: &str,
    config: &RecoveryConfig,
    value: bool,
) -> Result<String, String> {
    let _guard = lock(data)?;
    let run = resolve_manifest_run(data, selected, config)?;
    let mut manifest = read_manifest(data, &run)?;
    if value {
        if prune_intent_started(&manifest)
            || !matches!(
                manifest.state,
                RecoveryState::Available | RecoveryState::Conflicted
            )
        {
            return Err(format!(
                "cannot pin recovery evidence in state {}",
                selection_state(&manifest)
            ));
        }
        if logically_expired(&manifest, config, crate::history::now_secs()) {
            return Err("cannot pin recovery evidence after its expiry deadline".into());
        }
        let usage = status(data, None, config)?.snapshot_bytes;
        if usage > config.max_total_bytes {
            return Err(
                "cannot pin while retained snapshots exceed the configured total-byte cap".into(),
            );
        }
    }
    manifest.pinned = value;
    manifest.updated_at = crate::history::now_secs();
    write_manifest(data, &manifest)?;
    Ok(run)
}

pub fn resume_commit(
    data: &Path,
    selected: &str,
    config: &RecoveryConfig,
) -> Result<String, String> {
    let _guard = lock(data)?;
    let run = resolve_manifest_run(data, selected, config)?;
    let mut manifest = read_manifest(data, &run)?;
    if manifest.state != RecoveryState::CommitPartial {
        return Err(format!(
            "only commit_partial recovery can resume; this run is {}",
            manifest.state.as_str()
        ));
    }
    // `Staged` is the durable rename intent. If the staging name disappeared
    // but the destination already matches that intent, the crash happened
    // after rename and before item-state persistence; reconcile instead of
    // trying (and failing) to rename a second time.
    for index in 0..manifest.items.len() {
        if manifest.items[index].state == ItemState::Committed {
            continue;
        }
        let item = manifest.items[index].clone();
        let parent = open_parent(
            item.destination
                .parent()
                .ok_or("destination has no parent")?,
        )?;
        let staging_name = leaf_name(&item.staging)?;
        match openat_read(&parent, &staging_name) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let observed = current_hash(&item, config.max_file_bytes)?;
                let mode = current_mode(&item, config.max_file_bytes)?;
                if observed.as_deref() == item.staged_hash.as_deref() && mode == item.postimage_mode
                {
                    sync_directory_handle(&parent)?;
                    manifest.items[index].postimage_hash = observed;
                    manifest.items[index].state = ItemState::Committed;
                    write_manifest(data, &manifest)?;
                } else {
                    return Err(format!(
                        "staging for {} is missing and destination matches neither durable staged evidence nor a resumable preimage",
                        item.destination.display()
                    ));
                }
            }
            Err(error) => return Err(format!("inspect resume staging file: {error}")),
        }
    }
    let mut prepared = Vec::new();
    for item in &manifest.items {
        let parent_path = item
            .destination
            .parent()
            .ok_or("destination has no parent")?
            .to_path_buf();
        let parent = open_parent(&parent_path)?;
        supported_filesystem(&parent)?;
        let destination_name = leaf_name(&item.destination)?;
        let staging_name = leaf_name(&item.staging)?;
        let preimage = item.existed.then_some(Identity {
            device: item.device,
            inode: item.inode,
            len: item.preimage_bytes,
            mode: item.preimage_mode.ok_or("preimage mode is missing")?,
            modified_seconds: item.modified_seconds,
            modified_nanoseconds: item.modified_nanoseconds,
        });
        let value = PreparedItem {
            parent_path,
            parent,
            destination_name,
            staging_name,
            preimage,
            preimage_file: None,
        };
        if item.state == ItemState::Committed {
            let observed = current_hash(item, config.max_file_bytes)?;
            let mode = current_mode(item, config.max_file_bytes)?;
            if observed.as_deref() != item.postimage_hash.as_deref() || mode != item.postimage_mode
            {
                return Err(format!(
                    "already committed output {} changed; resume refused",
                    item.destination.display()
                ));
            }
        } else {
            precommit_matches(&value, item)?;
            let staged = openat_read(&value.parent, &value.staging_name)
                .map_err(|error| format!("resume staging file is unavailable: {error}"))?;
            let identity = validate_staged_file(&staged)?;
            if identity.len > config.max_total_bytes {
                return Err("resume staging file exceeds the configured recovery bound".into());
            }
            let hash = hash_file(staged, config.max_total_bytes)?;
            if Some(hash.as_str()) != item.staged_hash.as_deref() {
                return Err("resume staging hash differs from the preflighted staged hash".into());
            }
        }
        prepared.push(value);
    }
    for (index, prepared) in prepared.iter().enumerate() {
        if manifest.items[index].state == ItemState::Committed {
            continue;
        }
        let item = &manifest.items[index];
        if item.existed {
            rename_replace(
                &prepared.parent,
                &prepared.staging_name,
                &prepared.destination_name,
            )?;
        } else {
            rename_no_replace(
                &prepared.parent,
                &prepared.staging_name,
                &prepared.destination_name,
            )?;
        }
        let current = openat_read(&prepared.parent, &prepared.destination_name)
            .map_err(|error| format!("reopen resumed output: {error}"))?;
        let observed = hash_file(current, config.max_total_bytes)?;
        if Some(observed.as_str()) != manifest.items[index].staged_hash.as_deref() {
            manifest.items[index].state = ItemState::Conflicted;
            transition(&mut manifest, RecoveryState::Conflicted)?;
            manifest.reason = Some("resumed postimage hash mismatch".into());
            write_manifest(data, &manifest)?;
            return Err("resumed postimage hash mismatch".into());
        }
        manifest.items[index].postimage_hash = Some(observed);
        manifest.items[index].state = ItemState::Committed;
        sync_directory_handle(&prepared.parent)?;
        write_manifest(data, &manifest)?;
    }
    transition(&mut manifest, RecoveryState::Available)?;
    manifest.reason = None;
    write_manifest(data, &manifest)?;
    Ok(run)
}

fn finalize_expired_locked(data: &Path, manifest: &RecoveryManifest) -> Result<(), String> {
    if manifest.state != RecoveryState::Expired {
        return Err("only an expired recovery manifest can be finalized".into());
    }
    if !manifest.retirement_acknowledged {
        return Err("expired recovery manifest is still awaiting its durable history event".into());
    }
    remove_expired_snapshots_locked(data, manifest)?;
    remove_expired_manifest_locked(data, manifest)
}

fn remove_expired_snapshots_locked(data: &Path, manifest: &RecoveryManifest) -> Result<(), String> {
    let run = run_dir(data, &manifest.run_id);
    let snapshots = run_dir(data, &manifest.run_id).join(SNAPSHOTS);
    match snapshots.symlink_metadata() {
        Ok(metadata) => {
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err("refusing to finalize an unsafe recovery snapshots directory".into());
            }
            for item in &manifest.items {
                let Some(name) = item.snapshot_file.as_deref() else {
                    continue;
                };
                let path = snapshots.join(name);
                match validate_private_regular(&path, 1) {
                    Ok(()) => std::fs::remove_file(&path)
                        .map_err(|error| format!("remove expired recovery snapshot: {error}"))?,
                    Err(_) if !path.exists() => {}
                    Err(error) => return Err(error),
                }
            }
            if std::fs::read_dir(&snapshots)
                .map_err(|error| error.to_string())?
                .next()
                .is_some()
            {
                return Err(
                    "recovery snapshots directory contains unowned files after expiry".into(),
                );
            }
            std::fs::remove_dir(&snapshots)
                .map_err(|error| format!("remove expired snapshots directory: {error}"))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("inspect expired snapshots directory: {error}")),
    }
    // The snapshots-directory removal must reach stable storage before the
    // manifest unlink. Otherwise a crash could expose resurrected snapshots
    // with no manifest proving their ownership or lifecycle state.
    sync_parent(&run)
}

fn remove_expired_manifest_locked(data: &Path, manifest: &RecoveryManifest) -> Result<(), String> {
    let path = manifest_path(data, &manifest.run_id);
    validate_private_regular(&path, 1)?;
    std::fs::remove_file(&path)
        .map_err(|error| format!("remove expired recovery manifest: {error}"))?;
    // Persist the terminal manifest unlink as a separate directory-ordering
    // point after the snapshots directory is durably absent.
    sync_parent(&run_dir(data, &manifest.run_id))
}

pub fn acknowledge_expired(data: &Path, run: &str) -> Result<(), String> {
    let _guard = lock(data)?;
    let mut manifest = read_manifest(data, run)?;
    if manifest.state != RecoveryState::Expired {
        return Err("only an expired recovery manifest can be acknowledged".into());
    }
    if !manifest.retirement_acknowledged {
        manifest.retirement_acknowledged = true;
        manifest.updated_at = crate::history::now_secs();
        write_manifest(data, &manifest)?;
    }
    finalize_expired_locked(data, &manifest)
}

fn persist_restored_retirement_locked(
    data: &Path,
    manifest: &mut RecoveryManifest,
) -> Result<(), String> {
    transition(manifest, RecoveryState::Expired)?;
    manifest.retirement_acknowledged = true;
    manifest.retirement_event_required = false;
    manifest.reason = Some("restore completed and retained evidence was retired".into());
    for item in &mut manifest.items {
        if item.snapshot_file.is_some() {
            item.state = ItemState::Expired;
        }
    }
    // The completed-restore event is already durable. Persist the terminal
    // manifest before unlinking any snapshot so a crash can only leave an
    // acknowledged Expired manifest for the next prune pass to finalize.
    write_manifest(data, manifest)
}

pub fn retire_restored(data: &Path, run: &str) -> Result<(), String> {
    let _guard = lock(data)?;
    validate_run_id(run)?;
    let run_path = run_dir(data, run);
    match run_path.symlink_metadata() {
        Ok(_) => validate_private_directory(&run_path)?,
        // History retention may already have removed an otherwise empty run
        // directory. With no directory, no recovery-owned child can remain.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("inspect recovery retirement directory: {error}")),
    }
    let path = manifest_path(data, run);
    let mut manifest = match path.symlink_metadata() {
        Ok(_) => read_manifest(data, run)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let snapshots = run_path.join(SNAPSHOTS);
            return match snapshots.symlink_metadata() {
                // Both recovery-owned artifacts are absent. This is the
                // durable end state of a completed retirement, so retrying the
                // caller after the final unlink is harmless.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    sync_parent(&run_path)
                }
                Ok(_) => Err(
                    "cannot prove completed recovery retirement: snapshots remain without a manifest"
                        .into(),
                ),
                Err(error) => Err(format!(
                    "inspect recovery snapshots after missing manifest: {error}"
                )),
            };
        }
        Err(error) => return Err(format!("inspect recovery retirement manifest: {error}")),
    };
    match manifest.state {
        RecoveryState::Restored => persist_restored_retirement_locked(data, &mut manifest)?,
        RecoveryState::Expired if manifest.retirement_acknowledged => {}
        RecoveryState::Expired => {
            return Err(
                "expired recovery evidence is still awaiting durable retirement authority".into(),
            )
        }
        _ => {
            return Err("only a completed restore can retire its recovery evidence".into());
        }
    }
    finalize_expired_locked(data, &manifest)
}

fn prune_impl(
    data: &Path,
    config: &RecoveryConfig,
    dry_run: bool,
    all: bool,
    expiry_event_required: bool,
) -> Result<PruneReport, String> {
    let _guard = lock(data)?;
    let now = crate::history::now_secs();
    // Prune is the recovery path when the ordinary selection bound is full,
    // so it must inspect the complete private recovery inventory.
    let mut manifests = scan_manifests(data, None)?;
    manifests.sort_by_key(|manifest| manifest.created_at);
    let scanned = manifests.len();
    let mut report = PruneReport {
        dry_run,
        manifests_scanned: scanned,
        snapshots_removed: 0,
        bytes_removed: 0,
        retained_pinned: 0,
        retained_within_limits: 0,
        manifests_removed: 0,
        expired_runs: Vec::new(),
    };
    let mut active = Vec::with_capacity(manifests.len());
    for manifest in manifests {
        if manifest.state == RecoveryState::Expired {
            if manifest.retirement_acknowledged {
                report.manifests_removed += 1;
                if !dry_run {
                    finalize_expired_locked(data, &manifest)?;
                }
            } else {
                // This may be a prior process's crash window after persisting
                // Expired but before recording RecoveryExpired. Keep returning
                // it until management durably records and acknowledges it.
                report.expired_runs.push(manifest.run_id.clone());
            }
        } else {
            active.push(manifest);
        }
    }
    let mut total = active
        .iter()
        .flat_map(|m| &m.items)
        .filter(|i| i.snapshot_file.is_some() && !matches!(i.state, ItemState::Expired))
        .map(|i| i.preimage_bytes)
        .sum::<u64>();
    for mut manifest in active {
        if report.snapshots_removed >= config.prune_batch {
            break;
        }
        let pruning_started = prune_intent_started(&manifest);
        let preparing_expired =
            manifest.state == RecoveryState::Preparing && now > manifest.preparation_lease_until;
        let retire_after_restore = manifest.state == RecoveryState::Restored;
        // A crash can leave Restored durable before the caller records its
        // completion event. Only explicit management prune can establish a
        // replacement RecoveryExpired event for that uncertain state.
        if retire_after_restore && !expiry_event_required {
            continue;
        }
        if manifest.pinned && !retire_after_restore && !pruning_started {
            report.retained_pinned += 1;
            continue;
        }
        if manifest.state == RecoveryState::Preparing
            && now <= manifest.preparation_lease_until
            && !pruning_started
        {
            continue;
        }
        if matches!(
            manifest.state,
            RecoveryState::CommitPartial
                | RecoveryState::UndoPreflight
                | RecoveryState::UndoInProgress
        ) && !pruning_started
        {
            continue;
        }
        let age_expired = logically_expired(&manifest, config, now);
        if !all
            && !pruning_started
            && !age_expired
            && !preparing_expired
            && !retire_after_restore
            && total <= config.max_total_bytes
        {
            if manifest.items.iter().any(|item| {
                item.snapshot_file.is_some() && !matches!(item.state, ItemState::Expired)
            }) {
                report.retained_within_limits += 1;
            }
            continue;
        }
        let candidate = all
            || pruning_started
            || age_expired
            || preparing_expired
            || retire_after_restore
            || total > config.max_total_bytes;
        if !candidate {
            continue;
        }

        // An item-level Expired state is durable unlink intent. Reconcile any
        // crash-left files carrying that intent before allocating the remaining
        // batch to new items. The manifest-wide event requirement is sticky:
        // automatic retention may finish a management-started batch, but can
        // never silently finalize it without its RecoveryExpired event.
        let event_required = manifest.retirement_event_required || expiry_event_required;
        let mut pending = Vec::new();
        for (index, item) in manifest.items.iter().enumerate() {
            if item.snapshot_file.is_none() || item.state != ItemState::Expired {
                continue;
            }
            let path = linked_snapshot_path(data, &manifest, item)?;
            if owned_snapshot_exists(&path)? {
                pending.push((index, path));
            }
        }
        let available_batch = config.prune_batch.saturating_sub(report.snapshots_removed);
        let pending_count = pending.len().min(available_batch);
        let mut remaining_batch = available_batch.saturating_sub(pending_count);
        let mut planned = Vec::new();
        for (index, item) in manifest.items.iter().enumerate() {
            if remaining_batch == 0 {
                break;
            }
            if item.snapshot_file.is_none() || item.state == ItemState::Expired {
                continue;
            }
            let path = linked_snapshot_path(data, &manifest, item)?;
            let present = owned_snapshot_exists(&path)?;
            planned.push((index, path, present));
            remaining_batch -= 1;
        }

        for (index, _) in pending.iter().take(pending_count) {
            report.snapshots_removed += 1;
            report.bytes_removed = report
                .bytes_removed
                .saturating_add(manifest.items[*index].preimage_bytes);
        }
        for (index, _, _) in &planned {
            report.snapshots_removed += 1;
            report.bytes_removed = report
                .bytes_removed
                .saturating_add(manifest.items[*index].preimage_bytes);
            total = total.saturating_sub(manifest.items[*index].preimage_bytes);
        }

        if !dry_run && (!planned.is_empty() || manifest.retirement_event_required != event_required)
        {
            for (index, _, _) in &planned {
                manifest.items[*index].state = ItemState::Expired;
            }
            manifest.retirement_event_required = event_required;
            manifest.reason = Some("recovery pruning is incomplete and will resume".into());
            // This is the non-restorable intent point. No linked snapshot is
            // unlinked until the item states and event provenance are durable.
            write_manifest(data, &manifest)?;
        }
        if !dry_run {
            for (_, path) in pending.iter().take(pending_count) {
                std::fs::remove_file(path)
                    .map_err(|error| format!("remove pending recovery snapshot: {error}"))?;
            }
            for (_, path, present) in &planned {
                if *present {
                    std::fs::remove_file(path)
                        .map_err(|error| format!("remove recovery snapshot: {error}"))?;
                }
            }
        }

        let outstanding = manifest
            .items
            .iter()
            .filter(|item| {
                item.snapshot_file.is_some() && !matches!(item.state, ItemState::Expired)
            })
            .count();
        let fully_expired = if dry_run {
            outstanding == planned.len() && pending.len() == pending_count
        } else {
            outstanding == 0 && pending.len() == pending_count
        };
        if fully_expired {
            report.expired_runs.push(manifest.run_id.clone());
            if !dry_run {
                if !matches!(manifest.state, RecoveryState::Expired) {
                    transition(&mut manifest, RecoveryState::Expired)?;
                }
                manifest.retirement_event_required = event_required;
                manifest.retirement_acknowledged = !event_required;
                manifest.reason = Some("retained snapshots expired or were pruned".into());
                write_manifest(data, &manifest)?;
                if !event_required {
                    finalize_expired_locked(data, &manifest)?;
                    report.manifests_removed += 1;
                }
            }
        }
    }
    Ok(report)
}

pub fn prune(
    data: &Path,
    config: &RecoveryConfig,
    dry_run: bool,
    all: bool,
) -> Result<PruneReport, String> {
    prune_impl(data, config, dry_run, all, true)
}

fn transition(manifest: &mut RecoveryManifest, next: RecoveryState) -> Result<(), String> {
    use RecoveryState::*;
    let legal = matches!(
        (manifest.state, next),
        (Preparing, CommitPartial | Conflicted | Corrupt | Expired)
            | (CommitPartial, Available | Conflicted | Corrupt)
            | (Available, UndoPreflight | Expired | Corrupt)
            | (UndoPreflight, UndoInProgress | Conflicted | Corrupt)
            | (UndoInProgress, Restored | Conflicted | Corrupt)
            | (
                Conflicted,
                UndoPreflight | UndoInProgress | Expired | Corrupt
            )
            | (Restored, Expired | Corrupt)
            | (Corrupt, Expired)
    );
    if !legal {
        return Err(format!(
            "illegal recovery state transition {} -> {}",
            manifest.state.as_str(),
            next.as_str()
        ));
    }
    manifest.state = next;
    manifest.updated_at = crate::history::now_secs();
    Ok(())
}

fn open_parent(path: &Path) -> Result<File, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("open destination parent {}: {error}", path.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("destination parent must be a real directory".into());
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| format!("open destination parent descriptor: {error}"))?;
    let opened = file.metadata().map_err(|error| error.to_string())?;
    if opened.dev() != metadata.dev() || opened.ino() != metadata.ino() {
        return Err("destination parent changed while opening".into());
    }
    Ok(file)
}

fn revalidate_parent(prepared: &PreparedItem) -> Result<(), String> {
    let current = open_parent(&prepared.parent_path)?;
    let expected = prepared
        .parent
        .metadata()
        .map_err(|error| error.to_string())?;
    let observed = current.metadata().map_err(|error| error.to_string())?;
    if expected.dev() != observed.dev() || expected.ino() != observed.ino() {
        return Err("destination parent path changed before commit".into());
    }
    Ok(())
}

fn leaf_name(path: &Path) -> Result<CString, String> {
    let name = path
        .file_name()
        .ok_or("managed output has no filename")?
        .as_bytes();
    if name.is_empty() || name == b"." || name == b".." || name.contains(&b'/') {
        return Err("invalid managed output filename".into());
    }
    CString::new(name).map_err(|_| "managed output filename contains NUL".into())
}

fn openat_read(parent: &File, name: &CString) -> Result<File, std::io::Error> {
    let descriptor = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(descriptor) })
    }
}

fn validate_eligible_file(file: &File, max: u64) -> Result<Identity, String> {
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() } || metadata.nlink() != 1
    {
        return Err("destination must be a current-user-owned, single-link regular file".into());
    }
    if metadata.len() > max {
        return Err(format!("destination exceeds the {max}-byte snapshot limit"));
    }
    if has_extended_metadata(file)? {
        return Err("destination has ACLs, extended attributes, or resource forks".into());
    }
    Ok(identity(&metadata))
}

fn validate_staged_file(file: &File) -> Result<Identity, String> {
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
        || has_extended_metadata(file)?
    {
        return Err(
            "staged output must be an owned single-link regular file without extended metadata"
                .into(),
        );
    }
    Ok(identity(&metadata))
}

fn identity(metadata: &std::fs::Metadata) -> Identity {
    Identity {
        device: metadata.dev(),
        inode: metadata.ino(),
        len: metadata.len(),
        mode: metadata.permissions().mode() & 0o7777,
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
    }
}

fn precommit_matches(prepared: &PreparedItem, item: &RecoveryItem) -> Result<(), String> {
    revalidate_parent(prepared)?;
    match (
        &prepared.preimage,
        openat_read(&prepared.parent, &prepared.destination_name),
    ) {
        (Some(expected), Ok(file)) => {
            let observed = validate_eligible_file(&file, expected.len)?;
            if observed.device != expected.device
                || observed.inode != expected.inode
                || observed.len != expected.len
                || observed.mode != expected.mode
                || observed.modified_seconds != expected.modified_seconds
                || observed.modified_nanoseconds != expected.modified_nanoseconds
            {
                return Err("destination identity or metadata changed after snapshot".into());
            }
            let hash = hash_file(file, expected.len)?;
            if Some(&hash) != item.preimage_hash.as_ref() {
                return Err("destination bytes changed after snapshot".into());
            }
            Ok(())
        }
        (Some(_), Err(error)) => Err(format!("snapshotted destination disappeared: {error}")),
        (None, Err(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        (None, Ok(_)) => Err("a concurrent writer created the destination before commit".into()),
        (None, Err(error)) => Err(format!("cannot recheck absent destination: {error}")),
    }
}

fn copy_snapshot(source: &mut File, destination: &Path, max: u64) -> Result<String, String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    let mut output = options
        .open(destination)
        .map_err(|error| format!("create private recovery snapshot: {error}"))?;
    let mut hasher = blake3::Hasher::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = source
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        total = total.saturating_add(count as u64);
        if total > max {
            return Err("snapshot grew beyond its configured limit".into());
        }
        hasher.update(&buffer[..count]);
        output
            .write_all(&buffer[..count])
            .map_err(|error| error.to_string())?;
    }
    output.sync_all().map_err(|error| error.to_string())?;
    Ok(hasher.finalize().to_hex().to_string())
}

fn hash_file(mut file: File, max: u64) -> Result<String, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    let mut hasher = blake3::Hasher::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        total = total.saturating_add(count as u64);
        if total > max {
            return Err("file exceeds the bounded hash limit".into());
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn hash_file_path(path: &Path, max: u64) -> Result<String, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| format!("open bounded hash input: {error}"))?;
    validate_private_regular(path, 1)?;
    hash_file(file, max)
}

fn validate_private_regular(path: &Path, links: u64) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != links
        || metadata.permissions().mode() & 0o777 != 0o600
    {
        return Err("recovery file ownership, type, links, or permissions are invalid".into());
    }
    Ok(())
}

fn validate_private_directory(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o700
    {
        return Err("recovery directory ownership, type, or permissions are invalid".into());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn has_extended_metadata(file: &File) -> Result<bool, String> {
    let count = unsafe { libc::flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0) };
    if count < 0 {
        Err(format!(
            "inspect extended attributes: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(count > 0)
    }
}

#[cfg(target_os = "macos")]
fn has_extended_metadata(file: &File) -> Result<bool, String> {
    let count = unsafe { libc::flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0, 0) };
    if count < 0 {
        return Err(format!(
            "inspect extended attributes: {}",
            std::io::Error::last_os_error()
        ));
    }
    if count > 0 {
        return Ok(true);
    }
    macos_has_acl(file)
}

#[cfg(target_os = "macos")]
fn macos_has_acl(file: &File) -> Result<bool, String> {
    type Acl = *mut libc::c_void;
    unsafe extern "C" {
        fn acl_get_fd(fd: libc::c_int) -> Acl;
        fn acl_get_entry(
            acl: Acl,
            entry_id: libc::c_int,
            entry: *mut *mut libc::c_void,
        ) -> libc::c_int;
        fn acl_free(object: *mut libc::c_void) -> libc::c_int;
    }
    let acl = unsafe { acl_get_fd(file.as_raw_fd()) };
    if acl.is_null() {
        let error = std::io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(code) if code == libc::ENOENT || code == libc::ENOTSUP)
        {
            return Ok(false);
        }
        return Err(format!("inspect file ACL: {error}"));
    }
    let mut entry = std::ptr::null_mut();
    let result = unsafe { acl_get_entry(acl, 0, &mut entry) };
    unsafe { acl_free(acl) };
    if result < 0 {
        Err(format!(
            "inspect file ACL entries: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(result == 1)
    }
}

#[cfg(target_os = "linux")]
fn supported_filesystem(file: &File) -> Result<(), String> {
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::fstatfs(file.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(format!(
            "inspect destination filesystem: {}",
            std::io::Error::last_os_error()
        ));
    }
    let kind = unsafe { stat.assume_init() }.f_type as u64;
    if matches!(
        kind,
        0xEF53 | 0x5846_5342 | 0x9123_683E | 0x0102_1994 | 0x794C_7630
    ) {
        Ok(())
    } else {
        Err(format!(
            "destination filesystem type 0x{kind:x} is not in the tested local allowlist"
        ))
    }
}

#[cfg(target_os = "macos")]
fn supported_filesystem(file: &File) -> Result<(), String> {
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::fstatfs(file.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(format!(
            "inspect destination filesystem: {}",
            std::io::Error::last_os_error()
        ));
    }
    let stat = unsafe { stat.assume_init() };
    let bytes = stat
        .f_fstypename
        .iter()
        .map(|value| *value as u8)
        .take_while(|value| *value != 0)
        .collect::<Vec<_>>();
    let kind = String::from_utf8_lossy(&bytes);
    if matches!(kind.as_ref(), "apfs" | "hfs") {
        Ok(())
    } else {
        Err(format!(
            "destination filesystem '{kind}' is not in the tested local allowlist"
        ))
    }
}

fn atomic_no_replace_supported() -> bool {
    cfg!(any(target_os = "linux", target_os = "macos"))
}

fn rename_replace(parent: &File, source: &CString, destination: &CString) -> Result<(), String> {
    let result = unsafe {
        libc::renameat(
            parent.as_raw_fd(),
            source.as_ptr(),
            parent.as_raw_fd(),
            destination.as_ptr(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

#[cfg(target_os = "linux")]
fn rename_no_replace(parent: &File, source: &CString, destination: &CString) -> Result<(), String> {
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            parent.as_raw_fd(),
            source.as_ptr(),
            parent.as_raw_fd(),
            destination.as_ptr(),
            1u32,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

#[cfg(target_os = "macos")]
fn rename_no_replace(parent: &File, source: &CString, destination: &CString) -> Result<(), String> {
    let result = unsafe {
        libc::renameatx_np(
            parent.as_raw_fd(),
            source.as_ptr(),
            parent.as_raw_fd(),
            destination.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

fn sync_directory_handle(parent: &File) -> Result<(), String> {
    parent
        .sync_all()
        .map_err(|error| format!("sync destination directory: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    // Pins the blocking-exclusive contract of std::fs::File advisory locking
    // (the primitive recovery::exclusive_guard relies on for cross-process
    // mutual exclusion): a second holder must block until the first drops or
    // unlocks. Guards against a future accidental switch to a non-blocking
    // try_lock.
    #[cfg(unix)]
    #[test]
    fn lock_exclusive_blocks_until_the_holder_releases() {
        use std::sync::mpsc;
        use std::thread;
        use std::time::Duration;

        let path = std::env::temp_dir().join(format!(
            "uhm-file-lock-{}-{}.lock",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_file(&path);

        let holder = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .expect("open holder lock file");
        holder.lock().expect("acquire holder lock");

        let (tx, rx) = mpsc::channel::<()>();
        let waiter_path = path.clone();
        let waiter = thread::spawn(move || {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&waiter_path)
                .expect("open waiter lock file");
            // Blocks until the holder releases the lock.
            file.lock().expect("acquire waiter lock");
            tx.send(()).expect("signal acquisition");
            file.unlock().expect("release waiter lock");
        });

        // While the holder keeps the lock, the waiter must not have acquired it.
        thread::sleep(Duration::from_millis(200));
        assert!(
            rx.try_recv().is_err(),
            "lock returned while another holder still held the lock"
        );

        // Closing the holder's descriptor releases the lock; the waiter proceeds.
        drop(holder);
        assert!(
            rx.recv_timeout(Duration::from_secs(5)).is_ok(),
            "lock never completed after the holder released"
        );
        waiter.join().expect("waiter thread panicked");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn live_coordinator_ownership_blocks_stale_prune_until_commit_finishes() {
        use std::sync::mpsc;
        use std::time::Duration;

        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = RecoveryConfig::default();
        let (destination, staging) = paths(root.path(), "owned.txt");
        std::fs::write(&destination, b"before").unwrap();
        let mut coordinator = prepare_with_lease(
            &data,
            "owned-run-0001",
            &config,
            &[(destination.clone(), staging.clone())],
            0,
        )
        .unwrap();
        coordinator.manifest.preparation_lease_until = 0;
        write_manifest(&data, &coordinator.manifest).unwrap();
        std::fs::write(&staging, b"after").unwrap();

        let (entered_tx, entered_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let prune_data = data.clone();
        let prune_config = config.clone();
        let waiter = std::thread::spawn(move || {
            entered_tx.send(()).unwrap();
            let result = prune(&prune_data, &prune_config, false, true);
            done_tx.send(result).unwrap();
        });
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            done_rx.recv_timeout(Duration::from_millis(150)).is_err(),
            "stale pruning acquired the inventory while a coordinator still owned it"
        );

        coordinator.commit(config.max_total_bytes).unwrap();
        let report = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("prune never resumed after commit released ownership")
            .unwrap();
        waiter.join().unwrap();
        assert_eq!(report.snapshots_removed, 1);
        assert_eq!(std::fs::read(destination).unwrap(), b"after");
    }

    #[test]
    fn commit_refuses_when_durable_preimage_evidence_disappears() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = RecoveryConfig::default();
        let run = "evidence-run-01";
        let (destination, staging) = paths(root.path(), "evidence.txt");
        std::fs::write(&destination, b"before").unwrap();
        let mut coordinator = prepare(
            &data,
            run,
            &config,
            &[(destination.clone(), staging.clone())],
        )
        .unwrap();
        std::fs::write(&staging, b"after").unwrap();
        std::fs::remove_file(
            run_dir(&data, run)
                .join(SNAPSHOTS)
                .join("output-000.preimage"),
        )
        .unwrap();

        let error = coordinator.commit(config.max_total_bytes).unwrap_err();
        assert!(
            error.contains("preimage evidence is unavailable"),
            "{error}"
        );
        assert_eq!(std::fs::read(&destination).unwrap(), b"before");
        drop(coordinator);
        cleanup_incomplete_capture(&data, run);
    }

    fn paths(root: &Path, name: &str) -> (PathBuf, PathBuf) {
        (root.join(name), root.join(format!(".uhm-stage-{name}")))
    }

    fn committed_replacement() -> (tempfile::TempDir, PathBuf, RecoveryConfig, String) {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let (destination, staging) = paths(root.path(), "document.txt");
        std::fs::write(&destination, b"before").unwrap();
        let config = RecoveryConfig::default();
        let run = "run-00000001".to_string();
        let mut coordinator = prepare(
            &data,
            &run,
            &config,
            &[(destination.clone(), staging.clone())],
        )
        .unwrap();
        std::fs::write(&staging, b"after").unwrap();
        coordinator.commit(config.max_total_bytes).unwrap();
        (root, data, config, run)
    }

    fn commit_named(
        root: &Path,
        data: &Path,
        config: &RecoveryConfig,
        run: &str,
        name: &str,
    ) -> PathBuf {
        let (destination, staging) = paths(root, name);
        std::fs::write(&destination, b"before").unwrap();
        let mut coordinator =
            prepare(data, run, config, &[(destination.clone(), staging.clone())]).unwrap();
        std::fs::write(&staging, b"after").unwrap();
        coordinator.commit(config.max_total_bytes).unwrap();
        destination
    }

    fn record_expiry_event(data: &Path, run: &str) {
        crate::history::record_recovery_event(
            data,
            &crate::config::HistoryConfig::default(),
            run,
            "recovery",
            "minimal",
            crate::history::EventKind::RecoveryExpired,
            "expired",
            Some("retained recovery evidence expired or was explicitly pruned"),
            0,
            None,
        )
        .unwrap();
    }

    #[test]
    fn consent_is_separate_and_one_job_does_not_persist() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = RecoveryConfig::default();
        assert!(!effective_enabled(&data, &config));
        let once = classify(&data, root.path(), &["new.txt".into()], &config, true, true);
        assert!(once.requested);
        assert!(once.all_eligible());
        assert!(!effective_enabled(&data, &config));
        enable(&data).unwrap();
        assert!(effective_enabled(&data, &config));
        disable(&data).unwrap();
        assert!(!effective_enabled(&data, &config));
        let configured = RecoveryConfig {
            enabled: true,
            ..RecoveryConfig::default()
        };
        assert!(
            !classify(
                &data,
                root.path(),
                &["new.txt".into()],
                &configured,
                true,
                false,
            )
            .requested
        );
    }

    #[test]
    fn eligibility_rejects_symlinks_hardlinks_and_oversize_files() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = RecoveryConfig {
            max_file_bytes: 3,
            ..RecoveryConfig::default()
        };
        std::fs::write(root.path().join("large"), b"1234").unwrap();
        assert!(
            !classify(&data, root.path(), &["large".into()], &config, true, true).all_eligible()
        );
        std::fs::write(root.path().join("target"), b"x").unwrap();
        symlink(root.path().join("target"), root.path().join("link")).unwrap();
        assert!(
            !classify(&data, root.path(), &["link".into()], &config, true, true).all_eligible()
        );
        std::fs::hard_link(root.path().join("target"), root.path().join("hard")).unwrap();
        assert!(
            !classify(&data, root.path(), &["target".into()], &config, true, true).all_eligible()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn eligibility_rejects_extended_attributes_when_supported() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let path = root.path().join("tagged");
        std::fs::write(&path, b"value").unwrap();
        let file = File::open(&path).unwrap();
        let name = CString::new("user.uhm-recovery-test").unwrap();
        let value = b"present";
        let result = unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        };
        if result != 0 {
            let code = std::io::Error::last_os_error().raw_os_error();
            if matches!(code, Some(value) if value == libc::ENOTSUP || value == libc::EPERM) {
                return;
            }
            panic!("set test xattr: {}", std::io::Error::last_os_error());
        }
        assert!(!classify(
            &data,
            root.path(),
            &["tagged".into()],
            &RecoveryConfig::default(),
            true,
            true,
        )
        .all_eligible());
    }

    #[test]
    fn capture_is_private_durable_and_verified_undo_restores_bytes_and_mode() {
        let (root, data, config, run) = committed_replacement();
        let destination = root.path().join("document.txt");
        let snapshot = run_dir(&data, &run)
            .join(SNAPSHOTS)
            .join("output-000.preimage");
        let metadata = std::fs::metadata(&snapshot).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read(&snapshot).unwrap(), b"before");
        let preview = preview_restore(&data, &run, &config, false).unwrap();
        assert!(preview.items[0].conflict.is_none());
        let report = restore(&data, &run, "undo-00000001", &config, false).unwrap();
        assert_eq!(report.outcome, "verified_restore");
        assert_eq!(std::fs::read(destination).unwrap(), b"before");
        assert_eq!(
            read_manifest(&data, &run).unwrap().state,
            RecoveryState::Restored
        );
    }

    #[test]
    fn created_output_is_removed_only_when_postimage_matches() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = RecoveryConfig::default();
        let run = "run-00000002";
        let (destination, staging) = paths(root.path(), "created.txt");
        let mut coordinator = prepare(
            &data,
            run,
            &config,
            &[(destination.clone(), staging.clone())],
        )
        .unwrap();
        std::fs::write(&staging, b"created").unwrap();
        coordinator.commit(config.max_total_bytes).unwrap();
        let report = restore(&data, run, "undo-00000002", &config, false).unwrap();
        assert_eq!(report.removed, 1);
        assert!(!destination.exists());
    }

    #[test]
    fn later_edit_conflicts_and_force_has_a_distinct_outcome() {
        let (root, data, config, run) = committed_replacement();
        let destination = root.path().join("document.txt");
        std::fs::write(&destination, b"later work").unwrap();
        let error = restore(&data, &run, "undo-00000003", &config, false).unwrap_err();
        assert!(error.contains("verified undo refused"), "{error}");
        assert_eq!(std::fs::read(&destination).unwrap(), b"later work");
        let report = restore(&data, &run, "force-0000001", &config, true).unwrap();
        assert_eq!(report.outcome, "forced_restore");
        assert_eq!(std::fs::read(destination).unwrap(), b"before");
    }

    #[test]
    fn forced_restore_provenance_is_sticky_across_ordinary_resume() {
        let (root, data, config, run) = committed_replacement();
        let destination = root.path().join("document.txt");
        std::fs::write(&destination, b"later work").unwrap();
        let mut manifest = read_manifest(&data, &run).unwrap();
        manifest.forced_restore = true;
        manifest.state = RecoveryState::UndoInProgress;
        manifest.items[0].state = ItemState::UndoPending;
        write_manifest(&data, &manifest).unwrap();
        let report = restore(&data, &run, "undo-ordinary1", &config, false).unwrap();
        assert_eq!(report.outcome, "forced_restore");
        assert!(read_manifest(&data, &run).unwrap().forced_restore);
    }

    #[test]
    fn permission_changes_conflict_and_force_rejects_a_symlink_swap() {
        let (root, data, config, run) = committed_replacement();
        let destination = root.path().join("document.txt");
        std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o600)).unwrap();
        let preview = preview_restore(&data, &run, &config, false).unwrap();
        assert!(preview.items[0]
            .conflict
            .as_deref()
            .is_some_and(|reason| reason.contains("mode")));

        let (other_root, other_data, other_config, other_run) = committed_replacement();
        let other_destination = other_root.path().join("document.txt");
        let outside = other_root.path().join("outside.txt");
        std::fs::write(&outside, b"outside").unwrap();
        std::fs::remove_file(&other_destination).unwrap();
        symlink(&outside, &other_destination).unwrap();
        assert!(restore(
            &other_data,
            &other_run,
            "force-0000002",
            &other_config,
            true,
        )
        .is_err());
        assert_eq!(std::fs::read(outside).unwrap(), b"outside");
    }

    #[test]
    fn multi_output_commit_preflights_every_destination_before_first_rename() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = RecoveryConfig::default();
        let (first, first_stage) = paths(root.path(), "first");
        let (second, second_stage) = paths(root.path(), "second");
        std::fs::write(&first, b"one").unwrap();
        std::fs::write(&second, b"two").unwrap();
        let mut coordinator = prepare(
            &data,
            "run-00000003",
            &config,
            &[
                (first.clone(), first_stage.clone()),
                (second.clone(), second_stage.clone()),
            ],
        )
        .unwrap();
        std::fs::write(&first_stage, b"new one").unwrap();
        std::fs::write(&second_stage, b"new two").unwrap();
        std::fs::write(&second, b"concurrent").unwrap();
        assert!(coordinator.commit(config.max_total_bytes).is_err());
        assert_eq!(std::fs::read(first).unwrap(), b"one");
    }

    #[test]
    fn interrupted_partial_commit_resumes_only_hash_matching_items() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = RecoveryConfig::default();
        let run = "run-00000004";
        let (first, first_stage) = paths(root.path(), "first");
        let (second, second_stage) = paths(root.path(), "second");
        std::fs::write(&first, b"one").unwrap();
        std::fs::write(&second, b"two").unwrap();
        let mut coordinator = prepare(
            &data,
            run,
            &config,
            &[
                (first.clone(), first_stage.clone()),
                (second.clone(), second_stage.clone()),
            ],
        )
        .unwrap();
        std::fs::write(&first_stage, b"new one").unwrap();
        std::fs::write(&second_stage, b"new two").unwrap();
        for (index, stage) in [&first_stage, &second_stage].iter().enumerate() {
            coordinator.manifest.items[index].staged_hash =
                Some(hash_file(File::open(stage).unwrap(), config.max_total_bytes).unwrap());
            coordinator.manifest.items[index].postimage_mode =
                Some(std::fs::metadata(stage).unwrap().permissions().mode() & 0o7777);
            coordinator.manifest.items[index].state = ItemState::Staged;
        }
        transition(&mut coordinator.manifest, RecoveryState::CommitPartial).unwrap();
        // Durable intent exists, then simulate a crash after rename and before
        // the committed item state is persisted.
        write_manifest(&data, &coordinator.manifest).unwrap();
        let prepared = &coordinator.prepared[0];
        rename_replace(
            &prepared.parent,
            &prepared.staging_name,
            &prepared.destination_name,
        )
        .unwrap();
        drop(coordinator);

        assert_eq!(resume_commit(&data, run, &config).unwrap(), run);
        assert_eq!(std::fs::read(first).unwrap(), b"new one");
        assert_eq!(std::fs::read(second).unwrap(), b"new two");
        assert_eq!(
            read_manifest(&data, run).unwrap().state,
            RecoveryState::Available
        );
    }

    #[test]
    fn corrupt_snapshot_blocks_restore() {
        let (_root, data, config, run) = committed_replacement();
        let snapshot = run_dir(&data, &run)
            .join(SNAPSHOTS)
            .join("output-000.preimage");
        std::fs::write(&snapshot, b"tampered").unwrap();
        std::fs::set_permissions(&snapshot, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            preview_restore(&data, &run, &config, false).unwrap().items[0]
                .conflict
                .is_some()
        );
        assert!(restore(&data, &run, "undo-00000004", &config, false).is_err());
    }

    #[test]
    fn pruning_respects_pins_and_uses_an_expiry_transition() {
        let (_root, data, mut config, run) = committed_replacement();
        config.max_age_days = 1;
        pin(&data, &run, &config, true).unwrap();
        let mut aged = read_manifest(&data, &run).unwrap();
        aged.created_at = crate::history::now_secs().saturating_sub(2 * 86_400);
        write_manifest(&data, &aged).unwrap();
        let retained = prune(&data, &config, false, false).unwrap();
        assert_eq!(retained.snapshots_removed, 0);
        pin(&data, &run, &config, false).unwrap();
        let removed = prune(&data, &config, false, false).unwrap();
        assert_eq!(removed.snapshots_removed, 1);
        assert_eq!(
            read_manifest(&data, &run).unwrap().state,
            RecoveryState::Expired
        );
    }

    #[test]
    fn prune_dry_run_does_not_expire_new_file_only_manifest() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = RecoveryConfig {
            max_age_days: 1,
            ..RecoveryConfig::default()
        };
        let run = "run-00000011";
        let (destination, staging) = paths(root.path(), "created-only");
        let mut coordinator =
            prepare(&data, run, &config, &[(destination, staging.clone())]).unwrap();
        std::fs::write(&staging, b"new").unwrap();
        coordinator.commit(config.max_total_bytes).unwrap();
        let mut manifest = read_manifest(&data, run).unwrap();
        manifest.created_at = crate::history::now_secs().saturating_sub(2 * 86_400);
        write_manifest(&data, &manifest).unwrap();
        let path = manifest_path(&data, run);
        let before = std::fs::read(&path).unwrap();
        let report = prune(&data, &config, true, false).unwrap();
        assert!(report.expired_runs.contains(&run.to_string()));
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[test]
    fn recovery_total_byte_limit_is_global_across_runs() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = RecoveryConfig {
            max_total_bytes: 6,
            max_file_bytes: 6,
            ..RecoveryConfig::default()
        };
        let (first, first_stage) = paths(root.path(), "cap-first");
        std::fs::write(&first, b"four").unwrap();
        let mut first_run = prepare(
            &data,
            "cap-run-0001",
            &config,
            &[(first, first_stage.clone())],
        )
        .unwrap();
        std::fs::write(&first_stage, b"next").unwrap();
        first_run.commit(config.max_total_bytes).unwrap();
        let (second, second_stage) = paths(root.path(), "cap-second");
        std::fs::write(&second, b"four").unwrap();
        let error = match prepare(&data, "cap-run-0002", &config, &[(second, second_stage)]) {
            Ok(_) => panic!("global cap unexpectedly accepted a second snapshot"),
            Err(error) => error,
        };
        assert!(error.contains("global retained preimages"), "{error}");
    }

    #[test]
    fn state_machine_rejects_false_success_transitions() {
        let mut manifest = RecoveryManifest {
            schema_version: SCHEMA_VERSION,
            run_id: "run-00000009".into(),
            created_at: 1,
            updated_at: 1,
            state: RecoveryState::Preparing,
            pinned: false,
            forced_restore: false,
            selection_sequence: 1,
            expires_at: 100,
            retirement_acknowledged: false,
            retirement_event_required: false,
            preparation_lease_until: 0,
            items: vec![],
            reason: None,
        };
        assert!(transition(&mut manifest, RecoveryState::Available).is_err());
        assert!(transition(&mut manifest, RecoveryState::CommitPartial).is_ok());
        assert!(transition(&mut manifest, RecoveryState::Available).is_ok());
    }

    #[test]
    fn last_alias_prefers_the_restorable_manifest_and_names_the_skipped_run() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = RecoveryConfig::default();
        for (run, name) in [
            ("run-restorable1", "first.txt"),
            ("run-shadowing99", "second.txt"),
        ] {
            let (destination, staging) = paths(root.path(), name);
            std::fs::write(&destination, b"before").unwrap();
            let mut coordinator =
                prepare(&data, run, &config, &[(destination, staging.clone())]).unwrap();
            std::fs::write(&staging, b"after").unwrap();
            coordinator.commit(config.max_total_bytes).unwrap();
        }
        restore(&data, "run-shadowing99", "undo-shadow0001", &config, false).unwrap();
        let mut shadowing = read_manifest(&data, "run-shadowing99").unwrap();
        shadowing.updated_at = read_manifest(&data, "run-restorable1").unwrap().updated_at + 100;
        write_manifest(&data, &shadowing).unwrap();

        let preview = preview_restore(&data, "last", &config, false).unwrap();
        assert_eq!(preview.run_id, "run-restorable1");
        let note = preview
            .alias_note
            .expect("skipping a newer non-restorable manifest must be named");
        assert!(note.contains("run-restorable1"), "{note}");
        assert!(note.contains("run-shadowing99"), "{note}");
        assert!(note.contains("restored"), "{note}");

        let explicit = preview_restore(&data, "run-restorable1", &config, false).unwrap();
        assert!(explicit.alias_note.is_none());
    }

    #[test]
    fn last_alias_with_only_non_restorable_manifests_reports_their_state() {
        let (_root, data, config, run) = committed_replacement();
        restore(&data, &run, "undo-00000042", &config, false).unwrap();
        let error = preview_restore(&data, "last", &config, false).unwrap_err();
        assert!(error.contains("no restorable recovery manifest"), "{error}");
        assert!(error.contains(&run), "{error}");
        assert!(error.contains("restored"), "{error}");
    }

    #[test]
    fn prune_all_removes_retained_in_cap_snapshots_and_plain_prune_reports_the_skip() {
        let (_root, data, config, run) = committed_replacement();
        let retained = prune(&data, &config, false, false).unwrap();
        assert_eq!(retained.snapshots_removed, 0);
        assert_eq!(retained.retained_within_limits, 1);
        assert_eq!(status(&data, None, &config).unwrap().snapshots, 1);
        let removed = prune(&data, &config, false, true).unwrap();
        assert_eq!(removed.snapshots_removed, 1);
        assert_eq!(removed.retained_within_limits, 0);
        assert_eq!(status(&data, None, &config).unwrap().snapshots, 0);
        assert_eq!(
            read_manifest(&data, &run).unwrap().state,
            RecoveryState::Expired
        );
    }

    #[test]
    fn prune_dry_run_all_removes_nothing() {
        let (_root, data, config, run) = committed_replacement();
        let report = prune(&data, &config, true, true).unwrap();
        assert_eq!(report.snapshots_removed, 1);
        let snapshot = run_dir(&data, &run)
            .join(SNAPSHOTS)
            .join("output-000.preimage");
        assert!(snapshot.exists());
        assert_eq!(status(&data, None, &config).unwrap().snapshots, 1);
        assert_eq!(
            read_manifest(&data, &run).unwrap().state,
            RecoveryState::Available
        );
    }

    #[test]
    fn prune_batch_does_not_expire_a_partly_retained_manifest() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = RecoveryConfig {
            prune_batch: 1,
            ..RecoveryConfig::default()
        };
        let first = paths(root.path(), "first.txt");
        let second = paths(root.path(), "second.txt");
        std::fs::write(&first.0, b"before-first").unwrap();
        std::fs::write(&second.0, b"before-second").unwrap();
        let run = "batch-run-0001";
        let mut coordinator = prepare(
            &data,
            run,
            &config,
            &[
                (first.0.clone(), first.1.clone()),
                (second.0.clone(), second.1.clone()),
            ],
        )
        .unwrap();
        std::fs::write(&first.1, b"after-first").unwrap();
        std::fs::write(&second.1, b"after-second").unwrap();
        coordinator.commit(config.max_total_bytes).unwrap();

        let first_pass = prune(&data, &config, false, true).unwrap();
        assert_eq!(first_pass.snapshots_removed, 1);
        assert!(first_pass.expired_runs.is_empty());
        let partial = read_manifest(&data, run).unwrap();
        assert_eq!(partial.state, RecoveryState::Available);
        assert_eq!(
            partial
                .items
                .iter()
                .filter(|item| item.state == ItemState::Expired)
                .count(),
            1
        );
        let error = preview_restore(&data, "last", &config, false).unwrap_err();
        assert!(error.contains("expiring"), "{error}");

        // An automatic retention pass may resume the durable intent, but the
        // management-started event requirement must remain sticky.
        let second_pass = prune_impl(&data, &config, false, false, false).unwrap();
        assert_eq!(second_pass.snapshots_removed, 1);
        assert_eq!(second_pass.expired_runs, [run]);
        let terminal = read_manifest(&data, run).unwrap();
        assert_eq!(terminal.state, RecoveryState::Expired);
        assert!(terminal.retirement_event_required);
        assert!(!terminal.retirement_acknowledged);
    }

    #[test]
    fn last_skips_newer_partially_pruned_evidence() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = RecoveryConfig {
            prune_batch: 1,
            ..RecoveryConfig::default()
        };
        let older = "partial-older01";
        commit_named(root.path(), &data, &config, older, "older.txt");
        pin(&data, older, &config, true).unwrap();

        let newer = "partial-newer02";
        let first = paths(root.path(), "newer-first.txt");
        let second = paths(root.path(), "newer-second.txt");
        std::fs::write(&first.0, b"before-first").unwrap();
        std::fs::write(&second.0, b"before-second").unwrap();
        let mut coordinator = prepare(
            &data,
            newer,
            &config,
            &[
                (first.0.clone(), first.1.clone()),
                (second.0.clone(), second.1.clone()),
            ],
        )
        .unwrap();
        std::fs::write(&first.1, b"after-first").unwrap();
        std::fs::write(&second.1, b"after-second").unwrap();
        coordinator.commit(config.max_total_bytes).unwrap();

        let partial = prune(&data, &config, false, true).unwrap();
        assert_eq!(partial.snapshots_removed, 1);
        assert!(prune_intent_started(&read_manifest(&data, newer).unwrap()));

        let preview = preview_restore(&data, "last", &config, false).unwrap();
        assert_eq!(preview.run_id, older);
        let note = preview.alias_note.unwrap();
        assert!(note.contains(newer), "{note}");
        assert!(note.contains("expiring"), "{note}");
        let explicit = preview_restore(&data, newer, &config, false).unwrap_err();
        assert!(explicit.contains("expiring"), "{explicit}");
    }

    #[test]
    fn incomplete_inventory_never_resolves_last() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = RecoveryConfig::default();
        for (run, name) in [
            ("scan-run-0001", "one.txt"),
            ("scan-run-0002", "two.txt"),
            ("scan-run-0003", "three.txt"),
        ] {
            commit_named(root.path(), &data, &config, run, name);
        }
        let bounded = RecoveryConfig {
            scan_limit: 2,
            ..config
        };
        let error = preview_restore(&data, "last", &bounded, false).unwrap_err();
        assert!(error.contains("exceeds recovery.scan_limit"), "{error}");
        assert_eq!(
            std::fs::read(root.path().join("one.txt")).unwrap(),
            b"after"
        );
        assert_eq!(
            std::fs::read(root.path().join("two.txt")).unwrap(),
            b"after"
        );
        assert_eq!(
            std::fs::read(root.path().join("three.txt")).unwrap(),
            b"after"
        );
    }

    #[test]
    fn prune_all_can_unwedge_an_inventory_past_the_selection_limit() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = RecoveryConfig::default();
        for (run, name) in [
            ("prune-run-001", "one.txt"),
            ("prune-run-002", "two.txt"),
            ("prune-run-003", "three.txt"),
        ] {
            commit_named(root.path(), &data, &config, run, name);
        }
        let bounded = RecoveryConfig {
            scan_limit: 2,
            ..config
        };
        let expired = prune(&data, &bounded, false, true).unwrap();
        assert_eq!(expired.expired_runs.len(), 3);
        let pending = prune(&data, &bounded, false, true).unwrap();
        assert_eq!(pending.expired_runs.len(), 3);
        assert_eq!(pending.manifests_removed, 0);
        for run in &pending.expired_runs {
            record_expiry_event(&data, run);
            acknowledge_expired(&data, run).unwrap();
        }
        commit_named(root.path(), &data, &bounded, "prune-run-004", "four.txt");
        assert_eq!(
            preview_restore(&data, "last", &bounded, false)
                .unwrap()
                .run_id,
            "prune-run-004"
        );
    }

    #[test]
    fn history_only_directories_do_not_consume_recovery_capacity() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        for index in 0..8 {
            let directory = runs_dir(&data).join(format!("history-{index:08}"));
            dirs::ensure_private_dir(&directory).unwrap();
            std::fs::write(directory.join("proposal.json"), b"history").unwrap();
        }
        let config = RecoveryConfig {
            scan_limit: 1,
            ..RecoveryConfig::default()
        };
        commit_named(root.path(), &data, &config, "history-run-01", "managed.txt");
        assert_eq!(
            preview_restore(&data, "last", &config, false)
                .unwrap()
                .run_id,
            "history-run-01"
        );
    }

    #[test]
    fn pinning_an_old_run_does_not_change_last_ordering() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = RecoveryConfig::default();
        commit_named(root.path(), &data, &config, "ordered-run-01", "first.txt");
        commit_named(root.path(), &data, &config, "ordered-run-02", "second.txt");
        pin(&data, "ordered-run-01", &config, true).unwrap();
        assert_eq!(
            preview_restore(&data, "last", &config, false)
                .unwrap()
                .run_id,
            "ordered-run-02"
        );
    }

    #[test]
    fn management_expiry_retries_across_event_and_ack_crash_windows() {
        let (_root, data, config, run) = committed_replacement();
        let first = prune(&data, &config, false, true).unwrap();
        assert_eq!(first.expired_runs, [run.as_str()]);
        let pending = read_manifest(&data, &run).unwrap();
        assert_eq!(pending.state, RecoveryState::Expired);
        assert!(!pending.retirement_acknowledged);

        // Crash before the history event: the next management pass must report
        // the same pending run rather than finalize it silently.
        let before_event_retry = prune(&data, &config, false, true).unwrap();
        assert_eq!(before_event_retry.expired_runs, [run.as_str()]);
        assert_eq!(before_event_retry.manifests_removed, 0);
        assert!(manifest_path(&data, &run).exists());

        // Crash after the event but before recovery acknowledgment: the run is
        // still reported, and recording the event again is idempotent.
        record_expiry_event(&data, &run);
        let after_event_retry = prune(&data, &config, false, true).unwrap();
        assert_eq!(after_event_retry.expired_runs, [run.as_str()]);
        record_expiry_event(&data, &run);
        let expiry_events = crate::history::events_for(&data, &run)
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == crate::history::EventKind::RecoveryExpired)
            .count();
        assert_eq!(expiry_events, 1);

        acknowledge_expired(&data, &run).unwrap();
        assert!(!manifest_path(&data, &run).exists());
    }

    #[test]
    fn acknowledged_expiry_is_finalized_after_an_ack_crash_window() {
        let (_root, data, config, run) = committed_replacement();
        prune(&data, &config, false, true).unwrap();
        let mut manifest = read_manifest(&data, &run).unwrap();
        manifest.retirement_acknowledged = true;
        write_manifest(&data, &manifest).unwrap();

        let resumed = prune(&data, &config, false, true).unwrap();
        assert_eq!(resumed.manifests_removed, 1);
        assert!(resumed.expired_runs.is_empty());
        assert!(!manifest_path(&data, &run).exists());
    }

    #[test]
    fn prepare_preserves_pending_management_expiry_and_sequence_high_water() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = RecoveryConfig {
            scan_limit: 1,
            ..RecoveryConfig::default()
        };
        commit_named(root.path(), &data, &config, "pending-run-01", "first.txt");
        let old_sequence = read_manifest(&data, "pending-run-01")
            .unwrap()
            .selection_sequence;
        prune(&data, &config, false, true).unwrap();

        commit_named(root.path(), &data, &config, "pending-run-02", "second.txt");
        let pending = read_manifest(&data, "pending-run-01").unwrap();
        assert_eq!(pending.state, RecoveryState::Expired);
        assert!(!pending.retirement_acknowledged);
        assert!(
            read_manifest(&data, "pending-run-02")
                .unwrap()
                .selection_sequence
                > old_sequence
        );
    }

    #[test]
    fn prepare_automatic_prune_finalizes_without_an_expiry_event() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = RecoveryConfig {
            scan_limit: 1,
            ..RecoveryConfig::default()
        };
        commit_named(root.path(), &data, &config, "automatic-run-01", "first.txt");
        let mut expired = read_manifest(&data, "automatic-run-01").unwrap();
        expired.expires_at = crate::history::now_secs();
        write_manifest(&data, &expired).unwrap();

        commit_named(
            root.path(),
            &data,
            &config,
            "automatic-run-02",
            "second.txt",
        );
        assert!(!manifest_path(&data, "automatic-run-01").exists());
        assert!(crate::history::events_for(&data, "automatic-run-01").is_err());
    }

    #[test]
    fn prepare_does_not_retire_a_restored_run_without_event_authority() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = RecoveryConfig {
            scan_limit: 2,
            ..RecoveryConfig::default()
        };
        commit_named(root.path(), &data, &config, "restored-run-01", "first.txt");
        restore(
            &data,
            "restored-run-01",
            "restore-operation-01",
            &config,
            false,
        )
        .unwrap();

        commit_named(root.path(), &data, &config, "restored-run-02", "second.txt");
        let preserved = read_manifest(&data, "restored-run-01").unwrap();
        assert_eq!(preserved.state, RecoveryState::Restored);
        assert!(!preserved.retirement_acknowledged);
    }

    #[test]
    fn prune_retries_a_snapshot_unlinked_before_manifest_persistence() {
        let (_root, data, config, run) = committed_replacement();
        let snapshot = run_dir(&data, &run)
            .join(SNAPSHOTS)
            .join("output-000.preimage");
        std::fs::remove_file(snapshot).unwrap();

        let report = prune(&data, &config, false, true).unwrap();
        assert_eq!(report.expired_runs, [run.as_str()]);
        assert_eq!(
            read_manifest(&data, &run).unwrap().state,
            RecoveryState::Expired
        );
    }

    #[test]
    fn logical_expiry_blocks_explicit_and_last_restore_before_gc() {
        let (_root, data, config, run) = committed_replacement();
        let mut manifest = read_manifest(&data, &run).unwrap();
        manifest.expires_at = crate::history::now_secs();
        write_manifest(&data, &manifest).unwrap();

        let explicit = preview_restore(&data, &run, &config, false).unwrap_err();
        assert!(explicit.contains("expired"), "{explicit}");
        let last = preview_restore(&data, "last", &config, false).unwrap_err();
        assert!(last.contains("expired"), "{last}");
        let report = status(&data, Some(&run), &config).unwrap();
        assert_eq!(report.state, "expired");
    }

    #[test]
    fn restored_retirement_persists_expired_before_unlink_and_retries_idempotently() {
        let (_root, data, config, run) = committed_replacement();
        restore(&data, &run, "retire-order-01", &config, false).unwrap();
        let snapshot = run_dir(&data, &run)
            .join(SNAPSHOTS)
            .join("output-000.preimage");
        assert!(snapshot.exists());

        let guard = lock(&data).unwrap();
        let mut manifest = read_manifest(&data, &run).unwrap();
        persist_restored_retirement_locked(&data, &mut manifest).unwrap();
        assert!(snapshot.exists(), "terminal state must precede unlink");
        drop(guard);

        let persisted = read_manifest(&data, &run).unwrap();
        assert_eq!(persisted.state, RecoveryState::Expired);
        assert!(persisted.retirement_acknowledged);

        // Retry after a crash between terminal-manifest persistence and
        // unlinking. A further retry after the final unlink is also a no-op.
        retire_restored(&data, &run).unwrap();
        assert!(!manifest_path(&data, &run).exists());
        assert!(!run_dir(&data, &run).join(SNAPSHOTS).exists());
        retire_restored(&data, &run).unwrap();
    }

    #[test]
    fn finalization_removes_and_syncs_snapshots_before_manifest_unlink() {
        let (_root, data, config, run) = committed_replacement();
        restore(&data, &run, "retire-order-02", &config, false).unwrap();
        let _guard = lock(&data).unwrap();
        let mut manifest = read_manifest(&data, &run).unwrap();
        persist_restored_retirement_locked(&data, &mut manifest).unwrap();

        remove_expired_snapshots_locked(&data, &manifest).unwrap();
        assert!(!run_dir(&data, &run).join(SNAPSHOTS).exists());
        assert!(
            manifest_path(&data, &run).exists(),
            "manifest must remain until snapshot-directory removal is synced"
        );

        remove_expired_manifest_locked(&data, &manifest).unwrap();
        assert!(!manifest_path(&data, &run).exists());
    }

    #[test]
    fn restored_retirement_does_not_finalize_an_unacknowledged_expiry() {
        let (_root, data, config, run) = committed_replacement();
        prune(&data, &config, false, true).unwrap();

        let error = retire_restored(&data, &run).unwrap_err();
        assert!(
            error.contains("awaiting durable retirement authority"),
            "{error}"
        );
        let pending = read_manifest(&data, &run).unwrap();
        assert_eq!(pending.state, RecoveryState::Expired);
        assert!(!pending.retirement_acknowledged);
    }

    #[test]
    fn completed_restore_retires_only_recovery_owned_files() {
        let (root, data, config, run) = committed_replacement();
        let history_artifact = run_dir(&data, &run).join("proposal.json");
        std::fs::write(&history_artifact, b"history-owned").unwrap();
        restore(&data, &run, "retire-undo-01", &config, false).unwrap();
        retire_restored(&data, &run).unwrap();
        assert!(!manifest_path(&data, &run).exists());
        assert!(!run_dir(&data, &run).join(SNAPSHOTS).exists());
        assert_eq!(std::fs::read(history_artifact).unwrap(), b"history-owned");
        assert_eq!(
            std::fs::read(root.path().join("document.txt")).unwrap(),
            b"before"
        );
    }

    #[test]
    fn default_per_file_limit_handles_an_eight_mib_managed_workload() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let config = RecoveryConfig::default();
        let (destination, staging) = paths(root.path(), "bounded.bin");
        let preimage = vec![0x41; config.max_file_bytes as usize];
        let postimage = vec![0x42; config.max_file_bytes as usize];
        std::fs::write(&destination, &preimage).unwrap();
        let started = std::time::Instant::now();
        let mut coordinator = prepare(
            &data,
            "run-00000010",
            &config,
            &[(destination.clone(), staging.clone())],
        )
        .unwrap();
        std::fs::write(&staging, &postimage).unwrap();
        coordinator.commit(config.max_total_bytes).unwrap();
        restore(&data, "run-00000010", "undo-00000010", &config, false).unwrap();
        let elapsed = started.elapsed();
        assert_eq!(
            std::fs::metadata(destination).unwrap().len(),
            config.max_file_bytes
        );
        assert!(elapsed < std::time::Duration::from_secs(30), "{elapsed:?}");
        eprintln!("8 MiB capture, commit, and undo: {elapsed:?}");
    }
}
