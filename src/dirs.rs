//! Explicit XDG/macOS path resolution. Missing or relative roots are errors;
//! uhm never falls back to the current working directory.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub config_file: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

fn absolute_env(name: &str) -> Result<Option<PathBuf>, String> {
    let Some(value) = std::env::var_os(name) else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(format!("${} must be an absolute path", name));
    }
    Ok(Some(path))
}

fn home() -> Result<PathBuf, String> {
    absolute_env("HOME")?.ok_or_else(|| {
        "cannot resolve uhm directories: set HOME or the XDG_*_HOME variables".into()
    })
}

pub fn resolve() -> Result<Paths, String> {
    let home = home();
    let config_root = match absolute_env("XDG_CONFIG_HOME")? {
        Some(p) => p,
        None if cfg!(target_os = "macos") => home
            .as_ref()
            .map(|p| p.join("Library/Application Support"))
            .map_err(Clone::clone)?,
        None => home
            .as_ref()
            .map(|p| p.join(".config"))
            .map_err(Clone::clone)?,
    };
    let data_root = match absolute_env("XDG_DATA_HOME")? {
        Some(p) => p,
        None if cfg!(target_os = "macos") => home
            .as_ref()
            .map(|p| p.join("Library/Application Support"))
            .map_err(Clone::clone)?,
        None => home
            .as_ref()
            .map(|p| p.join(".local/share"))
            .map_err(Clone::clone)?,
    };
    let cache_root = match absolute_env("XDG_CACHE_HOME")? {
        Some(p) => p,
        None if cfg!(target_os = "macos") => home
            .as_ref()
            .map(|p| p.join("Library/Caches"))
            .map_err(Clone::clone)?,
        None => home
            .as_ref()
            .map(|p| p.join(".cache"))
            .map_err(Clone::clone)?,
    };
    Ok(Paths {
        config_file: config_root.join("uhm/config.yaml"),
        data_dir: data_root.join("uhm"),
        cache_dir: cache_root.join("uhm"),
    })
}

pub fn ensure_private_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path)
        .map_err(|e| format!("create private directory {}: {}", path.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("set private permissions on {}: {}", path.display(), e))?;
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn private_directories_are_mode_700() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("private");
        ensure_private_dir(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn relative_roots_are_rejected() {
        let key = "XDG_CONFIG_HOME";
        let old = std::env::var_os(key);
        std::env::set_var(key, "relative");
        assert!(resolve().unwrap_err().contains("absolute"));
        match old {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
}
