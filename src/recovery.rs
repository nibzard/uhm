//! Hash-verified restoration for Plan 4 managed file outputs.
//!
//! This is deliberately not a general rollback engine. Snapshot capture is a
//! separate opt-in, and only descriptor-validated sibling-staged regular files
//! can acquire a verified restore manifest.

use crate::config::RecoveryConfig;
use crate::dirs;
use fs2::FileExt;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryManifest {
    pub schema_version: u32,
    pub run_id: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub state: RecoveryState,
    pub pinned: bool,
    pub forced_restore: bool,
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
    file.lock_exclusive()
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

pub fn prepare(
    data: &Path,
    run: &str,
    config: &RecoveryConfig,
    outputs: &[(PathBuf, PathBuf)],
) -> Result<Coordinator, String> {
    validate_run_id(run)?;
    if outputs.is_empty() || outputs.len() > 16 {
        return Err("recovery capture requires 1..16 managed outputs".into());
    }
    let _guard = lock(data)?;
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
        items: Vec::new(),
        reason: None,
    };
    let mut prepared = Vec::new();
    let mut total_snapshot_bytes = 0u64;
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
                    return Err("preimages exceed the configured total recovery byte limit".into());
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
    Ok(Coordinator {
        data_dir: data.into(),
        run_dir: run_path,
        manifest,
        prepared,
    })
}

impl Coordinator {
    pub fn state(&self) -> &'static str {
        self.manifest.state.as_str()
    }

    pub fn commit(&mut self, max_total: u64) -> Result<Vec<PathBuf>, String> {
        let _guard = lock(&self.data_dir)?;
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

fn resolve_manifest_run(data: &Path, selected: &str, limit: usize) -> Result<String, String> {
    if selected != "last" {
        validate_run_id(selected)?;
        return Ok(selected.into());
    }
    let directory = runs_dir(data);
    let mut newest: Option<(u64, String)> = None;
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("no retained recovery manifest is available".into());
        }
        Err(error) => return Err(format!("scan recovery runs: {error}")),
    };
    for entry in entries.take(limit) {
        let Ok(entry) = entry else { continue };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if validate_run_id(&name).is_err() || !entry.path().join(MANIFEST).is_file() {
            continue;
        }
        if let Ok(manifest) = read_manifest(data, &name) {
            if newest
                .as_ref()
                .is_none_or(|(time, _)| manifest.updated_at > *time)
            {
                newest = Some((manifest.updated_at, name));
            }
        }
    }
    newest
        .map(|(_, run)| run)
        .ok_or_else(|| "no retained recovery manifest is available".into())
}

fn snapshot_path(
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
    validate_private_directory(&run_dir(data, &manifest.run_id).join(SNAPSHOTS))?;
    validate_private_regular(&path, 1)?;
    let observed = hash_file_path(&path, item.preimage_bytes)?;
    if item.preimage_hash.as_deref() != Some(&observed) {
        return Err("retained snapshot hash does not match its manifest".into());
    }
    Ok(path)
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
    let run = resolve_manifest_run(data, selected, config.scan_limit)?;
    let manifest = read_manifest(data, &run)?;
    if !matches!(
        manifest.state,
        RecoveryState::Available
            | RecoveryState::Conflicted
            | RecoveryState::UndoPreflight
            | RecoveryState::UndoInProgress
    ) {
        return Err(format!(
            "recovery manifest is {}, not restorable",
            manifest.state.as_str()
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
        items,
        concurrent_writer_warning: "Each rename is atomic, but the collection is not a transaction; another writer can race the final hash check and rename.",
    })
}

fn create_restore_temporary(
    parent: &File,
    item: &RecoveryItem,
    snapshot: &Path,
) -> Result<CString, String> {
    let name = CString::new(format!(".uhm-restore-{}-{}", &item.id, std::process::id()))
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
    let source = resolve_manifest_run(data, selected, config.scan_limit)?;
    let mut manifest = read_manifest(data, &source)?;
    if !matches!(
        manifest.state,
        RecoveryState::Available
            | RecoveryState::Conflicted
            | RecoveryState::UndoPreflight
            | RecoveryState::UndoInProgress
    ) {
        return Err(format!(
            "recovery manifest is {}, not restorable",
            manifest.state.as_str()
        ));
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
        .map(|value| resolve_manifest_run(data, value, config.scan_limit))
        .transpose()?;
    let entries = match std::fs::read_dir(runs_dir(data)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(error) => return Err(format!("scan recovery status: {error}")),
    };
    for entry in entries.take(config.scan_limit) {
        let Ok(entry) = entry else { continue };
        let Some(run) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if validate_run_id(&run).is_err() || !entry.path().join(MANIFEST).exists() {
            continue;
        }
        let manifest = match read_manifest(data, &run) {
            Ok(value) => value,
            Err(error) => {
                if selected_run.as_deref() == Some(run.as_str()) {
                    report.run_id = Some(run);
                    report.state = "corrupt".into();
                    report.reason = error;
                }
                continue;
            }
        };
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
        if selected_run.as_deref() == Some(run.as_str()) {
            report.run_id = Some(run);
            report.state = manifest.state.as_str().into();
            report.reason = manifest
                .reason
                .unwrap_or_else(|| "recovery manifest validated".into());
        }
    }
    Ok(report)
}

pub fn startup_check(data: &Path, config: &RecoveryConfig) -> usize {
    let Ok(entries) = std::fs::read_dir(runs_dir(data)) else {
        return 0;
    };
    entries
        .take(config.scan_limit.min(32))
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .filter(|run| {
            read_manifest(data, run).is_ok_and(|manifest| {
                matches!(
                    manifest.state,
                    RecoveryState::Preparing
                        | RecoveryState::CommitPartial
                        | RecoveryState::UndoPreflight
                        | RecoveryState::UndoInProgress
                )
            })
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
    let run = resolve_manifest_run(data, selected, config.scan_limit)?;
    let mut manifest = read_manifest(data, &run)?;
    if value {
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
    let run = resolve_manifest_run(data, selected, config.scan_limit)?;
    let mut manifest = read_manifest(data, &run)?;
    if manifest.state != RecoveryState::CommitPartial {
        return Err(format!(
            "only commit_partial recovery can resume; this run is {}",
            manifest.state.as_str()
        ));
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

pub fn prune(
    data: &Path,
    config: &RecoveryConfig,
    dry_run: bool,
    all: bool,
) -> Result<PruneReport, String> {
    let _guard = lock(data)?;
    let now = crate::history::now_secs();
    let cutoff = now.saturating_sub(config.max_age_days.saturating_mul(86_400));
    let entries = match std::fs::read_dir(runs_dir(data)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PruneReport {
                dry_run,
                manifests_scanned: 0,
                snapshots_removed: 0,
                bytes_removed: 0,
                retained_pinned: 0,
                expired_runs: Vec::new(),
            })
        }
        Err(error) => return Err(format!("scan recovery snapshots: {error}")),
    };
    let mut manifests = Vec::new();
    for entry in entries.take(config.scan_limit) {
        let Ok(entry) = entry else { continue };
        let Some(run) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if let Ok(manifest) = read_manifest(data, &run) {
            manifests.push(manifest);
        }
    }
    manifests.sort_by_key(|manifest| manifest.created_at);
    let scanned = manifests.len();
    let mut total = manifests
        .iter()
        .flat_map(|m| &m.items)
        .filter(|i| i.snapshot_file.is_some() && !matches!(i.state, ItemState::Expired))
        .map(|i| i.preimage_bytes)
        .sum::<u64>();
    let mut report = PruneReport {
        dry_run,
        manifests_scanned: scanned,
        snapshots_removed: 0,
        bytes_removed: 0,
        retained_pinned: 0,
        expired_runs: Vec::new(),
    };
    for mut manifest in manifests {
        if report.snapshots_removed >= config.prune_batch {
            break;
        }
        if manifest.pinned {
            report.retained_pinned += 1;
            continue;
        }
        if matches!(
            manifest.state,
            RecoveryState::Preparing
                | RecoveryState::CommitPartial
                | RecoveryState::UndoPreflight
                | RecoveryState::UndoInProgress
        ) {
            continue;
        }
        if !all && manifest.created_at >= cutoff && total <= config.max_total_bytes {
            continue;
        }
        let candidate = all || manifest.created_at < cutoff || total > config.max_total_bytes;
        if !candidate {
            continue;
        }
        let mut changed = false;
        for item in &mut manifest.items {
            let Some(name) = item.snapshot_file.as_deref() else {
                continue;
            };
            if matches!(item.state, ItemState::Expired) {
                continue;
            }
            if report.snapshots_removed >= config.prune_batch {
                break;
            }
            let path = run_dir(data, &manifest.run_id).join(SNAPSHOTS).join(name);
            if path
                != run_dir(data, &manifest.run_id)
                    .join(SNAPSHOTS)
                    .join(format!("{}.preimage", item.id))
            {
                return Err("refusing to prune an unlinked snapshot path".into());
            }
            report.snapshots_removed += 1;
            report.bytes_removed = report.bytes_removed.saturating_add(item.preimage_bytes);
            total = total.saturating_sub(item.preimage_bytes);
            if !dry_run {
                validate_private_regular(&path, 1)?;
                std::fs::remove_file(&path)
                    .map_err(|error| format!("remove recovery snapshot: {error}"))?;
                item.state = ItemState::Expired;
                changed = true;
            }
        }
        if changed
            || manifest
                .items
                .iter()
                .all(|item| item.snapshot_file.is_none())
        {
            report.expired_runs.push(manifest.run_id.clone());
            if !matches!(manifest.state, RecoveryState::Expired) {
                transition(&mut manifest, RecoveryState::Expired)?;
            }
            manifest.reason = Some("retained snapshots expired or were pruned".into());
            write_manifest(data, &manifest)?;
        }
    }
    Ok(report)
}

fn transition(manifest: &mut RecoveryManifest, next: RecoveryState) -> Result<(), String> {
    use RecoveryState::*;
    let legal = matches!(
        (manifest.state, next),
        (Preparing, CommitPartial | Conflicted | Corrupt)
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
        libc::renameat2(
            parent.as_raw_fd(),
            source.as_ptr(),
            parent.as_raw_fd(),
            destination.as_ptr(),
            1,
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
        let prepared = &coordinator.prepared[0];
        rename_replace(
            &prepared.parent,
            &prepared.staging_name,
            &prepared.destination_name,
        )
        .unwrap();
        coordinator.manifest.items[0].postimage_hash =
            coordinator.manifest.items[0].staged_hash.clone();
        coordinator.manifest.items[0].state = ItemState::Committed;
        write_manifest(&data, &coordinator.manifest).unwrap();
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
    fn pruning_respects_pins_and_leaves_expiry_tombstones() {
        let (_root, data, mut config, run) = committed_replacement();
        config.max_age_days = 1;
        let mut aged = read_manifest(&data, &run).unwrap();
        aged.created_at = crate::history::now_secs().saturating_sub(2 * 86_400);
        write_manifest(&data, &aged).unwrap();
        pin(&data, &run, &config, true).unwrap();
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
    fn state_machine_rejects_false_success_transitions() {
        let mut manifest = RecoveryManifest {
            schema_version: SCHEMA_VERSION,
            run_id: "run-00000009".into(),
            created_at: 1,
            updated_at: 1,
            state: RecoveryState::Preparing,
            pinned: false,
            forced_restore: false,
            items: vec![],
            reason: None,
        };
        assert!(transition(&mut manifest, RecoveryState::Available).is_err());
        assert!(transition(&mut manifest, RecoveryState::CommitPartial).is_ok());
        assert!(transition(&mut manifest, RecoveryState::Available).is_ok());
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
