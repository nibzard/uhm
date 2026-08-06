//! Minimal advisory file locking over `libc::flock(2)`.
//!
//! Replaces the dormant `fs2` crate for the two methods uhm uses:
//! blocking-exclusive [`FileExt::lock_exclusive`] and explicit
//! [`FileExt::unlock`]. `fs2`'s own Unix backend is `libc::flock(fd, LOCK_EX |
//! LOCK_UN)` with no `EINTR` retry, so this invokes the same syscall with the
//! same flags — a behaviorally identical replacement with no new dependency.
//! When the MSRV moves past 1.89, prefer `std::fs::File`'s inherent
//! `lock`/`lock_shared`/`unlock` and delete this module.

use std::fs::File;
use std::io;

#[cfg(not(unix))]
compile_error!(
    "file_lock is unix-only: uhm's recovery paths are already unix-only and \
     there is no Windows build target. Add a non-unix implementation here \
     before targeting another platform."
);

#[cfg(unix)]
use std::os::unix::io::{AsRawFd, RawFd};

/// Advisory file-locking surface, mirroring the subset of `fs2::FileExt` in use.
pub trait FileExt {
    /// Block until this process holds an exclusive advisory lock.
    fn lock_exclusive(&self) -> io::Result<()>;
    /// Release a held advisory lock.
    fn unlock(&self) -> io::Result<()>;
}

#[cfg(unix)]
impl FileExt for File {
    fn lock_exclusive(&self) -> io::Result<()> {
        flock(self.as_raw_fd(), libc::LOCK_EX)
    }

    fn unlock(&self) -> io::Result<()> {
        flock(self.as_raw_fd(), libc::LOCK_UN)
    }
}

/// Acquire or release an advisory lock via `flock(2)`. Mirrors `fs2`'s Unix
/// backend exactly: no `EINTR` retry; surface the OS error on a negative return.
#[cfg(unix)]
fn flock(fd: RawFd, operation: i32) -> io::Result<()> {
    // SAFETY: `fd` is a valid open descriptor borrowed from a live `File`, and
    // `operation` is one of the documented `LOCK_*` constants.
    let rc = unsafe { libc::flock(fd, operation) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn lock_exclusive_blocks_until_the_holder_releases() {
        use std::sync::mpsc;
        use std::thread;
        use std::time::Duration;

        let path = std::env::temp_dir().join(format!(
            "uhm-file-lock-{}-{}.lock",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_file(&path);

        let holder = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .expect("open holder lock file");
        holder.lock_exclusive().expect("acquire holder lock");

        let (tx, rx) = mpsc::channel::<()>();
        let waiter_path = path.clone();
        let waiter = thread::spawn(move || {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&waiter_path)
                .expect("open waiter lock file");
            // Blocks until the holder releases the lock.
            file.lock_exclusive().expect("acquire waiter lock");
            tx.send(()).expect("signal acquisition");
            file.unlock().expect("release waiter lock");
        });

        // While the holder keeps the lock, the waiter must not have acquired it.
        thread::sleep(Duration::from_millis(200));
        assert!(
            rx.try_recv().is_err(),
            "lock_exclusive returned while another holder still held the lock"
        );

        // Closing the holder's descriptor releases the lock; the waiter proceeds.
        drop(holder);
        assert!(
            rx.recv_timeout(Duration::from_secs(5)).is_ok(),
            "lock_exclusive never completed after the holder released"
        );
        waiter.join().expect("waiter thread panicked");

        let _ = std::fs::remove_file(&path);
    }
}
