//! Explicit Linux Bubblewrap containment for child execution.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Off,
    Bubblewrap,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Bubblewrap => "bubblewrap",
        }
    }
}

pub fn executable() -> Option<PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .map(|directory| directory.join("bwrap"))
        .find(|path| path.is_file())
}

pub fn command(
    mode: Mode,
    executable: &Path,
    arguments: &[std::ffi::OsString],
    cwd: &Path,
    writable_roots: &[&Path],
) -> Result<Command, String> {
    if mode == Mode::Off {
        let mut command = Command::new(executable);
        command.args(arguments).current_dir(cwd);
        return Ok(command);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (executable, arguments, cwd, writable_roots);
        return Err("Bubblewrap containment is available only on Linux".into());
    }
    #[cfg(target_os = "linux")]
    {
        let bwrap = self::executable()
            .ok_or("Bubblewrap containment was requested, but `bwrap` is not available in PATH")?;
        let mut command = Command::new(bwrap);
        command.args([
            "--die-with-parent",
            "--new-session",
            "--unshare-all",
            "--ro-bind",
            "/",
            "/",
            "--dev",
            "/dev",
            "--proc",
            "/proc",
            "--tmpfs",
            "/tmp",
        ]);
        let mut roots = vec![cwd];
        roots.extend_from_slice(writable_roots);
        roots.sort_unstable();
        roots.dedup();
        for root in roots {
            command.arg("--bind").arg(root).arg(root);
        }
        command
            .arg("--chdir")
            .arg(cwd)
            .arg("--")
            .arg(executable)
            .args(arguments)
            .current_dir(cwd);
        Ok(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_executes_the_requested_program_directly() {
        let command = command(
            Mode::Off,
            Path::new("/bin/sh"),
            &["-c".into(), "true".into()],
            Path::new("/tmp"),
            &[],
        )
        .unwrap();
        assert_eq!(command.get_program(), "/bin/sh");
        assert_eq!(command.get_args().count(), 2);
    }
}
