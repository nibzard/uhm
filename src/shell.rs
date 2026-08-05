//! Byte-preserving child-shell execution with bounded diagnostics and cancellation.

use std::collections::VecDeque;
use std::io::{IsTerminal, Read, Write};
use std::process::Stdio;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Once;
use std::time::{Duration, Instant};

static CHILD_GROUP: AtomicI32 = AtomicI32::new(0);
static SIGNALS: Once = Once::new();

#[derive(Debug)]
pub struct Result {
    pub code: i32,
    pub signal: Option<i32>,
    pub stdout_tail: Option<Vec<u8>>,
    pub stderr_tail: Option<Vec<u8>>,
    pub timed_out: bool,
    pub duration: Duration,
}
pub struct Request<'a> {
    pub shell: &'a str,
    pub command: &'a str,
    pub stdin: Option<&'a [u8]>,
    pub timeout: Duration,
    pub diagnostic_bytes: usize,
    pub deny_common_env: bool,
    pub deny_env: &'a [String],
    pub containment: crate::containment::Mode,
}

pub fn execute(req: Request<'_>) -> std::result::Result<Result, String> {
    install_signal_forwarding();
    let started = Instant::now();
    let name = std::path::Path::new(req.shell)
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or(req.shell);
    let flag = if matches!(name, "pwsh" | "powershell") {
        "-Command"
    } else {
        "-c"
    };
    let out_tty = std::io::stdout().is_terminal();
    let err_tty = std::io::stderr().is_terminal();
    let cwd = std::env::current_dir().map_err(|e| format!("resolve working directory: {e}"))?;
    let arguments = vec![flag.into(), req.command.into()];
    let mut cmd = crate::containment::command(
        req.containment,
        std::path::Path::new(req.shell),
        &arguments,
        &cwd,
        &[],
    )?;
    if req.stdin.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::inherit());
    }
    cmd.stdout(if out_tty {
        Stdio::inherit()
    } else {
        Stdio::piped()
    });
    cmd.stderr(if err_tty {
        Stdio::inherit()
    } else {
        Stdio::piped()
    });
    crate::environment::apply(&mut cmd, req.deny_common_env, req.deny_env);
    let separate_group = !out_tty && !err_tty;
    #[cfg(unix)]
    if separate_group {
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn shell ({}): {}", req.shell, e))?;
    CHILD_GROUP.store(
        if separate_group {
            child.id() as i32
        } else {
            -(child.id() as i32)
        },
        Ordering::SeqCst,
    );
    if let (Some(bytes), Some(mut input)) = (req.stdin, child.stdin.take()) {
        let data = bytes.to_vec();
        std::thread::spawn(move || {
            let _ = input.write_all(&data);
        });
    }
    let stdout = child
        .stdout
        .take()
        .map(|s| tee(s, std::io::stdout(), req.diagnostic_bytes));
    let stderr = child
        .stderr
        .take()
        .map(|s| tee(s, std::io::stderr(), req.diagnostic_bytes));
    let target = if separate_group {
        child.id() as i32
    } else {
        -(child.id() as i32)
    };
    let mut timed_out = false;
    let status = 'wait: loop {
        if let Some(s) = child
            .try_wait()
            .map_err(|e| format!("wait for child: {}", e))?
        {
            break s;
        }
        if started.elapsed() >= req.timeout {
            timed_out = true;
            terminate(target, libc::SIGTERM);
            let grace = Instant::now();
            loop {
                if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
                    break 'wait s;
                }
                if grace.elapsed() >= Duration::from_millis(500) {
                    terminate(target, libc::SIGKILL);
                    break 'wait child.wait().map_err(|e| e.to_string())?;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        } else {
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    CHILD_GROUP.store(0, Ordering::SeqCst);
    let stdout_tail = stdout.map(|h| h.join().unwrap_or_default());
    let stderr_tail = stderr.map(|h| h.join().unwrap_or_default());
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal = None;
    let code = status.code().unwrap_or_else(|| 128 + signal.unwrap_or(1));
    Ok(Result {
        code,
        signal,
        stdout_tail,
        stderr_tail,
        timed_out,
        duration: started.elapsed(),
    })
}
fn tee<R: Read + Send + 'static, W: Write + Send + 'static>(
    mut reader: R,
    mut output: W,
    limit: usize,
) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut ring = VecDeque::with_capacity(limit);
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    match output.write_all(&buf[..n]) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => break,
                        Err(_) => {}
                    }
                    for b in &buf[..n] {
                        if ring.len() == limit {
                            ring.pop_front();
                        }
                        ring.push_back(*b);
                    }
                }
                Err(_) => break,
            }
        }
        let _ = output.flush();
        ring.into_iter().collect()
    })
}
fn install_signal_forwarding() {
    SIGNALS.call_once(|| {
        for sig in [libc::SIGINT, libc::SIGTERM] {
            unsafe {
                let _ = signal_hook::low_level::register(sig, move || {
                    let target = CHILD_GROUP.load(Ordering::SeqCst);
                    if target != 0 {
                        terminate(target, sig);
                    }
                });
            }
        }
    });
}
fn terminate(target: i32, signal: i32) {
    #[cfg(unix)]
    unsafe {
        // A positive target denotes a process group; a negative target denotes
        // a single pid. Negation therefore maps both encodings to kill(2).
        libc::kill(-target, signal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_stdin_and_status() {
        let r = execute(Request {
            shell: "/bin/sh",
            command: "test \"$(wc -c)\" -eq 4",
            stdin: Some(&[0, 1, 2, 3]),
            timeout: Duration::from_secs(2),
            diagnostic_bytes: 32,
            deny_common_env: false,
            deny_env: &[],
            containment: crate::containment::Mode::Off,
        })
        .unwrap();
        assert_eq!(r.code, 0);
    }
    #[test]
    fn removes_all_provider_secrets() {
        std::env::set_var("OPENAI_API_KEY", "sentinel");
        std::env::set_var("CEREBRAS_API_KEY", "cerebras-sentinel");
        std::env::set_var("DEEPSEEK_API_KEY", "deepseek-sentinel");
        let r = execute(Request {
            shell: "/bin/sh",
            command: "test -z \"$OPENAI_API_KEY\" && test -z \"$CEREBRAS_API_KEY\" && test -z \"$DEEPSEEK_API_KEY\"",
            stdin: None,
            timeout: Duration::from_secs(2),
            diagnostic_bytes: 32,
            deny_common_env: false,
            deny_env: &[],
            containment: crate::containment::Mode::Off,
        })
        .unwrap();
        assert_eq!(r.code, 0);
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("CEREBRAS_API_KEY");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn common_secret_preset_removes_named_capabilities() {
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "sentinel");
        std::env::set_var("SSH_AUTH_SOCK", "/tmp/sentinel-agent");
        let result = execute(Request {
            shell: "/bin/sh",
            command: "test -z \"$AWS_SECRET_ACCESS_KEY\" && test -z \"$SSH_AUTH_SOCK\"",
            stdin: None,
            timeout: Duration::from_secs(2),
            diagnostic_bytes: 32,
            deny_common_env: true,
            deny_env: &[],
            containment: crate::containment::Mode::Off,
        })
        .unwrap();
        assert_eq!(result.code, 0);
        std::env::remove_var("AWS_SECRET_ACCESS_KEY");
        std::env::remove_var("SSH_AUTH_SOCK");
    }

    #[test]
    fn timeout_terminates_the_child() {
        let result = execute(Request {
            shell: "/bin/sh",
            command: "sleep 5",
            stdin: None,
            timeout: Duration::from_millis(30),
            diagnostic_bytes: 32,
            deny_common_env: false,
            deny_env: &[],
            containment: crate::containment::Mode::Off,
        })
        .unwrap();
        assert!(result.timed_out);
        assert_ne!(result.code, 0);
    }
}
