//! Child-shell execution seam.

use std::process::Command;

pub trait Executor {
    fn execute(&self, shell: &str, command: &str) -> Result<i32, String>;
}

pub struct SystemExecutor;

impl Executor for SystemExecutor {
    fn execute(&self, shell: &str, command: &str) -> Result<i32, String> {
        let shell = if shell.is_empty() {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
        } else {
            shell.to_string()
        };
        let name = std::path::Path::new(&shell)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(&shell);
        let flag = if matches!(name, "pwsh" | "powershell") {
            "-Command"
        } else {
            "-c"
        };
        let status = Command::new(&shell)
            .arg(flag)
            .arg(command)
            .status()
            .map_err(|error| format!("failed to spawn shell ({}): {}", shell, error))?;
        if let Some(code) = status.code() {
            return Ok(code);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(signal) = status.signal() {
                return Ok(128 + signal);
            }
        }
        Ok(1)
    }
}
