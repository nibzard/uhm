//! ABOUTME: Bounded direct-argv subprocess collector shared by every discovery probe.
//! ABOUTME: Drains stdout while the child runs and terminates the owned process group at the deadline.

use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Child environment for a probe: inherit the caller's, or run cleared with
/// only the given `PATH`.
pub(crate) enum ProbeEnv<'a> {
    Inherit,
    Cleared { path: &'a std::ffi::OsStr },
}

#[derive(Debug)]
pub(crate) struct ProbeOutput {
    /// Capped prefix of stdout, lossily decoded.
    pub stdout: String,
    /// More bytes existed past the cap and were discarded.
    pub truncated: bool,
    /// The child exited with a successful status.
    pub success: bool,
}

/// Grace period between terminating the owned group and killing it.
const CLEANUP_ALLOWANCE: Duration = Duration::from_millis(100);
const CHUNK: usize = 4096;

/// Run one bounded, inert probe: direct argv with no shell, stdin closed,
/// stderr discarded, stdout drained while the child runs. The same absolute
/// deadline bounds child wait and pipe drainage. The child runs in its own
/// process group so a probe that spawns descendants can be terminated whole;
/// on timeout or error the group is terminated, then killed after a short
/// explicit allowance, and the direct child is reaped. Output is retained
/// only up to `cap` bytes; the crossing chunk is not accumulated and further
/// bytes are drained and discarded so the child can still exit promptly.
pub(crate) fn run(
    argv: &[&str],
    deadline: Instant,
    cap: usize,
    env: ProbeEnv<'_>,
) -> Result<ProbeOutput, String> {
    if argv.is_empty() {
        return Err("probe requires a non-empty argv".into());
    }
    let mut command = Command::new(argv[0]);
    command
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped());
    if let ProbeEnv::Cleared { path } = env {
        command.env_clear().env("PATH", path);
    }
    unsafe {
        command.pre_exec(|| {
            // Own process group: the collector owns every descendant this
            // probe creates and can terminate the whole tree at the deadline.
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            #[cfg(target_os = "linux")]
            {
                // Die with the collector instead of outliving an interrupted
                // discovery loop. Child-side only: no parent signal handler.
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            }
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("spawn probe {}: {error}", argv[0]))?;
    let pid = child.id() as libc::pid_t;
    // Best effort from the parent side as well; the child sets its group
    // before exec, and a failure here only means the child already has.
    unsafe {
        let _ = libc::setpgid(pid, pid);
    }
    let mut pipe = child
        .stdout
        .take()
        .ok_or_else(|| "probe stdout was not captured".to_string())?;
    set_nonblocking(pipe.as_raw_fd())?;

    let mut collected: Vec<u8> = Vec::new();
    let mut truncated = false;
    let mut eof = false;
    let mut status: Option<std::process::ExitStatus> = None;
    let outcome = loop {
        if !eof {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let millis = remaining.as_millis().min(i32::MAX as u128) as i32;
            let mut fds = [libc::pollfd {
                fd: pipe.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            }];
            let ready = unsafe { libc::poll(fds.as_mut_ptr(), 1, millis) };
            if ready < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                break Err(format!("poll probe output: {error}"));
            }
            if ready > 0 && (fds[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR)) != 0 {
                let mut chunk = [0u8; CHUNK];
                match pipe.read(&mut chunk) {
                    Ok(0) => eof = true,
                    Ok(count) => {
                        if collected.len() < cap {
                            let room = cap - collected.len();
                            let keep = count.min(room);
                            collected.extend_from_slice(&chunk[..keep]);
                            if keep < count {
                                truncated = true;
                            }
                        } else {
                            // Past the cap: drain and discard, never accumulate.
                            truncated = true;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => break Err(format!("read probe output: {error}")),
                }
            }
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(observed) => status = observed,
                Err(error) => break Err(format!("wait probe child: {error}")),
            }
        }
        if status.is_some() && (eof || truncated) {
            break Ok(());
        }
        if Instant::now() >= deadline {
            break Err(format!("probe {} exceeded its deadline", argv[0]));
        }
        if eof {
            // The child still runs with every writer closed: short sleep
            // instead of a busy loop.
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    // Terminate exactly the processes this probe owns, then reap the direct
    // child within a bounded allowance. Escalation is aimed only at provably
    // live targets: the group is signaled only when it still has a member —
    // the leader is unreaped, or the pipe has an open writer, which can only
    // be a member of this probe's tree — and the leader is signaled only
    // while unreaped, so a recycled process id or group id can never be
    // targeted. The truncated-success path takes the same route: the child
    // exited, but a descendant may still hold the pipe.
    let group_may_live = status.is_none() || !eof;
    if (outcome.is_err() || truncated) && group_may_live {
        unsafe {
            libc::kill(-pid, libc::SIGTERM);
        }
        let kill_by = Instant::now() + CLEANUP_ALLOWANCE;
        while Instant::now() < kill_by {
            std::thread::sleep(Duration::from_millis(2));
        }
        // Signal 0 probes liveness without signaling: ESRCH means the target
        // is gone and must not be signaled (its id may already be recycled).
        let group_alive = unsafe { libc::kill(-pid, 0) } == 0;
        if group_alive {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        } else if unsafe { libc::kill(pid, 0) } == 0 {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
    drop(pipe);
    match outcome {
        Ok(()) => {
            let status = status.expect("a finished probe always observed a status");
            Ok(ProbeOutput {
                stdout: String::from_utf8_lossy(&collected).into_owned(),
                truncated,
                success: status.success(),
            })
        }
        Err(message) => {
            // Bounded reap: SIGKILL cannot preempt a child stuck in
            // uninterruptible sleep (stalled mount) or ptrace-stop, so the
            // wait is polled for the cleanup allowance and then abandoned —
            // a zombie until this process exits is bounded; blocking here
            // forever is not.
            let reap_by = Instant::now() + CLEANUP_ALLOWANCE;
            while Instant::now() < reap_by {
                if child.try_wait().is_ok_and(|observed| observed.is_some()) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(message)
        }
    }
}

fn set_nonblocking(fd: std::os::fd::RawFd) -> Result<(), String> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(format!(
            "read probe descriptor flags: {}",
            std::io::Error::last_os_error()
        ));
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(format!(
            "set probe descriptor nonblocking: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn deadline(ms: u64) -> Instant {
        Instant::now() + Duration::from_millis(ms)
    }

    fn script(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn string(path: &std::path::Path) -> String {
        path.to_string_lossy().into_owned()
    }

    #[cfg(unix)]
    #[test]
    fn audit19_probe_collects_a_short_successful_output() {
        let dir = tempfile::tempdir().unwrap();
        let tool = script(
            dir.path(),
            "hello",
            "#!/bin/sh\necho 'usage: hello world'\n",
        );
        let output = run(&[&string(&tool)], deadline(5_000), 4096, ProbeEnv::Inherit).unwrap();
        assert!(output.success);
        assert!(!output.truncated);
        assert!(output.stdout.contains("usage: hello world"));
    }

    #[cfg(unix)]
    #[test]
    fn audit19_probe_returns_unsuccessful_for_a_nonzero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let tool = script(dir.path(), "failing", "#!/bin/sh\nexit 3\n");
        let output = run(&[&string(&tool)], deadline(5_000), 4096, ProbeEnv::Inherit).unwrap();
        assert!(!output.success);
    }

    #[cfg(unix)]
    #[test]
    fn audit19_probe_fails_fast_for_a_missing_executable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("definitely-not-installed");
        let started = Instant::now();
        let result = run(&[&string(&path)], deadline(5_000), 4096, ProbeEnv::Inherit);
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn audit19_probe_honors_the_deadline_for_a_hanging_child() {
        let dir = tempfile::tempdir().unwrap();
        let tool = script(dir.path(), "hang", "#!/bin/sh\nsleep 30\necho late\n");
        let started = Instant::now();
        let result = run(&[&string(&tool)], deadline(400), 4096, ProbeEnv::Inherit);
        assert!(result.is_err(), "{result:?}");
        let elapsed = started.elapsed();
        // Outer watchdog: the deadline plus the bounded cleanup allowance
        // must bound the whole call.
        assert!(
            elapsed < Duration::from_secs(10),
            "hanging probe took {elapsed:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn audit19_probe_caps_output_larger_than_a_pipe_buffer() {
        let dir = tempfile::tempdir().unwrap();
        // Far more than a pipe buffer, and more than the cap.
        let tool = script(
            dir.path(),
            "flood",
            "#!/bin/sh\ni=0\nwhile [ $i -lt 2000 ]; do echo \"padding line $i\"; i=$((i+1)); done\n",
        );
        let started = Instant::now();
        let output = run(&[&string(&tool)], deadline(10_000), 4096, ProbeEnv::Inherit).unwrap();
        assert!(output.truncated, "output past the cap must be flagged");
        assert!(output.stdout.len() <= 4096);
        assert!(output.stdout.contains("padding line"));
        // The flood child exits promptly once drained instead of blocking
        // on a full pipe.
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "draining took {elapsed:?}",
            elapsed = started.elapsed()
        );
    }

    #[cfg(unix)]
    #[test]
    fn audit19_probe_stops_when_a_descendant_holds_stdout_past_the_deadline() {
        let dir = tempfile::tempdir().unwrap();
        // The direct child exits successfully, but a background descendant
        // keeps the pipe open: the call must still end at the deadline.
        let tool = script(dir.path(), "leaky", "#!/bin/sh\n(sleep 30) &\necho done\n");
        let started = Instant::now();
        let result = run(&[&string(&tool)], deadline(400), 4096, ProbeEnv::Inherit);
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(10),
            "descendant-held stdout blocked the probe for {elapsed:?}"
        );
        // Either the deadline fired (error) or EOF plus termination ended it;
        // both end the call without an unbounded read.
        let _ = result;
    }

    #[cfg(unix)]
    #[test]
    fn audit19_probe_cleared_environment_runs_with_only_the_given_path() {
        let dir = tempfile::tempdir().unwrap();
        let tool = script(
            dir.path(),
            "envprobe",
            "#!/bin/sh\nif [ \"$#\" -eq 0 ] && [ -z \"$HOME$USER\" ]; then echo isolated; fi\nexit 0\n",
        );
        let path = std::env::join_paths([dir.path()]).unwrap();
        let output = run(
            &[&string(&tool)],
            deadline(5_000),
            4096,
            ProbeEnv::Cleared {
                path: path.as_os_str(),
            },
        )
        .unwrap();
        assert!(output.success);
        assert_eq!(output.stdout.trim(), "isolated");
    }
}
