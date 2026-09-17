//! Small, bounded Python runtime inventory used by the action router and executor.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

/// Internal machine-time bound for one inventory probe. A version line is a
/// few dozen bytes, so half a second is generous even on a cold start, and a
/// Python that cannot answer in it is unusable for the bounded job anyway.
const INVENTORY_DEADLINE_MS: u64 = 500;
const INVENTORY_MAX_BYTES: usize = 4096;

fn inventory_from(path: Option<&std::ffi::OsStr>) -> PythonInventory {
    let Some(resolved) = resolve_python(path) else {
        return PythonInventory::unavailable();
    };
    let resolved_text = resolved.to_string_lossy().into_owned();
    let minimal = minimal_path(&resolved);
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_millis(INVENTORY_DEADLINE_MS);
    // Timeout, excessive output, or a nonzero exit means an unavailable
    // inventory; there is no fallback to an unbounded call.
    let outcome = crate::probe::run(
        &[
            &resolved_text,
            "-I",
            "-S",
            "-c",
            "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}.{sys.version_info.micro}')",
        ],
        deadline,
        INVENTORY_MAX_BYTES,
        crate::probe::ProbeEnv::Cleared {
            path: minimal.as_os_str(),
        },
    );
    let Ok(output) = outcome else {
        return PythonInventory {
            available: false,
            resolved_path: Some(resolved_text),
            version: None,
            isolated_no_site: false,
        };
    };
    let version = if output.truncated {
        None
    } else {
        Some(output.stdout.trim().chars().take(32).collect::<String>())
    };
    let supported = version
        .as_deref()
        .and_then(|value| value.split('.').next())
        .and_then(|major| major.parse::<u32>().ok())
        .is_some_and(|major| major == 3);
    let usable = output.success && supported;
    PythonInventory {
        available: usable,
        resolved_path: Some(resolved_text),
        version,
        isolated_no_site: usable,
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

/// The cleared child environment's `PATH`. It carries the interpreter's own
/// directory plus the standard system binary directories, because a
/// version-manager interpreter is often a shell script that resolves itself
/// through `#!/usr/bin/env bash` and cannot start without them.
pub(crate) fn minimal_path(runtime: &Path) -> std::ffi::OsString {
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
    fn minimal_path_keeps_the_interpreter_directory_and_the_system_directories() {
        let entries: Vec<PathBuf> =
            std::env::split_paths(&minimal_path(Path::new("/opt/versions/3.13/bin/python3")))
                .collect();
        assert_eq!(
            entries,
            vec![
                PathBuf::from("/opt/versions/3.13/bin"),
                PathBuf::from("/usr/bin"),
                PathBuf::from("/bin"),
            ]
        );
        // A shim living in a system directory must not duplicate that entry.
        assert_eq!(
            std::env::split_paths(&minimal_path(Path::new("/usr/bin/python3")))
                .collect::<Vec<PathBuf>>(),
            vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")]
        );
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

    // Plan 19 W08: the inventory probe is bounded; a hanging, flooding, or
    // failing interpreter means an unavailable inventory, never an
    // unbounded wait.

    fn python_shim(name: &str, body: &str) -> (tempfile::TempDir, std::ffi::OsString) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path_env = std::env::join_paths([dir.path()]).unwrap();
        (dir, path_env)
    }

    #[cfg(unix)]
    #[test]
    fn audit19_runtime_a_sleeping_python_shim_is_unavailable_quickly() {
        let (_dir, path_env) = python_shim("python3", "#!/bin/sh\nsleep 30\n");
        let started = std::time::Instant::now();
        let inventory = inventory_from(Some(path_env.as_os_str()));
        assert!(!inventory.available);
        assert!(!inventory.isolated_no_site);
        assert!(inventory.resolved_path.is_some());
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "a sleeping shim blocked inventory for {:?}",
            started.elapsed()
        );
    }

    #[cfg(unix)]
    #[test]
    fn audit19_runtime_excessive_python_output_is_unavailable() {
        let (_dir, path_env) = python_shim(
            "python3",
            "#!/bin/sh\ni=0\nwhile [ $i -lt 3000 ]; do echo padding-$i; i=$((i+1)); done\n",
        );
        let started = std::time::Instant::now();
        let inventory = inventory_from(Some(path_env.as_os_str()));
        assert!(!inventory.available, "{inventory:?}");
        // Excessive output specifically: the cap dropped the version instead
        // of parsing flood text, and the resolved path is still recorded.
        assert!(
            inventory.version.is_none(),
            "truncated output must yield no version: {inventory:?}"
        );
        assert!(inventory.resolved_path.is_some(), "{inventory:?}");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "a flooding shim blocked inventory for {:?}",
            started.elapsed()
        );
    }

    #[cfg(unix)]
    #[test]
    fn audit19_runtime_a_failing_or_malformed_python_is_unavailable() {
        let failing = python_shim("python3", "#!/bin/sh\nexit 1\n");
        let inventory = inventory_from(Some(failing.1.as_os_str()));
        assert!(!inventory.available);

        let malformed = python_shim("python3", "#!/bin/sh\necho not-a-version\n");
        let inventory = inventory_from(Some(malformed.1.as_os_str()));
        assert!(!inventory.available);
        assert!(!inventory.isolated_no_site);
    }
}
