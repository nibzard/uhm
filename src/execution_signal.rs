//! Process-wide signal ownership for bounded child execution.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

static ACTIVE_TARGET: AtomicI32 = AtomicI32::new(0);
static RECEIVED_SIGNAL: AtomicI32 = AtomicI32::new(0);
static EXECUTION_ACTIVE: AtomicBool = AtomicBool::new(false);
static EXECUTION: Mutex<()> = Mutex::new(());
static SIGNALS: OnceLock<Result<(), String>> = OnceLock::new();

/// Serializes child execution so one process-wide signal target is sufficient.
pub(crate) struct ExecutionGuard {
    _execution: MutexGuard<'static, ()>,
}

impl ExecutionGuard {
    /// A positive target denotes a process group; a negative target denotes a
    /// single pid. `terminate` maps both encodings to kill(2).
    pub(crate) fn activate(&self, target: i32) {
        debug_assert_ne!(target, 0);
        ACTIVE_TARGET.store(target, Ordering::SeqCst);
        // A signal can arrive after execution ownership is acquired but before
        // the child has a pid. Replay that recorded signal now that a concrete
        // target exists. Together with the handler's store-before-load order,
        // either this path or the handler must observe the other side.
        if let Some(signal) = self.received_signal() {
            terminate(target, signal);
        }
    }

    /// Keep signal ownership while making a reaped pid/process group
    /// unreachable to the handler during post-child cleanup.
    pub(crate) fn deactivate_target(&self) {
        ACTIVE_TARGET.store(0, Ordering::SeqCst);
    }

    pub(crate) fn received_signal(&self) -> Option<i32> {
        let signal = RECEIVED_SIGNAL.load(Ordering::SeqCst);
        (signal != 0).then_some(signal)
    }

    #[cfg(test)]
    pub(crate) fn active_target_for_test(&self) -> Option<i32> {
        let target = ACTIVE_TARGET.load(Ordering::SeqCst);
        (target != 0).then_some(target)
    }
}

impl Drop for ExecutionGuard {
    fn drop(&mut self) {
        EXECUTION_ACTIVE.store(false, Ordering::SeqCst);
        ACTIVE_TARGET.store(0, Ordering::SeqCst);
    }
}

pub(crate) fn acquire() -> Result<ExecutionGuard, String> {
    install()?;
    let execution = EXECUTION
        .lock()
        .map_err(|_| "child execution lock is poisoned".to_string())?;
    RECEIVED_SIGNAL.store(0, Ordering::SeqCst);
    ACTIVE_TARGET.store(0, Ordering::SeqCst);
    EXECUTION_ACTIVE.store(true, Ordering::SeqCst);
    Ok(ExecutionGuard {
        _execution: execution,
    })
}

fn install() -> Result<(), String> {
    SIGNALS
        .get_or_init(|| {
            for signal in [libc::SIGINT, libc::SIGTERM] {
                unsafe {
                    signal_hook::low_level::register(signal, move || {
                        if !EXECUTION_ACTIVE.load(Ordering::SeqCst) {
                            let _ = signal_hook::low_level::emulate_default_handler(signal);
                            return;
                        }
                        RECEIVED_SIGNAL.store(signal, Ordering::SeqCst);
                        let target = ACTIVE_TARGET.load(Ordering::SeqCst);
                        if target != 0 {
                            terminate(target, signal);
                        }
                    })
                }
                .map_err(|error| format!("install child signal forwarding: {error}"))?;
            }
            Ok(())
        })
        .clone()
}

pub(crate) fn terminate(target: i32, signal: i32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-target, signal);
    }
}

/// Grace given to a `SIGKILL`ed child to actually disappear before uhm stops
/// waiting for it. `SIGKILL` is pending, so a normally-scheduled child is gone
/// in milliseconds; this bound only elapses for a process stuck in
/// uninterruptible sleep (a hung page-in, NFS, or device ioctl), which the
/// kernel cannot interrupt.
pub(crate) const KILL_REAP_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// Place a freshly-spawned child in its own process group from the parent side,
/// racing idempotently against the child's own `setpgid(0, 0)` in `pre_exec`.
/// Without this, a signal delivered between fork and the child's `pre_exec`
/// targets a group that does not yet exist — `kill(-child_pid)` returns `ESRCH`
/// and the escalation silently misses, so a stalled or `ptrace`-stopped child
/// defeats `SIGTERM`/`SIGKILL` and the blocking `wait` that follows can hang
/// forever. Whichever side wins the race, the child becomes the leader of a
/// group equal to its pid; the loser's call is a harmless no-op. Errors are
/// ignored: `EACCES` means the child already `exec`'d (after setting its own
/// group), `ESRCH` means it is already gone.
#[cfg(unix)]
pub(crate) fn assign_process_group(child_pid: i32) {
    debug_assert!(child_pid > 0);
    unsafe {
        let _ = libc::setpgid(child_pid, child_pid);
    }
}

/// Bound a final reap after `SIGKILL`. A child stuck in uninterruptible sleep is
/// not killed until its syscall returns, so a blocking `wait` could hang
/// indefinitely; `SIGKILL` is already pending, so once `deadline` elapses we
/// stop waiting and report the escalation honestly. The kernel reaps the
/// reparented zombie when uhm exits.
#[cfg(unix)]
pub(crate) fn reap_within(
    child: &mut std::process::Child,
    deadline: std::time::Instant,
) -> std::process::ExitStatus {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            // No exit before the deadline (or an unrecoverable wait error): the
            // child is wedged in the kernel. Report the pending SIGKILL honestly
            // rather than blocking uhm on a process it cannot force to die.
            Ok(None) | Err(_) => return killed_status(),
        }
    }
}

/// Synthesize an `ExitStatus` meaning "killed by `SIGKILL`", for the rare case
/// where `reap_within` gives up on a child that never reaped. The raw wait-status
/// encoding keeps the terminating signal in its low byte.
#[cfg(unix)]
fn killed_status() -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(libc::SIGKILL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[cfg(unix)]
    #[test]
    fn activation_replays_a_signal_recorded_before_the_target_exists() {
        use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};

        let execution = acquire().unwrap();
        let mut command = std::process::Command::new("/bin/sleep");
        command.arg("30");
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let mut child = command.spawn().unwrap();
        RECEIVED_SIGNAL.store(libc::SIGTERM, Ordering::SeqCst);
        execution.activate(child.id() as i32);
        let status = child.wait().unwrap();
        execution.deactivate_target();
        assert_eq!(status.signal(), Some(libc::SIGTERM));
    }

    #[test]
    fn execution_target_ownership_is_process_wide_and_serialized() {
        let first = acquire().unwrap();
        const TEST_TARGET: i32 = -2_000_000_000;
        first.activate(TEST_TARGET);
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let second = acquire().unwrap();
            acquired_tx.send(()).unwrap();
            drop(second);
        });

        assert!(acquired_rx.recv_timeout(Duration::from_millis(50)).is_err());
        assert_eq!(ACTIVE_TARGET.load(Ordering::SeqCst), TEST_TARGET);
        first.deactivate_target();
        assert_eq!(ACTIVE_TARGET.load(Ordering::SeqCst), 0);
        drop(first);
        acquired_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        waiter.join().unwrap();
        assert_eq!(ACTIVE_TARGET.load(Ordering::SeqCst), 0);
        assert!(!EXECUTION_ACTIVE.load(Ordering::SeqCst));
    }
}
