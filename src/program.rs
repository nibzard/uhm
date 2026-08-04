//! Bounded Python microprogram execution. These controls reduce accidents; they are not a sandbox.

use crate::action::{ProgramFileAccess, ProgramProposal, ProgramStdinMode};
use crate::config::ProgramConfig;
use crate::runtime::PythonInventory;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

static CHILD_GROUP: AtomicI32 = AtomicI32::new(0);
static SIGNALS: Once = Once::new();
type ReaderHandle = std::thread::JoinHandle<(Vec<u8>, Vec<u8>)>;

#[derive(Debug)]
pub struct ExecutionResult {
    pub code: i32,
    pub signal: Option<i32>,
    pub stdout: Vec<u8>,
    pub stdout_tail: Vec<u8>,
    pub stderr_tail: Vec<u8>,
    pub timed_out: bool,
    pub output_overflow: bool,
    pub duration: Duration,
    pub helper_setup_duration: Duration,
    pub artifacts: Vec<PathBuf>,
    pub retained_workspace: Option<PathBuf>,
    pub recovery_prepared: bool,
    pub recovery_state: Option<String>,
    pub recovery_reason: Option<String>,
    pub artifact_commit_success: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    HardError,
    Warning,
    Availability,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramContractDiagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
}

#[derive(Serialize)]
struct LauncherResource {
    id: String,
    read_path: Option<String>,
    write_path: Option<String>,
}

#[derive(Serialize)]
struct LauncherContract {
    stdin_path: Option<String>,
    resources: Vec<LauncherResource>,
}

struct StagedOutput {
    destination: PathBuf,
    staging: PathBuf,
    cleanup: bool,
}

const LAUNCHER: &str = r#"import json
import pathlib
import runpy
import sys
import types
from collections import namedtuple

source_path = sys.argv[1]
contract_path = pathlib.Path(sys.argv[2])
contract = json.loads(contract_path.read_text(encoding="utf-8"))
contract_path.unlink()
Resource = namedtuple("Resource", "read_path write_path")
resources = {
    item["id"]: Resource(
        pathlib.Path(item["read_path"]) if item["read_path"] is not None else None,
        pathlib.Path(item["write_path"]) if item["write_path"] is not None else None,
    )
    for item in contract["resources"]
}
module = types.ModuleType("uhm_runtime")
module.stdin_path = pathlib.Path(contract["stdin_path"]) if contract["stdin_path"] is not None else None
def resource(resource_id):
    try:
        return resources[resource_id]
    except KeyError:
        raise KeyError("unknown uhm resource id") from None
module.resource = resource
sys.modules["uhm_runtime"] = module
sys.argv = [source_path]
runpy.run_path(source_path, run_name="__main__")
"#;

const AST_CHECKER: &str = r#"import ast,json,sys
p=json.load(sys.stdin); source=p['source']; ids=set(p['ids']); paths=set(p['paths'])
out=[]
def add(code,severity,message):
    if not any(x['code']==code for x in out): out.append({'code':code,'severity':severity,'message':message})
try: tree=ast.parse(source, filename='<uhm-model-source>')
except SyntaxError:
    add('invalid_python_syntax','hard_error','Python source has invalid syntax.')
    print(json.dumps(out)); raise SystemExit
helper=False; stdin_used=False; resource_calls=set(); write_resource_calls=set(); dynamic_resource=False; dynamic_write=False
aliases=set(); module_aliases=set(); sys_aliases={'sys'}
for n in ast.walk(tree):
    if isinstance(n,ast.ImportFrom) and n.module=='uhm_runtime':
        helper=True
        for a in n.names:
            aliases.add((a.asname or a.name,a.name))
    if isinstance(n,ast.Import):
        for a in n.names:
            if a.name=='uhm_runtime': helper=True; module_aliases.add(a.asname or a.name)
            if a.name=='sys': sys_aliases.add(a.asname or a.name)
stdin_names={a for a,b in aliases if b=='stdin_path'}
resource_names={a for a,b in aliases if b=='resource'}
for n in ast.walk(tree):
    if isinstance(n,ast.Call) and isinstance(n.func,ast.Name) and n.func.id=='input': add('builtin_input_is_unsupported','hard_error','Built-in input() is unsupported because process stdin is closed.')
    if isinstance(n,ast.Attribute) and isinstance(n.value,ast.Name) and n.value.id in sys_aliases and n.attr in ('stdin','__stdin__'): add('process_stdin_is_closed','hard_error','Process stdin is closed; use uhm_runtime.stdin_path.')
    if isinstance(n,ast.Name) and n.id in stdin_names: stdin_used=True
    if isinstance(n,ast.Attribute) and isinstance(n.value,ast.Name) and n.value.id in module_aliases and n.attr=='stdin_path': stdin_used=True
    if isinstance(n,ast.Attribute) and n.attr=='write_path':
        owner=n.value
        if isinstance(owner,ast.Call):
            is_resource=(isinstance(owner.func,ast.Name) and owner.func.id in resource_names) or (isinstance(owner.func,ast.Attribute) and isinstance(owner.func.value,ast.Name) and owner.func.value.id in module_aliases and owner.func.attr=='resource')
            if is_resource and len(owner.args)==1 and isinstance(owner.args[0],ast.Constant) and isinstance(owner.args[0].value,str): write_resource_calls.add(owner.args[0].value)
            elif is_resource: dynamic_write=True
    if isinstance(n,ast.Call):
        direct_open=(isinstance(n.func,ast.Name) and n.func.id=='open') or (isinstance(n.func,ast.Attribute) and n.func.attr=='open')
        if direct_open and n.args and isinstance(n.args[0],ast.Constant) and n.args[0].value in paths: add('declared_path_opened_directly','hard_error','Source opens a declared logical path directly; use resource IDs.')
        if isinstance(n.func,ast.Attribute) and n.func.attr in ('read_text','read_bytes','write_text','write_bytes') and isinstance(n.func.value,ast.Call) and n.func.value.args and isinstance(n.func.value.args[0],ast.Constant) and n.func.value.args[0].value in paths: add('declared_path_opened_directly','hard_error','Source opens a declared logical path directly; use resource IDs.')
        is_resource=(isinstance(n.func,ast.Name) and n.func.id in resource_names) or (isinstance(n.func,ast.Attribute) and isinstance(n.func.value,ast.Name) and n.func.value.id in module_aliases and n.func.attr=='resource')
        if is_resource:
            helper=True
            if len(n.args)==1 and isinstance(n.args[0],ast.Constant) and isinstance(n.args[0].value,str):
                rid=n.args[0].value; resource_calls.add(rid)
                if rid not in ids: add('unknown_resource','hard_error','Source references an undeclared resource ID.')
            else: dynamic_resource=True
if not helper and (p['stdin_required'] or ids): add('helper_not_referenced','hard_error','Declared resources require the uhm_runtime helper.')
if p['stdin_required'] and not stdin_used: add('stdin_not_consumed','warning','Piped input is declared but stdin_path is not statically referenced.')
for rid in p['read_ids']:
    if rid not in resource_calls: add('read_resource_not_consumed','warning','A readable resource is not statically referenced.')
missing_writes=set(p['write_ids'])-write_resource_calls
if missing_writes and not dynamic_write: add('write_resource_not_consumed','hard_error','A writable resource has no statically visible write_path use.')
if missing_writes and dynamic_write: add('write_resource_not_consumed','warning','Dynamic writable-resource access cannot be proven statically.')
if dynamic_resource: add('read_resource_not_consumed','warning','Dynamic resource access cannot be proven statically.')
print(json.dumps(out,separators=(',',':')))
"#;

impl Drop for StagedOutput {
    fn drop(&mut self) {
        if self.cleanup {
            let _ = std::fs::remove_file(&self.staging);
        }
    }
}

pub struct RecoveryRequest<'a> {
    pub data_dir: &'a Path,
    pub run_id: &'a str,
    pub config: &'a crate::config::RecoveryConfig,
    pub allow_unrecoverable: bool,
}

pub struct Request<'a> {
    pub proposal: &'a ProgramProposal,
    pub python: &'a PythonInventory,
    pub stdin: Option<&'a [u8]>,
    pub cwd: &'a Path,
    pub config: &'a ProgramConfig,
    pub containment: crate::containment::Mode,
    pub retain_workspace: bool,
    pub recovery: Option<RecoveryRequest<'a>>,
}

pub fn has_writable_files(proposal: &ProgramProposal) -> bool {
    proposal.files.iter().any(|file| {
        matches!(
            file.access,
            ProgramFileAccess::WriteOnly | ProgramFileAccess::ReadWrite
        )
    })
}

pub fn writable_paths(proposal: &ProgramProposal) -> Vec<String> {
    proposal
        .files
        .iter()
        .filter(|file| {
            matches!(
                file.access,
                ProgramFileAccess::WriteOnly | ProgramFileAccess::ReadWrite
            )
        })
        .map(|file| file.path.clone())
        .collect()
}

pub fn preflight(
    proposal: &ProgramProposal,
    python: &PythonInventory,
    piped_input_present: bool,
) -> Vec<ProgramContractDiagnostic> {
    let mut ids = std::collections::BTreeSet::new();
    let mut paths = std::collections::BTreeSet::new();
    if proposal
        .files
        .iter()
        .any(|file| !ids.insert(&file.id) || !paths.insert(&file.path))
    {
        return vec![ProgramContractDiagnostic {
            code: "duplicate_resource".into(),
            severity: DiagnosticSeverity::HardError,
            message: "Resource IDs and logical paths must each be unique.".into(),
        }];
    }
    if !python.available {
        return vec![ProgramContractDiagnostic {
            code: "runtime_unavailable".into(),
            severity: DiagnosticSeverity::Availability,
            message: "Python 3 runtime is unavailable.".into(),
        }];
    }
    if proposal.stdin_mode == ProgramStdinMode::LocalPath && !piped_input_present {
        return vec![ProgramContractDiagnostic {
            code: "stdin_not_consumed".into(),
            severity: DiagnosticSeverity::HardError,
            message: "stdin_mode=local_path requires piped input.".into(),
        }];
    }
    let python_path = match python.path() {
        Ok(value) => value,
        Err(_) => {
            return vec![ProgramContractDiagnostic {
                code: "runtime_unavailable".into(),
                severity: DiagnosticSeverity::Availability,
                message: "Python 3 runtime is unavailable.".into(),
            }]
        }
    };
    let payload = serde_json::json!({
        "source": proposal.source,
        "ids": proposal.files.iter().map(|file| &file.id).collect::<Vec<_>>(),
        "paths": proposal.files.iter().map(|file| &file.path).collect::<Vec<_>>(),
        "stdin_required": proposal.stdin_mode == ProgramStdinMode::LocalPath,
        "read_ids": proposal.files.iter().filter(|file| matches!(file.access, ProgramFileAccess::ReadOnly | ProgramFileAccess::ReadWrite)).map(|file| &file.id).collect::<Vec<_>>(),
        "write_ids": proposal.files.iter().filter(|file| matches!(file.access, ProgramFileAccess::WriteOnly | ProgramFileAccess::ReadWrite)).map(|file| &file.id).collect::<Vec<_>>(),
    });
    let mut child = match Command::new(python_path)
        .args(["-I", "-S", "-c", AST_CHECKER])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_clear()
        .env(
            "PATH",
            python_path.parent().unwrap_or(Path::new("/usr/bin")),
        )
        .spawn()
    {
        Ok(value) => value,
        Err(_) => {
            return vec![ProgramContractDiagnostic {
                code: "runtime_unavailable".into(),
                severity: DiagnosticSeverity::Availability,
                message: "Python 3 AST checker could not start.".into(),
            }]
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = serde_json::to_writer(&mut stdin, &payload);
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return vec![ProgramContractDiagnostic {
                    code: "runtime_unavailable".into(),
                    severity: DiagnosticSeverity::Availability,
                    message: "Python 3 AST checker did not complete.".into(),
                }];
            }
        }
    }
    let mut bytes = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.by_ref().take(64 * 1024).read_to_end(&mut bytes);
    }
    serde_json::from_slice(&bytes).unwrap_or_else(|_| {
        vec![ProgramContractDiagnostic {
            code: "runtime_unavailable".into(),
            severity: DiagnosticSeverity::Availability,
            message: "Python 3 AST checker returned an invalid result.".into(),
        }]
    })
}

/// Conservative source scan used only to strengthen review messaging.
pub fn detected_effects(source: &str) -> Vec<crate::action::Effect> {
    use crate::action::Effect;
    let lowered = source.to_ascii_lowercase();
    let mut effects = Vec::new();
    let checks: &[(Effect, &[&str])] = &[
        (Effect::NetworkRead, &["urllib", "http.client", "socket"]),
        (
            Effect::ProcessControl,
            &["subprocess", "os.system", "os.kill", "fork(", "execv"],
        ),
        (
            Effect::DeleteLocal,
            &["unlink(", "remove(", "rmtree(", "shutil.move"],
        ),
        (Effect::PrivilegeElevation, &["sudo", "seteuid", "setuid"]),
        (
            Effect::WriteLocal,
            &[
                "write(",
                "write_text",
                "write_bytes",
                "rename(",
                "replace(",
                "'w'",
                "\"w\"",
            ],
        ),
        (Effect::ReadLocal, &["read_text", "read_bytes", "open("]),
    ];
    for (effect, needles) in checks {
        if needles.iter().any(|needle| lowered.contains(needle)) && !effects.contains(effect) {
            effects.push(effect.clone());
        }
    }
    effects
}

pub fn execute(req: Request<'_>) -> std::result::Result<ExecutionResult, String> {
    let setup_started = Instant::now();
    if !req.config.enabled {
        return Err("microprogram execution is disabled by configuration".into());
    }
    if req.proposal.source.len() > req.config.source_max_bytes
        || req.proposal.files.len() > req.config.input_max_paths
        || writable_paths(req.proposal).len() > req.config.output_max_paths
    {
        return Err("microprogram exceeds configured manifest limits".into());
    }
    let python = req.python.path()?;
    install_signal_forwarding();
    let workspace = tempfile::Builder::new()
        .prefix("uhm-program-")
        .tempdir()
        .map_err(|e| format!("create private program workspace: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(workspace.path(), std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("protect program workspace: {e}"))?;
    }
    let source_path = workspace.path().join("model-source.py");
    write_private(&source_path, req.proposal.source.as_bytes())?;
    let launcher_path = workspace.path().join("launcher.py");
    write_private(&launcher_path, LAUNCHER.as_bytes())?;

    let local_input = if req.proposal.stdin_mode == ProgramStdinMode::LocalPath {
        let bytes = req
            .stdin
            .ok_or("program requested local_path stdin, but no piped input is available")?;
        let path = workspace.path().join("input.bin");
        write_private(&path, bytes)?;
        Some(path)
    } else {
        None
    };
    let writable = writable_paths(req.proposal);
    let mut staged = prepare_outputs(req.cwd, &writable)?;
    let mut resources = Vec::new();
    for file in &req.proposal.files {
        let logical = absolute(req.cwd, &file.path);
        let read_path = if matches!(
            file.access,
            ProgramFileAccess::ReadOnly | ProgramFileAccess::ReadWrite
        ) {
            let meta = std::fs::symlink_metadata(&logical).map_err(|e| {
                format!("program resource {} is unavailable: {e}", logical.display())
            })?;
            if meta.file_type().is_symlink() || !meta.is_file() {
                return Err(format!(
                    "program readable resource {} must be a regular file",
                    logical.display()
                ));
            }
            Some(logical.to_string_lossy().into_owned())
        } else {
            None
        };
        let write_path = if matches!(
            file.access,
            ProgramFileAccess::WriteOnly | ProgramFileAccess::ReadWrite
        ) {
            staged
                .iter()
                .find(|value| value.destination == logical)
                .map(|value| value.staging.to_string_lossy().into_owned())
                .ok_or("program writable resource has no staging plan")?
                .into()
        } else {
            None
        };
        resources.push(LauncherResource {
            id: file.id.clone(),
            read_path,
            write_path,
        });
    }
    let contract_path = workspace.path().join("launcher-contract.json");
    let launcher_contract = LauncherContract {
        stdin_path: local_input
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        resources,
    };
    write_private(
        &contract_path,
        &serde_json::to_vec(&launcher_contract).map_err(|error| error.to_string())?,
    )?;
    let mut recovery_reason = None;
    let mut recovery = if let Some(capture) = &req.recovery {
        let paths = staged
            .iter()
            .map(|output| (output.destination.clone(), output.staging.clone()))
            .collect::<Vec<_>>();
        match crate::recovery::prepare_with_lease(
            capture.data_dir,
            capture.run_id,
            capture.config,
            &paths,
            req.config.timeout_secs.saturating_add(2),
        ) {
            Ok(coordinator) => Some(coordinator),
            Err(error) if capture.allow_unrecoverable => {
                crate::recovery::cleanup_incomplete_capture(capture.data_dir, capture.run_id);
                recovery_reason = Some(error);
                None
            }
            Err(error) => {
                crate::recovery::cleanup_incomplete_capture(capture.data_dir, capture.run_id);
                return Err(format!(
                    "recovery snapshot failed before program execution: {error}"
                ));
            }
        }
    } else {
        None
    };
    let recovery_prepared = recovery.is_some();

    let arguments = fixed_arguments(&launcher_path, &source_path, &contract_path);
    let mut cmd = crate::containment::command(
        req.containment,
        python,
        &arguments,
        workspace.path(),
        &[req.cwd],
    )?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("PATH", python.parent().unwrap_or(Path::new("/usr/bin")))
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .env("TMPDIR", workspace.path());
    apply_limits(&mut cmd, req.config);
    let helper_setup_duration = setup_started.elapsed();
    let started = Instant::now();
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn isolated Python runtime: {e}"))?;
    CHILD_GROUP.store(child.id() as i32, Ordering::SeqCst);
    let total = Arc::new(AtomicUsize::new(0));
    let overflow = Arc::new(AtomicBool::new(false));
    let cancel_readers = Arc::new(AtomicBool::new(false));
    let stdout = read_bounded(
        child.stdout.take().expect("piped stdout"),
        total.clone(),
        overflow.clone(),
        cancel_readers.clone(),
        req.config.output_max_bytes,
        req.config.diagnostic_bytes,
        true,
    )?;
    let stderr = read_bounded(
        child.stderr.take().expect("piped stderr"),
        total,
        overflow.clone(),
        cancel_readers.clone(),
        req.config.output_max_bytes,
        req.config.diagnostic_bytes,
        false,
    )?;
    let mut timed_out = false;
    let deadline = started + Duration::from_secs(req.config.timeout_secs);
    let status = 'wait: loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("wait for program: {e}"))?
        {
            break status;
        }
        if overflow.load(Ordering::SeqCst) || Instant::now() >= deadline {
            timed_out = !overflow.load(Ordering::SeqCst);
            terminate(child.id() as i32, libc::SIGTERM);
            let grace = Instant::now();
            loop {
                if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                    break 'wait status;
                }
                if grace.elapsed() >= Duration::from_millis(500) {
                    terminate(child.id() as i32, libc::SIGKILL);
                    break 'wait child.wait().map_err(|e| e.to_string())?;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        } else {
            std::thread::sleep(Duration::from_millis(10));
        }
    };

    // The primary process can exit while a descendant still owns an inherited
    // pipe. Give ordinary buffered output a brief drain window, then terminate
    // remaining in-group descendants. A process that deliberately starts a new
    // session is outside group cleanup, so the nonblocking collectors also obey
    // the absolute deadline and can close their descriptors without EOF.
    let drain_grace = Instant::now() + Duration::from_millis(50);
    while (!stdout.is_finished() || !stderr.is_finished())
        && Instant::now() < deadline
        && Instant::now() < drain_grace
        && !overflow.load(Ordering::SeqCst)
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    let lingering_descendants = !stdout.is_finished() || !stderr.is_finished();
    if lingering_descendants {
        timed_out = true;
        terminate(child.id() as i32, libc::SIGTERM);
        let cleanup_deadline = deadline.min(Instant::now() + Duration::from_millis(500));
        while (!stdout.is_finished() || !stderr.is_finished()) && Instant::now() < cleanup_deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    if !stdout.is_finished() || !stderr.is_finished() {
        terminate(child.id() as i32, libc::SIGKILL);
        cancel_readers.store(true, Ordering::SeqCst);
    }
    cancel_readers.store(true, Ordering::SeqCst);
    let (stdout_bytes, stdout_tail) = stdout.join().unwrap_or_default();
    let (_, mut stderr_tail) = stderr.join().unwrap_or_default();
    CHILD_GROUP.store(0, Ordering::SeqCst);
    let output_overflow = overflow.load(Ordering::SeqCst);
    let _ = std::fs::remove_file(&contract_path);
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal = None;
    let mut code = status.code().unwrap_or_else(|| 128 + signal.unwrap_or(1));
    if timed_out || output_overflow {
        code = 124;
    }
    let mut artifacts = Vec::new();
    if code == 0 {
        if directory_size(workspace.path(), req.config.workspace_max_bytes)?
            > req.config.workspace_max_bytes
        {
            code = 1;
        } else if has_writable_files(req.proposal) {
            let commit = if let Some(coordinator) = &mut recovery {
                for output in &mut staged {
                    output.cleanup = false;
                }
                coordinator.commit(req.config.workspace_max_bytes)
            } else {
                commit_outputs(staged, req.config.workspace_max_bytes)
            };
            match commit {
                Ok(paths) => artifacts = paths,
                Err(error) => {
                    code = 1;
                    let message = format!("uhm managed artifact commit failed: {error}\n");
                    append_tail(
                        &mut stderr_tail,
                        message.as_bytes(),
                        req.config.diagnostic_bytes,
                    );
                    recovery_reason = Some(error);
                }
            }
        }
    }
    if recovery
        .as_ref()
        .is_some_and(|coordinator| coordinator.state() == "preparing")
    {
        recovery.take();
        if let Some(capture) = &req.recovery {
            crate::recovery::cleanup_incomplete_capture(capture.data_dir, capture.run_id);
        }
        recovery_reason.get_or_insert_with(|| {
            "the program did not complete a managed commit, so its uncommitted preimage capture was removed".into()
        });
    }
    let retained_workspace = if req.retain_workspace {
        Some(workspace.into_path())
    } else {
        None
    };
    let artifact_commit_success =
        code == 0 && (!has_writable_files(req.proposal) || !artifacts.is_empty());
    Ok(ExecutionResult {
        code,
        signal,
        stdout: stdout_bytes,
        stdout_tail,
        stderr_tail,
        timed_out,
        output_overflow,
        duration: started.elapsed(),
        helper_setup_duration,
        artifacts,
        retained_workspace,
        recovery_prepared,
        recovery_state: recovery.as_ref().map(|value| value.state().into()),
        recovery_reason,
        artifact_commit_success,
    })
}

fn append_tail(target: &mut Vec<u8>, bytes: &[u8], max: usize) {
    target.extend_from_slice(bytes);
    if target.len() > max {
        target.drain(..target.len() - max);
    }
}

fn fixed_arguments(launcher: &Path, source: &Path, contract: &Path) -> Vec<std::ffi::OsString> {
    vec![
        "-I".into(),
        "-S".into(),
        launcher.as_os_str().to_owned(),
        source.as_os_str().to_owned(),
        contract.as_os_str().to_owned(),
    ]
}

fn absolute(cwd: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn prepare_outputs(cwd: &Path, values: &[String]) -> Result<Vec<StagedOutput>, String> {
    let mut out = Vec::new();
    for value in values {
        let destination = absolute(cwd, value);
        let parent = destination
            .parent()
            .ok_or("program output has no parent directory")?;
        let parent_meta = std::fs::symlink_metadata(parent).map_err(|e| {
            format!(
                "program output directory {} is unavailable: {e}",
                parent.display()
            )
        })?;
        if !parent_meta.is_dir() || parent_meta.file_type().is_symlink() {
            return Err(format!(
                "program output parent {} must be a real directory",
                parent.display()
            ));
        }
        if let Ok(meta) = std::fs::symlink_metadata(&destination) {
            if meta.file_type().is_symlink() || !meta.is_file() {
                return Err(format!(
                    "program output {} is not a regular file",
                    destination.display()
                ));
            }
        }
        let named = tempfile::Builder::new()
            .prefix(".uhm-stage-")
            .tempfile_in(parent)
            .map_err(|e| format!("stage program output in {}: {e}", parent.display()))?;
        let (_, staging) = named.keep().map_err(|e| e.error.to_string())?;
        std::fs::remove_file(&staging).map_err(|e| e.to_string())?;
        out.push(StagedOutput {
            destination,
            staging,
            cleanup: true,
        });
    }
    Ok(out)
}

fn commit_outputs(values: Vec<StagedOutput>, max: u64) -> Result<Vec<PathBuf>, String> {
    let mut total = 0u64;
    for value in &values {
        let meta = std::fs::symlink_metadata(&value.staging).map_err(|e| {
            format!(
                "declared artifact {} was not produced: {e}",
                value.destination.display()
            )
        })?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(format!(
                "staged artifact {} is not a regular file",
                value.destination.display()
            ));
        }
        total = total.saturating_add(meta.len());
        if total > max {
            return Err("staged artifacts exceed the workspace byte limit".into());
        }
    }
    let mut committed = Vec::new();
    for value in values {
        File::open(&value.staging)
            .and_then(|f| f.sync_all())
            .map_err(|e| format!("sync staged artifact: {e}"))?;
        std::fs::rename(&value.staging, &value.destination)
            .map_err(|e| format!("commit artifact {}: {e}", value.destination.display()))?;
        if let Some(parent) = value.destination.parent() {
            let _ = File::open(parent).and_then(|f| f.sync_all());
        }
        committed.push(value.destination.clone());
    }
    Ok(committed)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("create {}: {e}", path.display()))?;
    file.write_all(bytes)
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    file.sync_all()
        .map_err(|e| format!("sync {}: {e}", path.display()))
}

fn directory_size(root: &Path, limit: u64) -> Result<u64, String> {
    let mut total = 0u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(&path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let meta = std::fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() {
                return Err(format!(
                    "program workspace contains a symlink: {}",
                    entry.path().display()
                ));
            }
            if meta.is_dir() {
                pending.push(entry.path());
            } else {
                total = total.saturating_add(meta.len());
                if total > limit {
                    return Ok(total);
                }
            }
        }
    }
    Ok(total)
}

fn read_bounded<R: Read + Send + std::os::fd::AsRawFd + 'static>(
    mut reader: R,
    total: Arc<AtomicUsize>,
    overflow: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    limit: usize,
    tail_limit: usize,
    retain: bool,
) -> Result<ReaderHandle, String> {
    set_nonblocking(reader.as_raw_fd())?;
    Ok(std::thread::spawn(move || {
        let mut retained = Vec::new();
        let mut tail = VecDeque::with_capacity(tail_limit);
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if cancel.load(Ordering::SeqCst) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
                Ok(n) => {
                    let before = total.fetch_add(n, Ordering::SeqCst);
                    if before.saturating_add(n) > limit {
                        overflow.store(true, Ordering::SeqCst);
                    }
                    if retain && before < limit {
                        let take = n.min(limit - before);
                        retained.extend_from_slice(&buf[..take]);
                    }
                    for byte in &buf[..n] {
                        if tail.len() == tail_limit {
                            tail.pop_front();
                        }
                        tail.push_back(*byte);
                    }
                }
            }
        }
        (retained, tail.into_iter().collect())
    }))
}

#[cfg(unix)]
fn set_nonblocking(fd: std::os::fd::RawFd) -> Result<(), String> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        Err(format!(
            "configure nonblocking program output: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
macro_rules! set_limit {
    ($resource:expr, $value:expr) => {{
        let limit = libc::rlimit {
            rlim_cur: $value as libc::rlim_t,
            rlim_max: $value as libc::rlim_t,
        };
        if libc::setrlimit($resource, &limit) == -1 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }};
}

fn apply_limits(cmd: &mut Command, config: &ProgramConfig) {
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        let cpu = config.cpu_secs;
        #[cfg(not(target_os = "macos"))]
        let address = config.address_space_bytes;
        let files = config.open_files;
        #[cfg(target_os = "linux")]
        let children = config.child_processes;
        cmd.pre_exec(move || {
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            libc::umask(0o077);
            set_limit!(libc::RLIMIT_CPU, cpu)?;
            #[cfg(not(target_os = "macos"))]
            set_limit!(libc::RLIMIT_AS, address)?;
            set_limit!(libc::RLIMIT_NOFILE, files)?;
            #[cfg(target_os = "linux")]
            set_limit!(libc::RLIMIT_NPROC, children)?;
            Ok(())
        });
    }
}

fn install_signal_forwarding() {
    SIGNALS.call_once(|| {
        for signal in [libc::SIGINT, libc::SIGTERM] {
            unsafe {
                let _ = signal_hook::low_level::register(signal, move || {
                    let group = CHILD_GROUP.load(Ordering::SeqCst);
                    if group > 0 {
                        terminate(group, signal);
                    }
                });
            }
        }
    });
}

fn terminate(group: i32, signal: i32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-group, signal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Effect, ProgramFile, ProgramRuntime};

    fn proposal(source: &str) -> ProgramProposal {
        ProgramProposal {
            runtime: ProgramRuntime::Python3,
            contract: "uhm_helper_v1".into(),
            source: source.into(),
            summary: "test".into(),
            assumptions: vec![],
            stdin_mode: ProgramStdinMode::None,
            files: vec![],
            effects: vec![Effect::ReadLocal],
        }
    }

    fn run_stdin(source: &str, input: &[u8]) -> Vec<u8> {
        let inventory = crate::runtime::inventory();
        if !inventory.available {
            return Vec::new();
        }
        let cwd = std::env::current_dir().unwrap();
        let mut value = proposal(source);
        value.stdin_mode = ProgramStdinMode::LocalPath;
        let result = execute(Request {
            proposal: &value,
            python: &inventory,
            stdin: Some(input),
            cwd: &cwd,
            config: &ProgramConfig::default(),
            containment: crate::containment::Mode::Off,
            retain_workspace: false,
            recovery: None,
        })
        .unwrap();
        assert_eq!(
            result.code,
            0,
            "{}",
            String::from_utf8_lossy(&result.stderr_tail)
        );
        result.stdout
    }

    #[test]
    fn runs_python_with_a_minimal_environment() {
        let inventory = crate::runtime::inventory();
        if !inventory.available {
            return;
        }
        std::env::set_var("UHM_PROGRAM_TEST_SECRET", "sentinel");
        std::env::set_var("OPENAI_API_KEY", "provider-sentinel");
        std::env::set_var("CEREBRAS_API_KEY", "cerebras-provider-sentinel");
        let cwd = std::env::current_dir().unwrap();
        let result = execute(Request {
            proposal: &proposal(
                "import os\nprint(os.environ.get('UHM_PROGRAM_TEST_SECRET', 'stripped'))\nprint(os.environ.get('OPENAI_API_KEY', 'stripped'))\nprint(os.environ.get('CEREBRAS_API_KEY', 'stripped'))",
            ),
            python: &inventory,
            stdin: None,
            cwd: &cwd,
            config: &ProgramConfig::default(),
            containment: crate::containment::Mode::Off,
            retain_workspace: false,
            recovery: None,
        })
        .unwrap();
        std::env::remove_var("UHM_PROGRAM_TEST_SECRET");
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("CEREBRAS_API_KEY");
        assert_eq!(result.code, 0);
        assert_eq!(result.stdout, b"stripped\nstripped\nstripped\n");
    }

    #[test]
    fn interpreter_arguments_are_fixed_and_source_is_one_opaque_argument() {
        let launcher = Path::new("/tmp/launcher.py");
        let path = Path::new("/tmp/source with spaces;$(nope).py");
        let contract = Path::new("/tmp/contract.json");
        assert_eq!(
            fixed_arguments(launcher, path, contract),
            vec![
                std::ffi::OsString::from("-I"),
                std::ffi::OsString::from("-S"),
                launcher.as_os_str().to_owned(),
                path.as_os_str().to_owned(),
                contract.as_os_str().to_owned(),
            ]
        );
    }

    #[test]
    fn enforces_the_combined_output_limit() {
        let inventory = crate::runtime::inventory();
        if !inventory.available {
            return;
        }
        let cwd = std::env::current_dir().unwrap();
        let config = ProgramConfig {
            output_max_bytes: 1024,
            ..ProgramConfig::default()
        };
        let result = execute(Request {
            proposal: &proposal("print('x' * 100000)"),
            python: &inventory,
            stdin: None,
            cwd: &cwd,
            config: &config,
            containment: crate::containment::Mode::Off,
            retain_workspace: false,
            recovery: None,
        })
        .unwrap();
        assert!(result.output_overflow);
        assert_ne!(result.code, 0);
    }

    #[test]
    fn descendant_held_pipes_are_bounded_and_report_timeout() {
        let inventory = crate::runtime::inventory();
        if !inventory.available {
            return;
        }
        let cwd = std::env::current_dir().unwrap();
        let config = ProgramConfig {
            timeout_secs: 1,
            child_processes: 4096,
            ..ProgramConfig::default()
        };
        let started = Instant::now();
        let result = execute(Request {
            proposal: &proposal(
                "import os,time\npid=os.fork()\nif pid == 0:\n time.sleep(30)\n os._exit(0)\nprint('primary done')",
            ),
            python: &inventory,
            stdin: None,
            cwd: &cwd,
            config: &config,
            containment: crate::containment::Mode::Off,
            retain_workspace: false,
            recovery: None,
        })
        .unwrap();
        assert!(result.timed_out);
        assert_eq!(result.code, 124);
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[cfg(unix)]
    #[test]
    fn workspace_measurement_never_follows_symlinks() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        symlink("/", dir.path().join("outside")).unwrap();
        assert!(directory_size(dir.path(), 1024).is_err());
        let loop_link = dir.path().join("loop");
        symlink(&loop_link, &loop_link).unwrap();
        assert!(directory_size(dir.path(), 1024).is_err());
    }

    #[test]
    fn representative_read_only_transformations_return_results() {
        let cases: &[(&str, &[u8], &[u8])] = &[
            ("import re\nfrom uhm_runtime import stdin_path\np=stdin_path.read_text()\nprint(len([x for x in re.split(r'\\n\\s*\\n',p.strip()) if x]))", b"one\n\ntwo\nline\n\nthree\n", b"3\n"),
            ("import re\nfrom uhm_runtime import stdin_path\ns=stdin_path.read_text()\nprint(len(re.findall(r'(?i)\\bworld\\b',s)))", b"World, world! otherworld", b"2\n"),
            ("import json\nfrom uhm_runtime import stdin_path\ns=stdin_path.read_text()\nw=s.split()\nprint(json.dumps({'characters':len(s),'lines':len(s.splitlines()),'words':len(w)},sort_keys=True))", b"one two\nthree\n", b"{\"characters\": 14, \"lines\": 2, \"words\": 3}\n"),
            ("from uhm_runtime import stdin_path\ns=stdin_path.read_text()\nprint(s.split('BEGIN',1)[1].split('END',1)[0].strip())", b"no BEGIN wanted text END no", b"wanted text\n"),
            ("import json\nfrom uhm_runtime import stdin_path\nfor line in stdin_path.open():\n o=json.loads(line)\n if o['active']: print(o['name'])", b"{\"name\":\"Ada\",\"active\":true}\n{\"name\":\"Lin\",\"active\":false}\n", b"Ada\n"),
            ("import csv\nfrom uhm_runtime import stdin_path\nprint(sum(int(r['amount']) for r in csv.DictReader(stdin_path.open())))", b"name,amount\na,2\nb,5\n", b"7\n"),
            ("from uhm_runtime import stdin_path\nprint('\\n'.join(dict.fromkeys(stdin_path.read_text().splitlines())))", b"a\nb\na\nc\nb\n", b"a\nb\nc\n"),
        ];
        if !crate::runtime::inventory().available {
            return;
        }
        for (source, input, expected) in cases {
            assert_eq!(run_stdin(source, input), *expected);
        }
    }

    #[test]
    fn artifact_staging_commits_on_success_and_not_on_failure() {
        let inventory = crate::runtime::inventory();
        if !inventory.available {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("result.txt");
        std::fs::write(&destination, "original").unwrap();
        let source = "from uhm_runtime import resource\nresource('result').write_path.write_text('replacement')";
        let mut value = proposal(source);
        value.files = vec![ProgramFile {
            id: "result".into(),
            path: "result.txt".into(),
            access: ProgramFileAccess::WriteOnly,
        }];
        value.effects = vec![Effect::WriteLocal];
        let result = execute(Request {
            proposal: &value,
            python: &inventory,
            stdin: None,
            cwd: root.path(),
            config: &ProgramConfig::default(),
            containment: crate::containment::Mode::Off,
            retain_workspace: false,
            recovery: None,
        })
        .unwrap();
        assert_eq!(result.code, 0);
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "replacement"
        );

        std::fs::write(&destination, "original-again").unwrap();
        value.source.push_str("\nraise SystemExit(2)");
        let result = execute(Request {
            proposal: &value,
            python: &inventory,
            stdin: None,
            cwd: root.path(),
            config: &ProgramConfig::default(),
            containment: crate::containment::Mode::Off,
            retain_workspace: false,
            recovery: None,
        })
        .unwrap();
        assert_ne!(result.code, 0);
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "original-again"
        );
        assert!(std::fs::read_dir(root.path()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".uhm-stage-")));
    }

    #[test]
    fn recovery_preimage_is_durable_before_the_child_and_commits_through_coordinator() {
        let inventory = crate::runtime::inventory();
        if !inventory.available {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let destination = root.path().join("result.txt");
        std::fs::write(&destination, "original").unwrap();
        let run = "run-00000100";
        let snapshot = data
            .join("runs")
            .join(run)
            .join("snapshots")
            .join("output-000.preimage");
        let snapshot_literal = serde_json::to_string(snapshot.to_str().unwrap()).unwrap();
        let source = format!(
            "from uhm_runtime import resource\np={snapshot_literal}\nassert open(p,'rb').read() == b'original'\nresource('result').write_path.write_text('replacement')"
        );
        let mut value = proposal(&source);
        value.files = vec![ProgramFile {
            id: "result".into(),
            path: "result.txt".into(),
            access: ProgramFileAccess::WriteOnly,
        }];
        value.effects = vec![Effect::WriteLocal];
        let recovery_config = crate::config::RecoveryConfig::default();
        let result = execute(Request {
            proposal: &value,
            python: &inventory,
            stdin: None,
            cwd: root.path(),
            config: &ProgramConfig::default(),
            containment: crate::containment::Mode::Off,
            retain_workspace: false,
            recovery: Some(RecoveryRequest {
                data_dir: &data,
                run_id: run,
                config: &recovery_config,
                allow_unrecoverable: false,
            }),
        })
        .unwrap();
        assert_eq!(
            result.code,
            0,
            "{}",
            String::from_utf8_lossy(&result.stderr_tail)
        );
        assert_eq!(result.recovery_state.as_deref(), Some("available"));
        assert_eq!(
            std::fs::read_to_string(&destination).unwrap(),
            "replacement"
        );
        crate::recovery::restore(&data, run, "undo-00000100", &recovery_config, false).unwrap();
        assert_eq!(std::fs::read_to_string(destination).unwrap(), "original");

        let failed_run = "run-00000101";
        let mut failed = value.clone();
        failed.source.push_str("\nraise SystemExit(2)");
        let result = execute(Request {
            proposal: &failed,
            python: &inventory,
            stdin: None,
            cwd: root.path(),
            config: &ProgramConfig::default(),
            containment: crate::containment::Mode::Off,
            retain_workspace: false,
            recovery: Some(RecoveryRequest {
                data_dir: &data,
                run_id: failed_run,
                config: &recovery_config,
                allow_unrecoverable: false,
            }),
        })
        .unwrap();
        assert_eq!(result.code, 2);
        assert!(result.recovery_prepared);
        assert!(result.recovery_state.is_none());
        assert!(!data
            .join("runs")
            .join(failed_run)
            .join("recovery.json")
            .exists());
    }

    #[test]
    fn concatenates_multiple_declared_files_in_manifest_order() {
        let inventory = crate::runtime::inventory();
        if !inventory.available {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("one.txt"), "one\n").unwrap();
        std::fs::write(root.path().join("two.txt"), "two\n").unwrap();
        let mut value = proposal("from uhm_runtime import resource\nfor name in ('one','two'):\n print(resource(name).read_path.read_text(),end='')");
        value.files = [("one", "one.txt"), ("two", "two.txt")]
            .into_iter()
            .map(|(id, path)| ProgramFile {
                id: id.into(),
                path: path.into(),
                access: ProgramFileAccess::ReadOnly,
            })
            .collect();
        let result = execute(Request {
            proposal: &value,
            python: &inventory,
            stdin: None,
            cwd: root.path(),
            config: &ProgramConfig::default(),
            containment: crate::containment::Mode::Off,
            retain_workspace: false,
            recovery: None,
        })
        .unwrap();
        assert_eq!(result.code, 0);
        assert_eq!(result.stdout, b"one\ntwo\n");
    }

    #[test]
    fn reports_program_signals() {
        let inventory = crate::runtime::inventory();
        if !inventory.available {
            return;
        }
        let cwd = std::env::current_dir().unwrap();
        let result = execute(Request {
            proposal: &proposal("import os,signal\nos.kill(os.getpid(), signal.SIGTERM)"),
            python: &inventory,
            stdin: None,
            cwd: &cwd,
            config: &ProgramConfig::default(),
            containment: crate::containment::Mode::Off,
            retain_workspace: false,
            recovery: None,
        })
        .unwrap();
        assert_eq!(result.signal, Some(libc::SIGTERM));
        assert_ne!(result.code, 0);
    }

    fn diagnostic_codes(
        source: &str,
        stdin_mode: ProgramStdinMode,
        files: Vec<ProgramFile>,
    ) -> Vec<String> {
        let mut value = proposal(source);
        value.stdin_mode = stdin_mode;
        value.files = files;
        preflight(
            &value,
            &crate::runtime::inventory(),
            stdin_mode == ProgramStdinMode::LocalPath,
        )
        .into_iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
    }

    #[test]
    fn ast_preflight_rejects_observed_contract_failures_without_executing_source() {
        if !crate::runtime::inventory().available {
            return;
        }
        let sentinel = tempfile::tempdir().unwrap().path().join("must-not-exist");
        let executable = format!("open({:?},'w').write('bad')\ninput()", sentinel);
        let codes = diagnostic_codes(&executable, ProgramStdinMode::None, vec![]);
        assert!(codes.contains(&"builtin_input_is_unsupported".into()));
        assert!(!sentinel.exists());
        assert!(
            diagnostic_codes("def broken(: pass", ProgramStdinMode::None, vec![])
                .contains(&"invalid_python_syntax".into())
        );
        assert!(diagnostic_codes(
            "import sys\nprint(sys.stdin.read())",
            ProgramStdinMode::None,
            vec![]
        )
        .contains(&"process_stdin_is_closed".into()));
    }

    #[test]
    fn ast_preflight_understands_helper_aliases_and_separates_warnings() {
        if !crate::runtime::inventory().available {
            return;
        }
        let files = vec![
            ProgramFile {
                id: "source".into(),
                path: "input.txt".into(),
                access: ProgramFileAccess::ReadOnly,
            },
            ProgramFile {
                id: "result".into(),
                path: "output.txt".into(),
                access: ProgramFileAccess::WriteOnly,
            },
        ];
        let mut value = proposal("import uhm_runtime as u\nu.resource('result').write_path.write_text(u.resource('source').read_path.read_text())");
        value.files = files.clone();
        assert!(preflight(&value, &crate::runtime::inventory(), false).is_empty());
        let direct = diagnostic_codes(
            "open('input.txt').read()",
            ProgramStdinMode::None,
            files.clone(),
        );
        assert!(direct.contains(&"declared_path_opened_directly".into()));
        assert!(direct.contains(&"helper_not_referenced".into()));
        let harmless_literal = diagnostic_codes("from uhm_runtime import resource\nprint('input.txt')\nprint(resource('source').read_path)\nresource('result').write_path.write_text('ok')", ProgramStdinMode::None, files.clone());
        assert!(!harmless_literal.contains(&"declared_path_opened_directly".into()));
        let unknown = diagnostic_codes(
            "from uhm_runtime import resource\nprint(resource('missing').read_path)",
            ProgramStdinMode::None,
            files.clone(),
        );
        assert!(unknown.contains(&"unknown_resource".into()));
        let dynamic = diagnostic_codes(
            "from uhm_runtime import resource\nname='source'\nprint(resource(name).read_path)",
            ProgramStdinMode::None,
            vec![files[0].clone()],
        );
        assert!(dynamic.contains(&"read_resource_not_consumed".into()));
        let duplicate = diagnostic_codes(
            "from uhm_runtime import resource\nprint(resource('source').read_path)",
            ProgramStdinMode::None,
            vec![
                files[0].clone(),
                ProgramFile {
                    id: "source".into(),
                    path: "other.txt".into(),
                    access: ProgramFileAccess::ReadOnly,
                },
            ],
        );
        assert_eq!(duplicate, vec!["duplicate_resource"]);
    }

    #[test]
    fn helper_exposes_only_private_capability_paths_and_unlinks_contract() {
        let inventory = crate::runtime::inventory();
        if !inventory.available {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("source.txt"), "hello").unwrap();
        let source = "import os,sys\nfrom uhm_runtime import resource\nr=resource('source'); w=resource('result')\nassert r.write_path is None and w.read_path is None\nassert os.getcwd()!=os.environ.get('USER_CWD')\nassert len(sys.argv)==1\nw.write_path.write_text(r.read_path.read_text().upper())";
        let mut value = proposal(source);
        value.files = vec![
            ProgramFile {
                id: "source".into(),
                path: "source.txt".into(),
                access: ProgramFileAccess::ReadOnly,
            },
            ProgramFile {
                id: "result".into(),
                path: "nested output.txt".into(),
                access: ProgramFileAccess::WriteOnly,
            },
        ];
        std::fs::create_dir(root.path().join("nested")).unwrap();
        value.files[1].path = "nested/result café.txt".into();
        let result = execute(Request {
            proposal: &value,
            python: &inventory,
            stdin: None,
            cwd: root.path(),
            config: &ProgramConfig::default(),
            containment: crate::containment::Mode::Off,
            retain_workspace: true,
            recovery: None,
        })
        .unwrap();
        assert_eq!(
            result.code,
            0,
            "{}",
            String::from_utf8_lossy(&result.stderr_tail)
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("nested/result café.txt")).unwrap(),
            "HELLO"
        );
        let retained = result.retained_workspace.unwrap();
        assert!(retained.join("launcher.py").exists());
        assert!(retained.join("model-source.py").exists());
        assert!(!retained.join("launcher-contract.json").exists());
        std::fs::remove_dir_all(retained).unwrap();
    }

    #[test]
    fn stdin_mode_none_does_not_expose_piped_bytes() {
        let inventory = crate::runtime::inventory();
        if !inventory.available {
            return;
        }
        let cwd = std::env::current_dir().unwrap();
        let result = execute(Request {
            proposal: &proposal("import sys\nfrom uhm_runtime import stdin_path\nprint(stdin_path is None, sys.stdin.read() == '')"),
            python: &inventory,
            stdin: Some(b"LOCAL-ONLY-SENTINEL"),
            cwd: &cwd,
            config: &ProgramConfig::default(),
            containment: crate::containment::Mode::Off,
            retain_workspace: false,
            recovery: None,
        }).unwrap();
        assert_eq!(result.stdout, b"True True\n");
    }

    #[test]
    fn read_write_requires_existing_regular_file_and_commits_replacement() {
        let inventory = crate::runtime::inventory();
        if !inventory.available {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let mut value = proposal("from uhm_runtime import resource\nr=resource('document')\nr.write_path.write_text(r.read_path.read_text().upper())");
        value.files = vec![ProgramFile {
            id: "document".into(),
            path: "document.txt".into(),
            access: ProgramFileAccess::ReadWrite,
        }];
        assert!(execute(Request {
            proposal: &value,
            python: &inventory,
            stdin: None,
            cwd: root.path(),
            config: &ProgramConfig::default(),
            containment: crate::containment::Mode::Off,
            retain_workspace: false,
            recovery: None
        })
        .is_err());
        std::fs::write(root.path().join("document.txt"), "hello").unwrap();
        let result = execute(Request {
            proposal: &value,
            python: &inventory,
            stdin: None,
            cwd: root.path(),
            config: &ProgramConfig::default(),
            containment: crate::containment::Mode::Off,
            retain_workspace: false,
            recovery: None,
        })
        .unwrap();
        assert!(result.artifact_commit_success);
        assert_eq!(
            std::fs::read_to_string(root.path().join("document.txt")).unwrap(),
            "HELLO"
        );
    }

    #[test]
    fn piped_input_can_produce_a_managed_artifact() {
        let inventory = crate::runtime::inventory();
        if !inventory.available {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let mut value = proposal("from uhm_runtime import stdin_path,resource\nresource('result').write_path.write_bytes(stdin_path.read_bytes().upper())");
        value.stdin_mode = ProgramStdinMode::LocalPath;
        value.files = vec![ProgramFile {
            id: "result".into(),
            path: "result.bin".into(),
            access: ProgramFileAccess::WriteOnly,
        }];
        let result = execute(Request {
            proposal: &value,
            python: &inventory,
            stdin: Some(b"hello"),
            cwd: root.path(),
            config: &ProgramConfig::default(),
            containment: crate::containment::Mode::Off,
            retain_workspace: false,
            recovery: None,
        })
        .unwrap();
        assert_eq!(result.code, 0);
        assert_eq!(
            std::fs::read(root.path().join("result.bin")).unwrap(),
            b"HELLO"
        );
    }

    #[test]
    fn resource_ids_are_order_independent_for_adversarial_file_names() {
        let inventory = crate::runtime::inventory();
        if !inventory.available {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("nested")).unwrap();
        std::fs::write(root.path().join("-- source café.txt"), "value").unwrap();
        for reversed in [false, true] {
            let mut files = vec![
                ProgramFile {
                    id: "source".into(),
                    path: "-- source café.txt".into(),
                    access: ProgramFileAccess::ReadOnly,
                },
                ProgramFile {
                    id: "result".into(),
                    path: "nested/.hidden result.txt".into(),
                    access: ProgramFileAccess::WriteOnly,
                },
            ];
            if reversed {
                files.reverse();
            }
            let mut value = proposal("from uhm_runtime import resource\nresource('result').write_path.write_text(resource('source').read_path.read_text())");
            value.files = files;
            let result = execute(Request {
                proposal: &value,
                python: &inventory,
                stdin: None,
                cwd: root.path(),
                config: &ProgramConfig::default(),
                containment: crate::containment::Mode::Off,
                retain_workspace: false,
                recovery: None,
            })
            .unwrap();
            assert_eq!(result.code, 0);
            assert_eq!(
                std::fs::read_to_string(root.path().join("nested/.hidden result.txt")).unwrap(),
                "value"
            );
        }
    }

    #[test]
    fn canonical_preflight_vectors_match_stable_codes() {
        if !crate::runtime::inventory().available {
            return;
        }
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/program-preflight-cases-v1.json"
        ))
        .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let action =
                crate::contract::decode_and_validate("run_program", case["arguments"].clone())
                    .unwrap();
            let crate::action::ProposedAction::Program { program } = action else {
                unreachable!()
            };
            let mut actual = preflight(
                &program,
                &crate::runtime::inventory(),
                case["piped_input_present"].as_bool().unwrap(),
            )
            .into_iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>();
            actual.sort();
            let mut expected = case["codes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_string())
                .collect::<Vec<_>>();
            expected.sort();
            assert_eq!(actual, expected, "{}", case["id"]);
        }
    }

    #[test]
    fn unavailable_runtime_is_a_stable_availability_diagnostic() {
        let diagnostics = preflight(
            &proposal("print('unused')"),
            &crate::runtime::PythonInventory::unavailable(),
            false,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "runtime_unavailable");
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Availability);
    }

    #[test]
    fn every_statically_declared_writable_resource_must_be_used() {
        if !crate::runtime::inventory().available {
            return;
        }
        let mut value = proposal(
            "from uhm_runtime import resource\nresource('first').write_path.write_text('x')",
        );
        value.files = vec![
            ProgramFile {
                id: "first".into(),
                path: "first.txt".into(),
                access: ProgramFileAccess::WriteOnly,
            },
            ProgramFile {
                id: "second".into(),
                path: "second.txt".into(),
                access: ProgramFileAccess::WriteOnly,
            },
        ];
        assert!(preflight(&value, &crate::runtime::inventory(), false)
            .iter()
            .any(
                |diagnostic| diagnostic.code == "write_resource_not_consumed"
                    && diagnostic.severity == DiagnosticSeverity::HardError
            ));
    }
}
