//! Byte-preserving child-shell execution with bounded diagnostics and cancellation.

use std::collections::VecDeque;
use std::io::{IsTerminal, Read, Write};
use std::os::fd::AsRawFd;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

type TeeHandle = std::thread::JoinHandle<Vec<u8>>;

struct RestoreFdFlags {
    fd: std::os::fd::RawFd,
    flags: libc::c_int,
}

struct RestoreSinkFlags {
    descriptors: Vec<RestoreFdFlags>,
}

impl RestoreSinkFlags {
    fn set_nonblocking(fds: &[std::os::fd::RawFd]) -> std::result::Result<Self, String> {
        // Snapshot every descriptor before changing any of them. Two descriptor
        // numbers can share one open-file-description (for example `2>&1`), so
        // a sequential snapshot-and-set would let the second snapshot observe
        // our temporary O_NONBLOCK and later restore the wrong state.
        let descriptors = fds
            .iter()
            .map(|fd| fd_flags(*fd).map(|flags| RestoreFdFlags { fd: *fd, flags }))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let restore = Self { descriptors };
        for descriptor in &restore.descriptors {
            if unsafe {
                libc::fcntl(
                    descriptor.fd,
                    libc::F_SETFL,
                    descriptor.flags | libc::O_NONBLOCK,
                )
            } == -1
            {
                return Err(format!(
                    "configure nonblocking shell output sink: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }
        Ok(restore)
    }
}

impl Drop for RestoreFdFlags {
    fn drop(&mut self) {
        unsafe {
            let _ = libc::fcntl(self.fd, libc::F_SETFL, self.flags);
        }
    }
}

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
    let execution = crate::execution_signal::acquire()?;
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
    let target = target_for(&child, separate_group);
    execution.activate(target);
    if let (Some(bytes), Some(mut input)) = (req.stdin, child.stdin.take()) {
        let data = bytes.to_vec();
        std::thread::spawn(move || {
            let _ = input.write_all(&data);
        });
    }
    let cancel_readers = Arc::new(AtomicBool::new(false));
    let mut stdout_sink = (!out_tty).then(std::io::stdout);
    let mut stderr_sink = (!err_tty).then(std::io::stderr);
    let mut sink_fds = Vec::with_capacity(2);
    if let Some(sink) = &stdout_sink {
        sink_fds.push(sink.as_raw_fd());
    }
    if let Some(sink) = &stderr_sink {
        sink_fds.push(sink.as_raw_fd());
    }
    let sink_flags = match RestoreSinkFlags::set_nonblocking(&sink_fds) {
        Ok(restore) => restore,
        Err(error) => {
            let mut stdout = None;
            let mut stderr = None;
            cleanup_failed_execution(
                &mut child,
                target,
                &cancel_readers,
                &mut stdout,
                &mut stderr,
                &execution,
            );
            return Err(error);
        }
    };
    let mut stdout = None;
    if let (Some(stream), Some(sink)) = (child.stdout.take(), stdout_sink.take()) {
        match tee(stream, sink, req.diagnostic_bytes, cancel_readers.clone()) {
            Ok(handle) => stdout = Some(handle),
            Err(error) => {
                let mut stderr = None;
                cleanup_failed_execution(
                    &mut child,
                    target,
                    &cancel_readers,
                    &mut stdout,
                    &mut stderr,
                    &execution,
                );
                return Err(error);
            }
        }
    }
    let mut stderr = match (child.stderr.take(), stderr_sink.take()) {
        (Some(stream), Some(sink)) => {
            match tee(stream, sink, req.diagnostic_bytes, cancel_readers.clone()) {
                Ok(handle) => Some(handle),
                Err(error) => {
                    cleanup_failed_execution(
                        &mut child,
                        target,
                        &cancel_readers,
                        &mut stdout,
                        &mut None,
                        &execution,
                    );
                    return Err(error);
                }
            }
        }
        _ => None,
    };
    let mut timed_out = false;
    let deadline = started + req.timeout;
    let status = 'wait: loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                cleanup_failed_execution(
                    &mut child,
                    target,
                    &cancel_readers,
                    &mut stdout,
                    &mut stderr,
                    &execution,
                );
                return Err(format!("wait for child: {error}"));
            }
        }
        if let Some(signal) = execution.received_signal() {
            // `activate` normally replays a signal recorded before the child
            // pid existed. Re-send here as well so this wait path remains
            // correct if activation and signal delivery race.
            terminate(target, signal);
            let grace = Instant::now() + Duration::from_millis(500);
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => break 'wait status,
                    Ok(None) if Instant::now() < grace => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Ok(None) => {
                        terminate(target, libc::SIGKILL);
                        match child.wait() {
                            Ok(status) => break 'wait status,
                            Err(error) => {
                                cleanup_failed_execution(
                                    &mut child,
                                    target,
                                    &cancel_readers,
                                    &mut stdout,
                                    &mut stderr,
                                    &execution,
                                );
                                return Err(format!("reap interrupted child: {error}"));
                            }
                        }
                    }
                    Err(error) => {
                        cleanup_failed_execution(
                            &mut child,
                            target,
                            &cancel_readers,
                            &mut stdout,
                            &mut stderr,
                            &execution,
                        );
                        return Err(format!("wait for interrupted child: {error}"));
                    }
                }
            }
        }
        if Instant::now() >= deadline {
            timed_out = true;
            terminate(target, libc::SIGTERM);
            let grace = Instant::now();
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => break 'wait status,
                    Ok(None) => {}
                    Err(error) => {
                        cleanup_failed_execution(
                            &mut child,
                            target,
                            &cancel_readers,
                            &mut stdout,
                            &mut stderr,
                            &execution,
                        );
                        return Err(format!("wait for timed out child: {error}"));
                    }
                }
                if grace.elapsed() >= Duration::from_millis(500) {
                    terminate(target, libc::SIGKILL);
                    match child.wait() {
                        Ok(status) => break 'wait status,
                        Err(error) => {
                            cleanup_failed_execution(
                                &mut child,
                                target,
                                &cancel_readers,
                                &mut stdout,
                                &mut stderr,
                                &execution,
                            );
                            return Err(format!("reap timed out child: {error}"));
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        } else {
            std::thread::sleep(Duration::from_millis(10));
        }
    };

    // `wait`/`try_wait` has reaped the leader, so its pid or process-group id
    // can no longer be treated as stable identity. Stop exposing it to the
    // signal handler before draining inherited pipes. Nonblocking readers are
    // canceled at the absolute deadline instead of signaling a recyclable id.
    execution.deactivate_target();

    while readers_running(&stdout, &stderr)
        && Instant::now() < deadline
        && execution.received_signal().is_none()
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    if readers_running(&stdout, &stderr) {
        timed_out |= execution.received_signal().is_none();
    }
    cancel_readers.store(true, Ordering::SeqCst);
    let stdout_tail = stdout
        .take()
        .map(|handle| handle.join().unwrap_or_default());
    let stderr_tail = stderr
        .take()
        .map(|handle| handle.join().unwrap_or_default());
    drop(sink_flags);
    #[cfg(unix)]
    let child_signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let child_signal = None;
    let forwarded_signal = execution.received_signal();
    let signal = child_signal.or(forwarded_signal);
    let code = if timed_out {
        124
    } else if let Some(forwarded_signal) = forwarded_signal {
        128 + forwarded_signal
    } else {
        status.code().unwrap_or_else(|| 128 + signal.unwrap_or(1))
    };
    Ok(Result {
        code,
        signal,
        stdout_tail,
        stderr_tail,
        timed_out,
        duration: started.elapsed(),
    })
}
fn tee<R: Read + Send + AsRawFd + 'static, W: Write + Send + 'static>(
    mut reader: R,
    mut output: W,
    limit: usize,
    cancel: Arc<AtomicBool>,
) -> std::result::Result<TeeHandle, String> {
    let reader_flags = set_nonblocking(reader.as_raw_fd(), "shell output reader")?;
    Ok(std::thread::spawn(move || {
        let _reader_flags = RestoreFdFlags {
            fd: reader.as_raw_fd(),
            flags: reader_flags,
        };
        let mut ring = VecDeque::with_capacity(limit);
        let mut buf = [0u8; 8192];
        'read: loop {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    for b in &buf[..n] {
                        if ring.len() == limit {
                            ring.pop_front();
                        }
                        ring.push_back(*b);
                    }
                    let mut written = 0;
                    while written < n {
                        if cancel.load(Ordering::SeqCst) {
                            break 'read;
                        }
                        match output.write(&buf[written..n]) {
                            Ok(0) => break 'read,
                            Ok(count) => written += count,
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    std::io::ErrorKind::WouldBlock
                                        | std::io::ErrorKind::Interrupted
                                ) =>
                            {
                                std::thread::sleep(Duration::from_millis(5));
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {
                                break 'read;
                            }
                            Err(_) => break 'read,
                        }
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
        let _ = output.flush();
        ring.into_iter().collect()
    }))
}

fn set_nonblocking(
    fd: std::os::fd::RawFd,
    label: &str,
) -> std::result::Result<libc::c_int, String> {
    let flags = fd_flags(fd)?;
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        Err(format!(
            "configure nonblocking {label}: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(flags)
    }
}

fn fd_flags(fd: std::os::fd::RawFd) -> std::result::Result<libc::c_int, String> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        Err(format!(
            "inspect shell output descriptor flags: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(flags)
    }
}

fn target_for(child: &std::process::Child, separate_group: bool) -> i32 {
    if separate_group {
        child.id() as i32
    } else {
        -(child.id() as i32)
    }
}

fn readers_running(stdout: &Option<TeeHandle>, stderr: &Option<TeeHandle>) -> bool {
    stdout.as_ref().is_some_and(|handle| !handle.is_finished())
        || stderr.as_ref().is_some_and(|handle| !handle.is_finished())
}

fn cleanup_failed_execution(
    child: &mut std::process::Child,
    target: i32,
    cancel: &Arc<AtomicBool>,
    stdout: &mut Option<TeeHandle>,
    stderr: &mut Option<TeeHandle>,
    execution: &crate::execution_signal::ExecutionGuard,
) {
    terminate(target, libc::SIGKILL);
    let _ = child.wait();
    // The leader is now reaped. Clear the pid/group before any reader join so
    // a concurrent signal cannot be forwarded to a recycled identifier.
    execution.deactivate_target();
    cancel.store(true, Ordering::SeqCst);
    if let Some(handle) = stdout.take() {
        let _ = handle.join();
    }
    if let Some(handle) = stderr.take() {
        let _ = handle.join();
    }
}
fn terminate(target: i32, signal: i32) {
    crate::execution_signal::terminate(target, signal);
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

    #[test]
    fn descendant_held_pipes_are_bounded_and_report_timeout() {
        let timeout = Duration::from_millis(200);
        let result = execute(Request {
            shell: "/bin/sh",
            // Keep the inherited pipe open past the deadline without leaving a
            // long-lived orphan now that post-reap PGID signaling is forbidden.
            command: "sleep 2 & printf primary-done",
            stdin: None,
            timeout,
            diagnostic_bytes: 64,
            deny_common_env: false,
            deny_env: &[],
            containment: crate::containment::Mode::Off,
        })
        .unwrap();
        assert!(result.timed_out);
        assert_eq!(result.code, 124);
        assert!(result.duration >= timeout);
        assert!(result.duration < Duration::from_secs(2));
        assert_eq!(
            result.stdout_tail.as_deref(),
            Some(b"primary-done".as_slice())
        );
    }

    #[test]
    fn pending_descendant_output_drains_before_the_absolute_deadline() {
        let timeout = Duration::from_secs(1);
        let result = execute(Request {
            shell: "/bin/sh",
            command: "(sleep 0.2; printf delayed-output) &",
            stdin: None,
            timeout,
            diagnostic_bytes: 64,
            deny_common_env: false,
            deny_env: &[],
            containment: crate::containment::Mode::Off,
        })
        .unwrap();
        assert!(!result.timed_out);
        assert_eq!(result.code, 0);
        assert!(result.duration >= Duration::from_millis(150));
        assert!(result.duration < timeout);
        assert_eq!(
            result.stdout_tail.as_deref(),
            Some(b"delayed-output".as_slice())
        );
    }

    #[test]
    fn tee_cancellation_is_bounded_when_the_output_sink_stops_reading() {
        use std::os::unix::net::UnixStream;

        let (reader, mut producer) = UnixStream::pair().unwrap();
        let (sink, unread_sink) = UnixStream::pair().unwrap();
        let sink_owner = sink.try_clone().unwrap();
        let sink_flags = RestoreSinkFlags::set_nonblocking(&[sink_owner.as_raw_fd()]).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let handle = tee(reader, sink, 64, cancel.clone()).unwrap();
        let producer_thread = std::thread::spawn(move || {
            let bytes = vec![b'x'; 1024 * 1024];
            let _ = producer.write_all(&bytes);
        });
        std::thread::sleep(Duration::from_millis(50));
        let started = Instant::now();
        cancel.store(true, Ordering::SeqCst);
        let _ = handle.join().unwrap();
        drop(sink_flags);
        drop(sink_owner);
        drop(unread_sink);
        producer_thread.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn aliased_sink_descriptors_restore_the_original_file_status_flags() {
        use std::os::unix::net::UnixStream;

        let (sink, _peer) = UnixStream::pair().unwrap();
        let alias = sink.try_clone().unwrap();
        let original = fd_flags(sink.as_raw_fd()).unwrap();
        assert_eq!(original & libc::O_NONBLOCK, 0);

        {
            let _restore =
                RestoreSinkFlags::set_nonblocking(&[sink.as_raw_fd(), alias.as_raw_fd()]).unwrap();
            assert_ne!(fd_flags(sink.as_raw_fd()).unwrap() & libc::O_NONBLOCK, 0);
            assert_ne!(fd_flags(alias.as_raw_fd()).unwrap() & libc::O_NONBLOCK, 0);
        }

        assert_eq!(fd_flags(sink.as_raw_fd()).unwrap(), original);
        assert_eq!(fd_flags(alias.as_raw_fd()).unwrap(), original);
    }

    #[test]
    fn post_reap_drain_deactivates_process_group_identity() {
        const TEST_GROUP_TARGET: i32 = 2_000_000_000;
        let execution = crate::execution_signal::acquire().unwrap();
        execution.activate(TEST_GROUP_TARGET);
        execution.deactivate_target();
        assert_eq!(execution.active_target_for_test(), None);
    }

    #[test]
    fn failed_execution_cleanup_deactivates_a_reaped_pid_before_return() {
        let execution = crate::execution_signal::acquire().unwrap();
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let target = -(child.id() as i32);
        execution.activate(target);
        let cancel = Arc::new(AtomicBool::new(false));
        let mut stdout = None;
        let mut stderr = None;
        cleanup_failed_execution(
            &mut child,
            target,
            &cancel,
            &mut stdout,
            &mut stderr,
            &execution,
        );
        assert_eq!(execution.active_target_for_test(), None);
    }
}
