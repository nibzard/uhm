//! Small, bounded Python runtime inventory used by the action router and executor.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PythonInventory {
    pub available: bool,
    pub resolved_path: Option<String>,
    pub version: Option<String>,
    pub isolated_no_site: bool,
}

impl PythonInventory {
    pub fn unavailable() -> Self {
        Self {
            available: false,
            resolved_path: None,
            version: None,
            isolated_no_site: false,
        }
    }

    pub fn path(&self) -> Result<&Path, String> {
        if !self.available || !self.isolated_no_site {
            return Err(
                "Python 3 with isolated/no-site mode is unavailable; install python3 and verify `python3 -I -S`"
                    .into(),
            );
        }
        self.resolved_path
            .as_deref()
            .map(Path::new)
            .ok_or_else(|| "Python 3 runtime path is unavailable".into())
    }
}

pub fn inventory() -> PythonInventory {
    inventory_from(std::env::var_os("PATH").as_deref())
}

fn inventory_from(path: Option<&std::ffi::OsStr>) -> PythonInventory {
    let Some(resolved) = resolve_python(path) else {
        return PythonInventory::unavailable();
    };
    let mut command = Command::new(&resolved);
    command
        .args([
            "-I",
            "-S",
            "-c",
            "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}.{sys.version_info.micro}')",
        ])
        .env_clear()
        .env("PATH", minimal_path(&resolved))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped());
    let output = match command.output() {
        Ok(value) => value,
        Err(_) => return PythonInventory::unavailable(),
    };
    let version = String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().chars().take(32).collect::<String>());
    let supported = version
        .as_deref()
        .and_then(|value| value.split('.').next())
        .and_then(|major| major.parse::<u32>().ok())
        .is_some_and(|major| major == 3);
    PythonInventory {
        available: output.status.success() && supported,
        resolved_path: Some(resolved.to_string_lossy().into_owned()),
        version,
        isolated_no_site: output.status.success() && supported,
    }
}

fn resolve_python(path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let candidates = path
        .map(std::env::split_paths)
        .into_iter()
        .flatten()
        .map(|directory| directory.join("python3"));
    for candidate in candidates {
        if is_executable(&candidate) {
            return std::fs::canonicalize(&candidate).ok().or(Some(candidate));
        }
    }
    None
}

fn minimal_path(runtime: &Path) -> std::ffi::OsString {
    let mut entries = Vec::new();
    if let Some(parent) = runtime.parent() {
        entries.push(parent.to_path_buf());
    }
    for path in [PathBuf::from("/usr/bin"), PathBuf::from("/bin")] {
        if !entries.contains(&path) {
            entries.push(path);
        }
    }
    std::env::join_paths(entries).unwrap_or_default()
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && std::fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_path_is_an_explicit_unavailable_inventory() {
        let inventory = inventory_from(Some(std::ffi::OsStr::new("/definitely/missing")));
        assert!(!inventory.available);
        assert!(inventory.resolved_path.is_none());
        assert!(inventory.path().is_err());
    }

    #[test]
    fn installed_python_supports_isolated_no_site_mode() {
        let inventory = inventory();
        if inventory.resolved_path.is_some() {
            assert!(inventory.available);
            assert!(inventory.isolated_no_site);
            assert!(inventory.version.as_deref().unwrap_or("").starts_with("3."));
        }
    }
}
