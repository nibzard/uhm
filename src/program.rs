//! Bounded Python microprogram execution. These controls reduce accidents; they are not a sandbox.

use crate::action::{ProgramInputAccess, ProgramProposal, ProgramResultMode};
use crate::config::ProgramConfig;
use crate::runtime::PythonInventory;
use serde::Serialize;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

static CHILD_GROUP: AtomicI32 = AtomicI32::new(0);
static SIGNALS: Once = Once::new();

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
    pub artifacts: Vec<PathBuf>,
    pub retained_workspace: Option<PathBuf>,
}

#[derive(Serialize)]
struct InputEnv {
    path: String,
    access: &'static str,
}

#[derive(Serialize)]
struct OutputEnv {
    path: String,
    destination: String,
}

struct StagedOutput {
    destination: PathBuf,
    staging: PathBuf,
}

impl Drop for StagedOutput {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.staging);
    }
}

pub struct Request<'a> {
    pub proposal: &'a ProgramProposal,
    pub python: &'a PythonInventory,
    pub stdin: Option<&'a [u8]>,
    pub cwd: &'a Path,
    pub config: &'a ProgramConfig,
    pub retain_workspace: bool,
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
    if !req.config.enabled {
        return Err("microprogram execution is disabled by configuration".into());
    }
    if req.proposal.source.len() > req.config.source_max_bytes
        || req.proposal.inputs.len() > req.config.input_max_paths
        || req.proposal.outputs.len() > req.config.output_max_paths
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
    let source_path = workspace.path().join("program.py");
    write_private(&source_path, req.proposal.source.as_bytes())?;

    let local_input = if req.proposal.inputs.iter().any(|v| v.path == "stdin") {
        let bytes = req
            .stdin
            .ok_or("program requested stdin, but no piped input is available")?;
        let path = workspace.path().join("input.bin");
        write_private(&path, bytes)?;
        Some(path)
    } else {
        None
    };
    let inputs = req
        .proposal
        .inputs
        .iter()
        .map(|input| {
            let path = if input.path == "stdin" {
                local_input.as_ref().expect("validated above").clone()
            } else {
                absolute(req.cwd, &input.path)
            };
            if input.path != "stdin" {
                let meta = std::fs::symlink_metadata(&path)
                    .map_err(|e| format!("program input {} is unavailable: {e}", path.display()))?;
                if meta.file_type().is_symlink() {
                    return Err(format!("program input {} is a symlink", path.display()));
                }
            }
            Ok(InputEnv {
                path: path.to_string_lossy().into_owned(),
                access: match input.access {
                    ProgramInputAccess::ReadOnly => "read_only",
                    ProgramInputAccess::Replace => "replace",
                },
            })
        })
        .collect::<std::result::Result<Vec<_>, String>>()?;
    let staged = prepare_outputs(req.cwd, &req.proposal.outputs)?;
    let outputs = staged
        .iter()
        .map(|value| OutputEnv {
            path: value.staging.to_string_lossy().into_owned(),
            destination: value.destination.to_string_lossy().into_owned(),
        })
        .collect::<Vec<_>>();

    let mut cmd = Command::new(python);
    cmd.args(fixed_arguments(&source_path));
    cmd.current_dir(workspace.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("PATH", python.parent().unwrap_or(Path::new("/usr/bin")))
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .env("TMPDIR", workspace.path())
        .env(
            "UHM_PROGRAM_INPUTS",
            serde_json::to_string(&inputs).map_err(|e| e.to_string())?,
        )
        .env(
            "UHM_PROGRAM_OUTPUTS",
            serde_json::to_string(&outputs).map_err(|e| e.to_string())?,
        );
    if let Some(path) = &local_input {
        cmd.env("UHM_PROGRAM_LOCAL_INPUT", path);
    }
    apply_limits(&mut cmd, req.config);
    let started = Instant::now();
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn isolated Python runtime: {e}"))?;
    CHILD_GROUP.store(child.id() as i32, Ordering::SeqCst);
    let total = Arc::new(AtomicUsize::new(0));
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout = read_bounded(
        child.stdout.take().expect("piped stdout"),
        total.clone(),
        overflow.clone(),
        req.config.output_max_bytes,
        req.config.diagnostic_bytes,
        true,
    );
    let stderr = read_bounded(
        child.stderr.take().expect("piped stderr"),
        total,
        overflow.clone(),
        req.config.output_max_bytes,
        req.config.diagnostic_bytes,
        false,
    );
    let mut timed_out = false;
    let status = 'wait: loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("wait for program: {e}"))?
        {
            break status;
        }
        if overflow.load(Ordering::SeqCst)
            || started.elapsed() >= Duration::from_secs(req.config.timeout_secs)
        {
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
    CHILD_GROUP.store(0, Ordering::SeqCst);
    let (stdout_bytes, stdout_tail) = stdout.join().unwrap_or_default();
    let (_, stderr_tail) = stderr.join().unwrap_or_default();
    let output_overflow = overflow.load(Ordering::SeqCst);
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
        if directory_size(workspace.path())? > req.config.workspace_max_bytes {
            code = 1;
        } else if req.proposal.result_mode == ProgramResultMode::Artifacts {
            artifacts = commit_outputs(staged, req.config.workspace_max_bytes)?;
        }
    }
    let retained_workspace = if req.retain_workspace {
        Some(workspace.into_path())
    } else {
        None
    };
    Ok(ExecutionResult {
        code,
        signal,
        stdout: stdout_bytes,
        stdout_tail,
        stderr_tail,
        timed_out,
        output_overflow,
        duration: started.elapsed(),
        artifacts,
        retained_workspace,
    })
}

fn fixed_arguments(source: &Path) -> Vec<std::ffi::OsString> {
    vec!["-I".into(), "-S".into(), source.as_os_str().to_owned()]
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

fn directory_size(root: &Path) -> Result<u64, String> {
    let mut total = 0u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(&path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let meta = entry.metadata().map_err(|e| e.to_string())?;
            if meta.is_dir() {
                pending.push(entry.path());
            } else {
                total = total.saturating_add(meta.len());
            }
        }
    }
    Ok(total)
}

fn read_bounded<R: Read + Send + 'static>(
    mut reader: R,
    total: Arc<AtomicUsize>,
    overflow: Arc<AtomicBool>,
    limit: usize,
    tail_limit: usize,
    retain: bool,
) -> std::thread::JoinHandle<(Vec<u8>, Vec<u8>)> {
    std::thread::spawn(move || {
        let retained = Arc::new(Mutex::new(Vec::new()));
        let mut tail = VecDeque::with_capacity(tail_limit);
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let before = total.fetch_add(n, Ordering::SeqCst);
                    if before.saturating_add(n) > limit {
                        overflow.store(true, Ordering::SeqCst);
                    }
                    if retain && before < limit {
                        let take = n.min(limit - before);
                        retained.lock().unwrap().extend_from_slice(&buf[..take]);
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
        let bytes = Arc::try_unwrap(retained).unwrap().into_inner().unwrap();
        (bytes, tail.into_iter().collect())
    })
}

fn apply_limits(cmd: &mut Command, config: &ProgramConfig) {
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        let cpu = config.cpu_secs;
        let address = config.address_space_bytes;
        let files = config.open_files;
        #[cfg(target_os = "linux")]
        let children = config.child_processes;
        cmd.pre_exec(move || {
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            libc::umask(0o077);
            set_limit(libc::RLIMIT_CPU, cpu)?;
            set_limit(libc::RLIMIT_AS, address)?;
            set_limit(libc::RLIMIT_NOFILE, files)?;
            #[cfg(target_os = "linux")]
            set_limit(libc::RLIMIT_NPROC, children)?;
            Ok(())
        });
    }
}

#[cfg(unix)]
fn set_limit(resource: libc::__rlimit_resource_t, value: u64) -> std::io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: value as libc::rlim_t,
        rlim_max: value as libc::rlim_t,
    };
    if unsafe { libc::setrlimit(resource, &limit) } == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
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
    use crate::action::{Effect, ProgramRuntime};

    fn proposal(source: &str) -> ProgramProposal {
        ProgramProposal {
            runtime: ProgramRuntime::Python3,
            source: source.into(),
            summary: "test".into(),
            assumptions: vec![],
            inputs: vec![],
            outputs: vec![],
            effects: vec![Effect::ReadLocal],
            result_mode: ProgramResultMode::Stdout,
        }
    }

    fn run_stdin(source: &str, input: &[u8]) -> Vec<u8> {
        let inventory = crate::runtime::inventory();
        if !inventory.available {
            return Vec::new();
        }
        let cwd = std::env::current_dir().unwrap();
        let mut value = proposal(source);
        value.inputs.push(crate::action::ProgramInput {
            path: "stdin".into(),
            access: ProgramInputAccess::ReadOnly,
        });
        let result = execute(Request {
            proposal: &value,
            python: &inventory,
            stdin: Some(input),
            cwd: &cwd,
            config: &ProgramConfig::default(),
            retain_workspace: false,
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
        let cwd = std::env::current_dir().unwrap();
        let result = execute(Request {
            proposal: &proposal(
                "import os\nprint(os.environ.get('UHM_PROGRAM_TEST_SECRET', 'stripped'))\nprint(os.environ.get('OPENAI_API_KEY', 'stripped'))",
            ),
            python: &inventory,
            stdin: None,
            cwd: &cwd,
            config: &ProgramConfig::default(),
            retain_workspace: false,
        })
        .unwrap();
        std::env::remove_var("UHM_PROGRAM_TEST_SECRET");
        std::env::remove_var("OPENAI_API_KEY");
        assert_eq!(result.code, 0);
        assert_eq!(result.stdout, b"stripped\nstripped\n");
    }

    #[test]
    fn interpreter_arguments_are_fixed_and_source_is_one_opaque_argument() {
        let path = Path::new("/tmp/source with spaces;$(nope).py");
        assert_eq!(
            fixed_arguments(path),
            vec![
                std::ffi::OsString::from("-I"),
                std::ffi::OsString::from("-S"),
                path.as_os_str().to_owned()
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
            retain_workspace: false,
        })
        .unwrap();
        assert!(result.output_overflow);
        assert_ne!(result.code, 0);
    }

    #[test]
    fn representative_read_only_transformations_return_results() {
        let cases: &[(&str, &[u8], &[u8])] = &[
            ("import os,re\np=open(os.environ['UHM_PROGRAM_LOCAL_INPUT']).read()\nprint(len([x for x in re.split(r'\\n\\s*\\n',p.strip()) if x]))", b"one\n\ntwo\nline\n\nthree\n", b"3\n"),
            ("import os,re\ns=open(os.environ['UHM_PROGRAM_LOCAL_INPUT']).read()\nprint(len(re.findall(r'(?i)\\bworld\\b',s)))", b"World, world! otherworld", b"2\n"),
            ("import os,json\ns=open(os.environ['UHM_PROGRAM_LOCAL_INPUT']).read()\nw=s.split()\nprint(json.dumps({'characters':len(s),'lines':len(s.splitlines()),'words':len(w)},sort_keys=True))", b"one two\nthree\n", b"{\"characters\": 14, \"lines\": 2, \"words\": 3}\n"),
            ("import os\ns=open(os.environ['UHM_PROGRAM_LOCAL_INPUT']).read()\nprint(s.split('BEGIN',1)[1].split('END',1)[0].strip())", b"no BEGIN wanted text END no", b"wanted text\n"),
            ("import os,json\np=os.environ['UHM_PROGRAM_LOCAL_INPUT']\nfor line in open(p):\n o=json.loads(line)\n if o['active']: print(o['name'])", b"{\"name\":\"Ada\",\"active\":true}\n{\"name\":\"Lin\",\"active\":false}\n", b"Ada\n"),
            ("import os,csv\np=os.environ['UHM_PROGRAM_LOCAL_INPUT']\nprint(sum(int(r['amount']) for r in csv.DictReader(open(p))))", b"name,amount\na,2\nb,5\n", b"7\n"),
            ("import os\np=os.environ['UHM_PROGRAM_LOCAL_INPUT']\nprint('\\n'.join(dict.fromkeys(open(p).read().splitlines())))", b"a\nb\na\nc\nb\n", b"a\nb\nc\n"),
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
        let source = "import os,json\no=json.loads(os.environ['UHM_PROGRAM_OUTPUTS'])[0]\nopen(o['path'],'w').write('replacement')";
        let mut value = proposal(source);
        value.outputs = vec!["result.txt".into()];
        value.result_mode = ProgramResultMode::Artifacts;
        value.effects = vec![Effect::WriteLocal];
        let result = execute(Request {
            proposal: &value,
            python: &inventory,
            stdin: None,
            cwd: root.path(),
            config: &ProgramConfig::default(),
            retain_workspace: false,
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
            retain_workspace: false,
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
    fn concatenates_multiple_declared_files_in_manifest_order() {
        let inventory = crate::runtime::inventory();
        if !inventory.available {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("one.txt"), "one\n").unwrap();
        std::fs::write(root.path().join("two.txt"), "two\n").unwrap();
        let mut value = proposal("import os,json\nfor i in json.loads(os.environ['UHM_PROGRAM_INPUTS']):\n print(open(i['path']).read(),end='')");
        value.inputs = ["one.txt", "two.txt"]
            .into_iter()
            .map(|path| crate::action::ProgramInput {
                path: path.into(),
                access: ProgramInputAccess::ReadOnly,
            })
            .collect();
        let result = execute(Request {
            proposal: &value,
            python: &inventory,
            stdin: None,
            cwd: root.path(),
            config: &ProgramConfig::default(),
            retain_workspace: false,
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
            retain_workspace: false,
        })
        .unwrap();
        assert_eq!(result.signal, Some(libc::SIGTERM));
        assert_ne!(result.code, 0);
    }
}
