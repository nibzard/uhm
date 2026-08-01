//! Context bundle: gather machine state with zero crates. Each field is a
//! concurrent, timeout-bounded subprocess spawn; failures/timeouts are omitted.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[derive(Default)]
pub struct Bundle {
    pub os: String,
    pub shell: String,
    pub cwd: String,
    pub user: String,
    pub host: String,
    pub kernel: String,
    pub git_branch: String,
    pub git_dirty: String,
    pub ls_hint: String,
}

pub trait Provider {
    fn gather(&self, shell: &str, include_ls: bool, timeout_ms: u64) -> Bundle;
}

pub struct SystemProvider;

impl Provider for SystemProvider {
    fn gather(&self, shell: &str, include_ls: bool, timeout_ms: u64) -> Bundle {
        gather(shell, include_ls, timeout_ms)
    }
}

/// Run `argv` and capture stdout, killing it if it exceeds `timeout`.
fn run_timed(argv: &[&str], timeout: Duration) -> Option<String> {
    if argv.is_empty() {
        return None;
    }
    let mut cmd = Command::new(argv[0]);
    cmd.args(&argv[1..])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = cmd.spawn().ok()?;
    let mut out = child.stdout.take()?;
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        let _ = tx.send(s);
    });
    let result = match rx.recv_timeout(timeout) {
        Ok(s) => Some(s),
        Err(_) => {
            let _ = child.kill();
            None
        }
    };
    let _ = child.wait(); // reap either way
    result
}

pub fn gather(shell: &str, include_ls: bool, timeout_ms: u64) -> Bundle {
    let to = Duration::from_millis(timeout_ms);
    let os = std::env::consts::OS.to_string();
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let sh = if shell.is_empty() {
        std::env::var("SHELL").unwrap_or_default()
    } else {
        shell.to_string()
    };
    let user = run_timed(&["id", "-un"], to)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let uname = run_timed(&["uname", "-srn"], to).unwrap_or_default();
    let parts: Vec<&str> = uname.split_whitespace().collect();
    let host = parts.get(2).map(|s| s.to_string()).unwrap_or_default();
    let kernel = match (parts.first(), parts.get(1)) {
        (Some(a), Some(b)) => format!("{} {}", a, b),
        _ => String::new(),
    };
    let git_branch = run_timed(&["git", "rev-parse", "--abbrev-ref", "HEAD"], to)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let dirty = run_timed(&["git", "status", "--porcelain"], to).unwrap_or_default();
    let git_dirty = if git_branch.is_empty() {
        String::new()
    } else if dirty.trim().is_empty() {
        "clean".to_string()
    } else {
        format!("{} changed", dirty.lines().count())
    };
    let ls_hint = if include_ls {
        run_timed(&["ls", "-1Ap"], to)
            .map(|s| {
                let lines: Vec<&str> = s.lines().take(40).collect();
                lines.join(" ")
            })
            .unwrap_or_default()
    } else {
        String::new()
    };
    Bundle {
        os,
        shell: sh,
        cwd,
        user,
        host,
        kernel,
        git_branch,
        git_dirty,
        ls_hint,
    }
}

impl Bundle {
    pub fn render(&self) -> String {
        let mut s = String::new();
        let mut add = |k: &str, v: &str| {
            if !v.is_empty() {
                s.push_str(&format!("- {}: {}\n", k, v));
            }
        };
        add("os", &self.os);
        add("shell", &self.shell);
        add("cwd", &self.cwd);
        add("user", &self.user);
        add("host", &self.host);
        if !self.kernel.is_empty() {
            add("kernel", &self.kernel);
        }
        if !self.git_branch.is_empty() {
            add("git", &format!("{} ({})", self.git_branch, self.git_dirty));
        }
        if !self.ls_hint.is_empty() {
            add("cwd-contents", &self.ls_hint);
        }
        s
    }
}
