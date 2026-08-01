//! Optional, invocation-scoped parent-shell control protocol.

use crate::action::{ParentAction, ParentActionKind};
use crate::config::Config;
use crate::dirs;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const PROTOCOL_VERSION: u8 = 1;
pub const INTEGRATION_FAILURE: i32 = 15;
const REQUEST: &str = "request.json";
const RESPONSE: &str = "response.json";
const RESPONSE_TMP: &str = "response.tmp";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellFamily {
    Bash,
    Zsh,
    Fish,
}

impl ShellFamily {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "bash" => Ok(Self::Bash),
            "zsh" => Ok(Self::Zsh),
            "fish" => Ok(Self::Fish),
            _ => Err("supported integration shells are bash, zsh, and fish".into()),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    protocol_version: u8,
    nonce: String,
    shell: ShellFamily,
    parent_cwd: String,
    previous_status: i32,
    created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub protocol_version: u8,
    pub nonce: String,
    pub run_id: String,
    pub action: ParentAction,
}

#[derive(Debug)]
pub struct Session {
    root: PathBuf,
    dir: PathBuf,
    handle: std::fs::File,
    request: Request,
}

impl Session {
    pub fn shell(&self) -> ShellFamily {
        self.request.shell
    }
    pub fn parent_cwd(&self) -> &str {
        &self.request.parent_cwd
    }
    pub fn previous_status(&self) -> i32 {
        self.request.previous_status
    }
    pub fn write_response(&self, run_id: &str, action: &ParentAction) -> Result<(), String> {
        self.revalidate()?;
        action.validate()?;
        if !run_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || !(8..=64).contains(&run_id.len())
        {
            return Err("invalid integration run ID".into());
        }
        if fixed_exists(&self.handle, RESPONSE)? || fixed_exists(&self.handle, RESPONSE_TMP)? {
            return Err("integration response already exists".into());
        }
        let response = Response {
            protocol_version: PROTOCOL_VERSION,
            nonce: self.request.nonce.clone(),
            run_id: run_id.into(),
            action: action.clone(),
        };
        let bytes = serde_json::to_vec(&response).map_err(|e| e.to_string())?;
        write_fixed_new(&self.handle, RESPONSE_TMP, &bytes)?;
        link_fixed(&self.handle, RESPONSE_TMP, RESPONSE)?;
        unlink_fixed(&self.handle, RESPONSE_TMP, false)?;
        validate_fixed_file(&self.handle, RESPONSE)?;
        Ok(())
    }
    fn revalidate(&self) -> Result<(), String> {
        let current = validate_dir(&self.root, &self.dir)?;
        if !same_file(&self.handle, &current)? {
            return Err("integration directory changed after validation".into());
        }
        let request = read_request(&self.handle)?;
        if request.nonce != self.request.nonce {
            return Err("integration nonce changed".into());
        }
        Ok(())
    }
}

pub fn template(shell: ShellFamily) -> &'static str {
    match shell {
        ShellFamily::Bash => include_str!("../assets/shell/uhm.bash"),
        ShellFamily::Zsh => include_str!("../assets/shell/uhm.zsh"),
        ShellFamily::Fish => include_str!("../assets/shell/uhm.fish"),
    }
}

fn runtime_root(config: &Config) -> Result<PathBuf, String> {
    if !config.paths.data_dir.is_absolute() {
        return Err("integration runtime root must be absolute".into());
    }
    dirs::ensure_private_dir(&config.paths.data_dir)?;
    validate_owned_dir(&config.paths.data_dir)?;
    let root = config.paths.data_dir.join("shell-runtime");
    if root.exists()
        && std::fs::symlink_metadata(&root)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
    {
        return Err("integration runtime root must not be a symlink".into());
    }
    dirs::ensure_private_dir(&root)?;
    validate_owned_dir(&root)?;
    Ok(root)
}

pub fn open(
    config: &Config,
    shell: ShellFamily,
    cwd: &str,
    previous_status: i32,
) -> Result<(PathBuf, String), String> {
    if cwd.is_empty() || cwd.len() > 4096 || cwd.contains('\0') || !Path::new(cwd).is_absolute() {
        return Err("parent working directory must be an absolute path up to 4096 bytes".into());
    }
    if !(0..=255).contains(&previous_status) {
        return Err("previous shell status must be between 0 and 255".into());
    }
    let root = runtime_root(config)?;
    for _ in 0..8 {
        let nonce = nonce()?;
        let dir = root.join(format!("invocation-{}", &nonce[..24]));
        match std::fs::create_dir(&dir) {
            Ok(()) => {
                dirs::ensure_private_dir(&dir)?;
                let handle = validate_dir(&root, &dir)?;
                let request = Request {
                    protocol_version: PROTOCOL_VERSION,
                    nonce: nonce.clone(),
                    shell,
                    parent_cwd: cwd.into(),
                    previous_status,
                    created_at: crate::history::now_secs(),
                };
                write_fixed_new(
                    &handle,
                    REQUEST,
                    &serde_json::to_vec(&request).map_err(|e| e.to_string())?,
                )?;
                return Ok((dir, nonce));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("create integration directory: {}", e)),
        }
    }
    Err("could not allocate a unique integration directory".into())
}

pub fn load(config: &Config, dir: &Path, nonce_value: &str) -> Result<Session, String> {
    validate_nonce(nonce_value)?;
    let root = runtime_root(config)?;
    let handle = validate_dir(&root, dir)?;
    let request = read_request(&handle)?;
    if request.protocol_version != PROTOCOL_VERSION {
        return Err("unsupported integration protocol version".into());
    }
    if request.nonce != nonce_value {
        return Err("integration nonce mismatch".into());
    }
    if crate::history::now_secs().saturating_sub(request.created_at) > 600 {
        return Err("integration request is stale".into());
    }
    Ok(Session {
        root,
        dir: dir.into(),
        handle,
        request,
    })
}

pub fn validate_response(
    config: &Config,
    dir: &Path,
    nonce_value: &str,
    shell: ShellFamily,
) -> Result<Response, String> {
    let session = load(config, dir, nonce_value)?;
    if session.shell() != shell {
        return Err("integration shell mismatch".into());
    }
    validate_fixed_file(&session.handle, RESPONSE)?;
    let bytes = read_fixed_bounded(&session.handle, RESPONSE, 24 * 1024)?;
    let response: Response = serde_json::from_slice(&bytes)
        .map_err(|e| format!("invalid integration response: {}", e))?;
    if response.protocol_version != PROTOCOL_VERSION || response.nonce != nonce_value {
        return Err("integration response version or nonce mismatch".into());
    }
    if !response
        .run_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || !(8..=64).contains(&response.run_id.len())
    {
        return Err("invalid integration response run ID".into());
    }
    response.action.validate()?;
    Ok(response)
}

pub fn render(action: &ParentAction, shell: ShellFamily) -> Result<String, String> {
    action.validate()?;
    let path = || quote(action.path.as_deref().unwrap_or(""), shell);
    let value = || quote(action.value.as_deref().unwrap_or(""), shell);
    Ok(match (shell, action.kind) {
        (ShellFamily::Bash | ShellFamily::Zsh, ParentActionKind::ChangeDirectory) => {
            format!("builtin cd -- {}", path())
        }
        (ShellFamily::Bash | ShellFamily::Zsh, ParentActionKind::SetEnvironment) => format!(
            "export {}={}",
            action.name.as_deref().unwrap_or(""),
            value()
        ),
        (ShellFamily::Bash | ShellFamily::Zsh, ParentActionKind::UnsetEnvironment) => {
            format!("unset {}", action.name.as_deref().unwrap_or(""))
        }
        (ShellFamily::Bash | ShellFamily::Zsh, ParentActionKind::SourceFile) => {
            format!("builtin source -- {}", path())
        }
        (ShellFamily::Fish, ParentActionKind::ChangeDirectory) => {
            format!("builtin cd -- {}", path())
        }
        (ShellFamily::Fish, ParentActionKind::SetEnvironment) => format!(
            "set -gx {} {}",
            action.name.as_deref().unwrap_or(""),
            value()
        ),
        (ShellFamily::Fish, ParentActionKind::UnsetEnvironment) => {
            format!("set -e {}", action.name.as_deref().unwrap_or(""))
        }
        (ShellFamily::Fish, ParentActionKind::SourceFile) => format!("source {}", path()),
    })
}

pub fn fallback(action: &ParentAction, shell_name: &str) -> Result<String, String> {
    let name = Path::new(shell_name)
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or(shell_name);
    render(
        action,
        ShellFamily::parse(name).unwrap_or(ShellFamily::Bash),
    )
}

pub fn clean(config: &Config, dir: &Path, nonce_value: &str) -> Result<(), String> {
    let session = load(config, dir, nonce_value)?;
    for name in [RESPONSE, RESPONSE_TMP, REQUEST] {
        unlink_fixed(&session.handle, name, true)?;
    }
    drop(session);
    std::fs::remove_dir(dir).map_err(|e| format!("clean integration directory: {}", e))
}

fn quote(value: &str, shell: ShellFamily) -> String {
    match shell {
        ShellFamily::Bash | ShellFamily::Zsh => format!("'{}'", value.replace('\'', "'\\''")),
        ShellFamily::Fish => format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'")),
    }
}

fn nonce() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| format!("read system randomness: {}", e))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn validate_nonce(value: &str) -> Result<(), String> {
    if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("integration nonce must be 64 hexadecimal characters".into())
    }
}

#[cfg(unix)]
fn validate_owned_dir(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_dir()
        || meta.file_type().is_symlink()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.permissions().mode() & 0o777 != 0o700
        || meta.nlink() < 2
    {
        return Err("integration directory ownership, type, links, or mode is invalid".into());
    }
    Ok(())
}
#[cfg(unix)]
#[cfg(test)]
fn validate_owned_file(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file()
        || meta.file_type().is_symlink()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.permissions().mode() & 0o777 != 0o600
        || meta.nlink() != 1
    {
        return Err("integration file ownership, type, links, or mode is invalid".into());
    }
    Ok(())
}

#[cfg(unix)]
fn validate_owned_dir_handle(file: &std::fs::File) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_dir()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.permissions().mode() & 0o777 != 0o700
        || meta.nlink() < 2
    {
        return Err(
            "integration directory handle ownership, type, links, or mode is invalid".into(),
        );
    }
    Ok(())
}

#[cfg(unix)]
fn validate_owned_file_handle(file: &std::fs::File) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.permissions().mode() & 0o777 != 0o600
        || meta.nlink() != 1
    {
        return Err("integration file handle ownership, type, links, or mode is invalid".into());
    }
    Ok(())
}

#[cfg(unix)]
fn validate_dir(root: &Path, dir: &Path) -> Result<std::fs::File, String> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    validate_owned_dir(root)?;
    if !dir.is_absolute() {
        return Err("integration directory must be absolute".into());
    }
    validate_owned_dir(dir)?;
    let root_real = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    let dir_real = std::fs::canonicalize(dir).map_err(|e| e.to_string())?;
    if dir_real.parent() != Some(root_real.as_path())
        || !dir_real
            .file_name()
            .and_then(|v| v.to_str())
            .is_some_and(|v| v.starts_with("invocation-"))
    {
        return Err("integration directory is outside the validated runtime root".into());
    }
    let handle = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(dir)
        .map_err(|e| format!("open integration directory without following links: {}", e))?;
    validate_owned_dir_handle(&handle)?;
    let path_meta = std::fs::symlink_metadata(dir).map_err(|e| e.to_string())?;
    let handle_meta = handle.metadata().map_err(|e| e.to_string())?;
    if path_meta.dev() != handle_meta.dev() || path_meta.ino() != handle_meta.ino() {
        return Err("integration directory changed while it was opened".into());
    }
    Ok(handle)
}

#[cfg(unix)]
fn same_file(a: &std::fs::File, b: &std::fs::File) -> Result<bool, String> {
    use std::os::unix::fs::MetadataExt;
    let a = a.metadata().map_err(|e| e.to_string())?;
    let b = b.metadata().map_err(|e| e.to_string())?;
    Ok(a.dev() == b.dev() && a.ino() == b.ino())
}

fn read_request(dir: &std::fs::File) -> Result<Request, String> {
    let bytes = read_fixed_bounded(dir, REQUEST, 8192)?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid integration request: {}", e))
}

#[cfg(unix)]
fn fixed_name(name: &str) -> Result<std::ffi::CString, String> {
    if !matches!(name, REQUEST | RESPONSE | RESPONSE_TMP) {
        return Err("invalid integration control filename".into());
    }
    std::ffi::CString::new(name).map_err(|_| "invalid integration control filename".into())
}

#[cfg(unix)]
fn open_fixed_read(dir: &std::fs::File, name: &str) -> Result<std::fs::File, String> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let name = fixed_name(name)?;
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(format!(
            "open integration control file: {}",
            std::io::Error::last_os_error()
        ));
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    validate_owned_file_handle(&file)?;
    Ok(file)
}

#[cfg(unix)]
fn validate_fixed_file(dir: &std::fs::File, name: &str) -> Result<(), String> {
    open_fixed_read(dir, name).map(|_| ())
}

#[cfg(unix)]
fn read_fixed_bounded(dir: &std::fs::File, name: &str, max: usize) -> Result<Vec<u8>, String> {
    let file = open_fixed_read(dir, name)?;
    read_bounded_file(file, max)
}

fn read_bounded_file(file: std::fs::File, max: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    file.take((max + 1) as u64)
        .read_to_end(&mut out)
        .map_err(|e| e.to_string())?;
    if out.len() > max {
        return Err("integration control file is oversized".into());
    }
    Ok(out)
}

#[cfg(unix)]
fn write_fixed_new(dir: &std::fs::File, name: &str, bytes: &[u8]) -> Result<(), String> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let name = fixed_name(name)?;
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(format!(
            "create integration control file: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let chmod = unsafe { libc::fchmod(file.as_raw_fd(), 0o600) };
    if chmod != 0 {
        return Err(format!(
            "set integration control permissions: {}",
            std::io::Error::last_os_error()
        ));
    }
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    validate_owned_file_handle(&file)
}

#[cfg(unix)]
fn fixed_exists(dir: &std::fs::File, name: &str) -> Result<bool, String> {
    use std::os::fd::AsRawFd;
    let name = fixed_name(name)?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::NotFound {
        Ok(false)
    } else {
        Err(format!("inspect integration control file: {}", error))
    }
}

#[cfg(unix)]
fn link_fixed(dir: &std::fs::File, source: &str, destination: &str) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    let source = fixed_name(source)?;
    let destination = fixed_name(destination)?;
    let result = unsafe {
        libc::linkat(
            dir.as_raw_fd(),
            source.as_ptr(),
            dir.as_raw_fd(),
            destination.as_ptr(),
            0,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(format!(
            "publish exclusive integration response: {}",
            std::io::Error::last_os_error()
        ))
    }
}

#[cfg(unix)]
fn unlink_fixed(dir: &std::fs::File, name: &str, allow_missing: bool) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    let name = fixed_name(name)?;
    let result = unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if allow_missing && error.kind() == std::io::ErrorKind::NotFound {
        Ok(())
    } else {
        Err(format!("remove integration control file: {}", error))
    }
}
#[cfg(test)]
fn write_new_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("create integration file: {}", e))?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    validate_owned_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dirs::Paths;
    fn config(root: &Path) -> Config {
        Config::test(Paths {
            config_file: root.join("config"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
        })
    }
    fn action(
        kind: ParentActionKind,
        path: Option<&str>,
        name: Option<&str>,
        value: Option<&str>,
    ) -> ParentAction {
        ParentAction {
            kind,
            path: path.map(str::to_owned),
            name: name.map(str::to_owned),
            value: value.map(str::to_owned),
        }
    }
    #[test]
    fn templates_are_static_and_guard_recursion() {
        for shell in [ShellFamily::Bash, ShellFamily::Zsh, ShellFamily::Fish] {
            let text = template(shell);
            assert!(text.contains("integration v1"));
            assert!(text.contains("__uhm_binary"));
            assert!(!text.contains("curl"));
        }
    }
    #[test]
    fn installed_shells_accept_their_template_syntax() {
        let d = tempfile::tempdir().unwrap();
        for (shell, flag, family) in [
            ("bash", "-n", ShellFamily::Bash),
            ("zsh", "-n", ShellFamily::Zsh),
            ("fish", "-n", ShellFamily::Fish),
        ] {
            if std::process::Command::new(shell)
                .arg("--version")
                .output()
                .is_err()
            {
                continue;
            }
            let path = d.path().join(format!("init.{}", family.as_str()));
            std::fs::write(&path, template(family)).unwrap();
            assert!(std::process::Command::new(shell)
                .args([flag, path.to_str().unwrap()])
                .status()
                .unwrap()
                .success());
        }
    }
    #[cfg(unix)]
    #[test]
    fn every_installed_wrapper_applies_in_its_parent_shell() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let fake = d.path().join("uhm");
        std::fs::write(
            &fake,
            r##"#!/bin/sh
case "$*" in
*shell-history-enabled*) exit 1;;
*shell-control-open*) p=$(mktemp -d); printf '%s\t%s\n' "$p" nonce; exit 0;;
*shell-validate*)
  case "$*" in
    *--uhm-shell\ fish*) printf '%s\n' "set -gx UHM_APPLIED 'fish'";;
    *--uhm-shell\ zsh*) printf '%s\n' "export UHM_APPLIED='zsh'";;
    *) printf '%s\n' "export UHM_APPLIED='bash'";;
  esac
  exit 0;;
*shell-ack*) exit 0;;
*shell-clean*)
  previous=""
  for value in "$@"; do
    if [ "$previous" = dir ]; then rm -f "$value/response.json"; rmdir "$value"; exit $?; fi
    if [ "$value" = --uhm-control-dir ]; then previous=dir; else previous=""; fi
  done
  exit 1;;
*)
  previous=""
  for value in "$@"; do
    if [ "$previous" = dir ]; then touch "$value/response.json"; break; fi
    if [ "$value" = --uhm-control-dir ]; then previous=dir; else previous=""; fi
  done
  exit 0;;
esac
"##,
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        for (program, family, prefix) in [
            ("bash", ShellFamily::Bash, vec!["--noprofile", "--norc"]),
            ("zsh", ShellFamily::Zsh, vec!["-f"]),
            ("fish", ShellFamily::Fish, vec!["--no-config"]),
        ] {
            if std::process::Command::new(program)
                .arg("--version")
                .output()
                .is_err()
            {
                continue;
            }
            let init = d.path().join(format!("wrapper.{}", family.as_str()));
            std::fs::write(&init, template(family)).unwrap();
            let mut command = std::process::Command::new(program);
            command.args(prefix).args([
                "-c",
                "source \"$UHM_INIT\"; uhm anything; printf '%s' \"$UHM_APPLIED\"",
            ]);
            let output = command
                .env("UHM_INIT", init)
                .env(
                    "PATH",
                    format!(
                        "{}:{}",
                        d.path().display(),
                        std::env::var("PATH").unwrap_or_default()
                    ),
                )
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}: {}",
                program,
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(output.stdout, family.as_str().as_bytes(), "{program}");
        }
    }
    #[test]
    fn protocol_round_trip_and_nonce_binding() {
        let d = tempfile::tempdir().unwrap();
        let c = config(d.path());
        let (dir, nonce) = open(&c, ShellFamily::Bash, "/tmp", 7).unwrap();
        let session = load(&c, &dir, &nonce).unwrap();
        assert_eq!(session.previous_status(), 7);
        assert_eq!(session.parent_cwd(), "/tmp");
        let a = action(
            ParentActionKind::ChangeDirectory,
            Some("/tmp/space here"),
            None,
            None,
        );
        session.write_response("abcdefgh1234", &a).unwrap();
        let r = validate_response(&c, &dir, &nonce, ShellFamily::Bash).unwrap();
        assert_eq!(r.action, a);
        assert!(validate_response(&c, &dir, &"0".repeat(64), ShellFamily::Bash).is_err());
        clean(&c, &dir, &nonce).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn an_open_session_rejects_directory_replacement_and_replay() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let c = config(d.path());
        let (dir, nonce) = open(&c, ShellFamily::Bash, "/tmp", 0).unwrap();
        let session = load(&c, &dir, &nonce).unwrap();
        let request = std::fs::read(dir.join(REQUEST)).unwrap();
        let moved = dir.parent().unwrap().join("invocation-moved-aside");
        std::fs::rename(&dir, &moved).unwrap();
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        write_new_private(&dir.join(REQUEST), &request).unwrap();
        let action = action(ParentActionKind::ChangeDirectory, Some("/tmp"), None, None);
        assert!(session.write_response("abcdefgh1234", &action).is_err());
    }
    #[test]
    fn field_matrix_and_environment_names_are_strict() {
        assert!(action(
            ParentActionKind::SetEnvironment,
            None,
            Some("GOOD_2"),
            Some("x")
        )
        .validate()
        .is_ok());
        assert!(action(
            ParentActionKind::SetEnvironment,
            Some("bad"),
            Some("A"),
            Some("x")
        )
        .validate()
        .is_err());
        assert!(
            action(ParentActionKind::UnsetEnvironment, None, Some("2BAD"), None)
                .validate()
                .is_err()
        );
    }
    #[test]
    fn renderer_quotes_metacharacters_as_one_operand() {
        let a = action(
            ParentActionKind::ChangeDirectory,
            Some("-a b'$();\n雪"),
            None,
            None,
        );
        let rendered = render(&a, ShellFamily::Bash).unwrap();
        assert_eq!(rendered, "builtin cd -- '-a b'\\''$();\n雪'");
        assert!(!rendered.contains("eval"));
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_and_loose_permissions() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let d = tempfile::tempdir().unwrap();
        let c = config(d.path());
        let (dir, nonce) = open(&c, ShellFamily::Bash, "/tmp", 0).unwrap();
        std::fs::set_permissions(dir.join(REQUEST), std::fs::Permissions::from_mode(0o644))
            .unwrap();
        assert!(load(&c, &dir, &nonce).is_err());
        let fake = d.path().join("fake");
        symlink(&dir, &fake).unwrap();
        assert!(load(&c, &fake, &nonce).is_err());
    }
    #[test]
    fn concurrent_invocations_are_isolated() {
        let d = tempfile::tempdir().unwrap();
        let c = config(d.path());
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..32 {
            let pair = open(&c, ShellFamily::Bash, "/tmp", 0).unwrap();
            assert!(seen.insert(pair));
        }
    }
    #[test]
    fn stale_requests_fail_closed() {
        let d = tempfile::tempdir().unwrap();
        let c = config(d.path());
        let (dir, nonce) = open(&c, ShellFamily::Bash, "/tmp", 0).unwrap();
        let mut request: Request =
            serde_json::from_slice(&std::fs::read(dir.join(REQUEST)).unwrap()).unwrap();
        request.created_at = 0;
        std::fs::remove_file(dir.join(REQUEST)).unwrap();
        write_new_private(&dir.join(REQUEST), &serde_json::to_vec(&request).unwrap()).unwrap();
        assert!(load(&c, &dir, &nonce).is_err());
    }
    #[test]
    fn malformed_unknown_compound_and_oversized_responses_fail_closed() {
        let d = tempfile::tempdir().unwrap();
        let c = config(d.path());
        let invalid = [
            serde_json::json!({"protocol_version":99,"nonce":"NONCE","run_id":"abcdefgh1234","action":{"kind":"change_directory","path":"/tmp","name":null,"value":null}}),
            serde_json::json!({"protocol_version":1,"nonce":"NONCE","run_id":"abcdefgh1234","action":{"kind":"exit","path":null,"name":null,"value":null}}),
            serde_json::json!({"protocol_version":1,"nonce":"NONCE","run_id":"abcdefgh1234","action":[{"kind":"change_directory","path":"/tmp","name":null,"value":null},{"kind":"unset_environment","path":null,"name":"A","value":null}]}),
            serde_json::json!({"protocol_version":1,"nonce":"NONCE","run_id":"abcdefgh1234","output":"/tmp/chosen","action":{"kind":"change_directory","path":"/tmp","name":null,"value":null}}),
        ];
        for mut value in invalid {
            let (dir, nonce) = open(&c, ShellFamily::Bash, "/tmp", 0).unwrap();
            value["nonce"] = serde_json::json!(nonce);
            write_new_private(&dir.join(RESPONSE), &serde_json::to_vec(&value).unwrap()).unwrap();
            assert!(validate_response(&c, &dir, &nonce, ShellFamily::Bash).is_err());
        }
        let (dir, nonce) = open(&c, ShellFamily::Bash, "/tmp", 0).unwrap();
        write_new_private(&dir.join(RESPONSE), &vec![b'x'; 24 * 1024 + 1]).unwrap();
        assert!(validate_response(&c, &dir, &nonce, ShellFamily::Bash).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn hard_links_traversal_and_outside_roots_are_rejected() {
        let d = tempfile::tempdir().unwrap();
        let c = config(d.path());
        let (dir, nonce) = open(&c, ShellFamily::Bash, "/tmp", 0).unwrap();
        std::fs::hard_link(dir.join(REQUEST), dir.join("request-copy")).unwrap();
        assert!(load(&c, &dir, &nonce).is_err());
        assert!(load(&c, d.path(), &nonce).is_err());
        assert!(load(&c, &dir.join(".."), &nonce).is_err());
    }
    #[test]
    fn bash_renderer_round_trips_an_adversarial_directory() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("space '$(); 雪\nline");
        std::fs::create_dir(&path).unwrap();
        let value = action(
            ParentActionKind::ChangeDirectory,
            Some(path.to_str().unwrap()),
            None,
            None,
        );
        let output = std::process::Command::new("bash")
            .args([
                "--noprofile",
                "--norc",
                "-c",
                "eval \"$UHM_CODE\"; printf '%s' \"$PWD\"",
            ])
            .env("UHM_CODE", render(&value, ShellFamily::Bash).unwrap())
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            PathBuf::from(String::from_utf8(output.stdout).unwrap())
                .canonicalize()
                .unwrap(),
            path.canonicalize().unwrap()
        );
    }
    #[test]
    fn every_installed_shell_renderer_round_trips_a_quoted_directory() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("- space '$(); 雪\nline");
        std::fs::create_dir(&path).unwrap();
        let action = action(
            ParentActionKind::ChangeDirectory,
            Some(path.to_str().unwrap()),
            None,
            None,
        );
        for (program, family, script) in [
            (
                "bash",
                ShellFamily::Bash,
                "eval \"$UHM_CODE\"; printf '%s' \"$PWD\"",
            ),
            (
                "zsh",
                ShellFamily::Zsh,
                "eval \"$UHM_CODE\"; printf '%s' \"$PWD\"",
            ),
            (
                "fish",
                ShellFamily::Fish,
                "eval \"$UHM_CODE\"; printf '%s' \"$PWD\"",
            ),
        ] {
            if std::process::Command::new(program)
                .arg("--version")
                .output()
                .is_err()
            {
                continue;
            }
            let output = std::process::Command::new(program)
                .args(["-c", script])
                .env("UHM_CODE", render(&action, family).unwrap())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}: {}",
                program,
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                PathBuf::from(String::from_utf8(output.stdout).unwrap())
                    .canonicalize()
                    .unwrap(),
                path.canonicalize().unwrap()
            );
        }
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn bash_parent_change_persists_through_a_real_pty_fixture() {
        if std::process::Command::new("script")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("pty directory");
        std::fs::create_dir(&path).unwrap();
        let action = action(
            ParentActionKind::ChangeDirectory,
            Some(path.to_str().unwrap()),
            None,
            None,
        );
        let output = std::process::Command::new("script")
            .args([
                "-qefc",
                "bash --noprofile --norc -c 'eval \"$UHM_CODE\"; printf \"%s\" \"$PWD\"'",
                "/dev/null",
            ])
            .env("UHM_CODE", render(&action, ShellFamily::Bash).unwrap())
            .env("SSH_CONNECTION", "fixture 0 fixture 0")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains(path.to_str().unwrap()));
    }
    #[test]
    fn bash_parent_change_persists_in_a_representative_tmux_session() {
        if std::process::Command::new("tmux")
            .arg("-V")
            .output()
            .is_err()
        {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("tmux directory");
        std::fs::create_dir(&path).unwrap();
        let output_path = d.path().join("result");
        let script = d.path().join("fixture.sh");
        let action = action(
            ParentActionKind::ChangeDirectory,
            Some(path.to_str().unwrap()),
            None,
            None,
        );
        std::fs::write(
            &script,
            format!(
                "{}\nprintf '%s' \"$PWD\" > '{}'\n",
                render(&action, ShellFamily::Bash).unwrap(),
                output_path.display()
            ),
        )
        .unwrap();
        let socket = format!("uhm-test-{}", std::process::id());
        let status = std::process::Command::new("tmux")
            .args([
                "-L",
                &socket,
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "bash",
                script.to_str().unwrap(),
            ])
            .status()
            .unwrap();
        assert!(status.success());
        for _ in 0..100 {
            if output_path.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = std::process::Command::new("tmux")
            .args(["-L", &socket, "kill-server"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        assert_eq!(
            PathBuf::from(std::fs::read_to_string(output_path).unwrap())
                .canonicalize()
                .unwrap(),
            path.canonicalize().unwrap()
        );
    }
    #[test]
    fn bash_set_unset_and_source_persist_in_the_evaluating_shell() {
        let d = tempfile::tempdir().unwrap();
        let source = d.path().join("source file.sh");
        std::fs::write(&source, "UHM_SOURCED='yes value'\n").unwrap();
        let set = action(
            ParentActionKind::SetEnvironment,
            None,
            Some("UHM_VALUE"),
            Some("space '$(); 雪\nline"),
        );
        let unset = action(
            ParentActionKind::UnsetEnvironment,
            None,
            Some("UHM_GONE"),
            None,
        );
        let source_action = action(
            ParentActionKind::SourceFile,
            Some(source.to_str().unwrap()),
            None,
            None,
        );
        let script = format!(
            "{}; {}; {}; printf '%s|%s|%s' \"$UHM_VALUE\" \"${{UHM_GONE-unset}}\" \"$UHM_SOURCED\"",
            render(&set, ShellFamily::Bash).unwrap(),
            render(&unset, ShellFamily::Bash).unwrap(),
            render(&source_action, ShellFamily::Bash).unwrap()
        );
        let output = std::process::Command::new("bash")
            .args(["--noprofile", "--norc", "-c", &script])
            .env("UHM_GONE", "present")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            output.stdout,
            "space '$(); 雪\nline|unset|yes value".as_bytes()
        );
    }
    #[cfg(unix)]
    #[test]
    fn bash_template_preserves_streams_and_child_status_without_a_response() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let fake = d.path().join("uhm");
        std::fs::write(
            &fake,
            "#!/bin/sh\ncase \"$*\" in\n*shell-history-enabled*) exit 1;;\n*shell-control-open*) p=$(mktemp -d); printf '%s\\t%s\\n' \"$p\" nonce;;\n*shell-clean*) exit 0;;\n*) printf RESULT; printf UI >&2; exit 7;;\nesac\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        let init = d.path().join("init.bash");
        std::fs::write(&init, template(ShellFamily::Bash)).unwrap();
        let output = std::process::Command::new("bash")
            .args([
                "--noprofile",
                "--norc",
                "-c",
                "source \"$UHM_INIT\"; uhm anything; exit $?",
            ])
            .env("UHM_INIT", init)
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    d.path().display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(output.stdout, b"RESULT");
        assert_eq!(output.stderr, b"UI");
    }
    #[cfg(unix)]
    #[test]
    fn bash_wrapper_enforces_validation_and_application_status_precedence() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let fake = d.path().join("uhm");
        std::fs::write(&fake, r##"#!/bin/sh
case "$*" in
*shell-history-enabled*) exit 1;;
*shell-control-open*) p=$(mktemp -d); printf '%s\t%s\n' "$p" nonce; exit 0;;
*shell-validate*)
  case "$UHM_FAKE_MODE" in
    invalid|non_file) exit 1;;
    apply_fail) printf "%s\n" "builtin cd -- '/definitely/missing/uhm-path'";;
    *) printf "%s\n" "export UHM_APPLIED='ok'";;
  esac
  exit 0;;
*shell-ack*|*shell-clean*) exit 0;;
*)
  previous=""; for value in "$@"; do if [ "$previous" = dir ]; then if [ "$UHM_FAKE_MODE" = non_file ]; then mkdir "$value/response.json"; else touch "$value/response.json"; fi; break; fi; if [ "$value" = --uhm-control-dir ]; then previous=dir; else previous=""; fi; done
  exit 0;;
esac
"##).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        let init = d.path().join("init.bash");
        std::fs::write(&init, template(ShellFamily::Bash)).unwrap();
        for (mode, expected, applied) in [
            ("success", "0", "ok"),
            ("invalid", "15", ""),
            ("apply_fail", "15", ""),
            ("non_file", "15", ""),
        ] {
            let output = std::process::Command::new("bash")
                .args(["--noprofile", "--norc", "-c", "source \"$UHM_INIT\"; uhm anything; code=$?; printf '%s|%s' \"$code\" \"${UHM_APPLIED-}\""])
                .env("UHM_INIT", &init)
                .env("UHM_FAKE_MODE", mode)
                .env("PATH",format!("{}:{}",d.path().display(),std::env::var("PATH").unwrap_or_default()))
                .output().unwrap();
            assert!(output.status.success());
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                format!("{}|{}", expected, applied)
            );
        }
    }
    #[cfg(unix)]
    #[test]
    fn bash_wrapper_cleans_after_a_signaled_child() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let fake = d.path().join("uhm");
        let record = d.path().join("control-path");
        std::fs::write(&fake,r##"#!/bin/sh
case "$*" in
*shell-history-enabled*) exit 1;;
*shell-control-open*) p=$(mktemp -d); printf '%s' "$p" > "$UHM_DIR_RECORD"; printf '%s\t%s\n' "$p" nonce; exit 0;;
*shell-clean*) previous=""; for value in "$@"; do if [ "$previous" = dir ]; then rmdir "$value"; exit $?; fi; if [ "$value" = --uhm-control-dir ]; then previous=dir; else previous=""; fi; done; exit 1;;
*) kill -TERM $$;;
esac
"##).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        let init = d.path().join("init.bash");
        std::fs::write(&init, template(ShellFamily::Bash)).unwrap();
        let output = std::process::Command::new("bash")
            .args([
                "--noprofile",
                "--norc",
                "-c",
                "source \"$UHM_INIT\"; uhm anything; exit $?",
            ])
            .env("UHM_INIT", init)
            .env("UHM_DIR_RECORD", &record)
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    d.path().display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(143));
        let control = std::fs::read_to_string(record).unwrap();
        assert!(!Path::new(&control).exists());
    }

    #[cfg(unix)]
    #[test]
    fn source_that_terminates_the_shell_cannot_be_acknowledged_or_cleaned() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let fake = d.path().join("uhm");
        let source = d.path().join("terminate.sh");
        let after_source = d.path().join("ack-or-clean-ran");
        std::fs::write(&source, "exit 23\n").unwrap();
        std::fs::write(
            &fake,
            r##"#!/bin/sh
case "$*" in
*shell-history-enabled*) exit 1;;
*shell-control-open*) p=$(mktemp -d); printf '%s\t%s\n' "$p" nonce; exit 0;;
*shell-validate*) printf '%s\n' 'builtin source -- "$UHM_EXIT_SOURCE"'; exit 0;;
*shell-ack*|*shell-clean*) touch "$UHM_AFTER_SOURCE"; exit 0;;
*)
  previous=""
  for value in "$@"; do
    if [ "$previous" = dir ]; then touch "$value/response.json"; break; fi
    if [ "$value" = --uhm-control-dir ]; then previous=dir; else previous=""; fi
  done
  exit 0;;
esac
"##,
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        let init = d.path().join("init.bash");
        std::fs::write(&init, template(ShellFamily::Bash)).unwrap();
        let output = std::process::Command::new("bash")
            .args([
                "--noprofile",
                "--norc",
                "-c",
                "source \"$UHM_INIT\"; uhm anything; exit $?",
            ])
            .env("UHM_INIT", init)
            .env("UHM_EXIT_SOURCE", source)
            .env("UHM_AFTER_SOURCE", &after_source)
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    d.path().display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(23));
        assert!(!after_source.exists());
    }
}
