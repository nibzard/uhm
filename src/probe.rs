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
    // Observe the leader with waitid(WNOWAIT) so its pid/process-group identity
    // remains pinned until every cleanup signal has been sent. Child::try_wait
    // reaps immediately and would allow the numerical pid/pgid to be reused.
    let mut leader_exited = false;
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
        if !leader_exited {
            match exited_without_reaping(pid) {
                Ok(observed) => leader_exited = observed,
                Err(error) => break Err(format!("observe probe child: {error}")),
            }
        }
        if leader_exited && (eof || truncated) {
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
    // Always clean the owned group, including after a short successful leader:
    // a help program may spawn a descendant that closes stdout and would
    // otherwise outlive the machine deadline. The unreaped leader pins both
    // its pid and pgid through the final signal, so no liveness probe or pipe
    // state is mistaken for proof of ownership.
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    if !leader_exited {
        let term_by = Instant::now() + CLEANUP_ALLOWANCE;
        while Instant::now() < term_by {
            match exited_without_reaping(pid) {
                Ok(true) => break,
                Ok(false) => std::thread::sleep(Duration::from_millis(2)),
                Err(_) => break,
            }
        }
    }
    // Send the final group signal before any wait call can reap the leader.
    // This is harmless when only the exited leader remains and terminates any
    // same-group descendants, including successful probes with closed stdio.
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    drop(pipe);
    match outcome {
        Ok(()) => {
            // waitid established that this cannot block; it now performs the
            // one deliberate reap after all group signalling is finished.
            let status = child
                .wait()
                .map_err(|error| format!("reap probe child: {error}"))?;
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

/// Report whether the direct child has exited without reaping it. Keeping the
/// zombie waitable pins its pid and process-group identity until cleanup is
/// complete, avoiding signals to a subsequently reused numeric id.
fn exited_without_reaping(pid: libc::pid_t) -> std::io::Result<bool> {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { info.si_pid() } != 0)
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
    fn audit19_probe_cleans_successful_descendants_with_closed_stdio() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("descendant.pid");
        let body = format!(
            "#!/bin/sh\n(sleep 30) >/dev/null 2>&1 &\necho $! > '{}'\necho done\n",
            pid_file.display()
        );
        let tool = script(dir.path(), "successful-leak", &body);
        let output = run(&[&string(&tool)], deadline(2_000), 4096, ProbeEnv::Inherit).unwrap();
        assert!(output.success);
        assert_eq!(output.stdout.trim(), "done");
        let descendant: libc::pid_t = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let gone_by = Instant::now() + Duration::from_secs(2);
        while Instant::now() < gone_by && unsafe { libc::kill(descendant, 0) } == 0 {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_ne!(
            unsafe { libc::kill(descendant, 0) },
            0,
            "a successful probe descendant survived group cleanup"
        );
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
