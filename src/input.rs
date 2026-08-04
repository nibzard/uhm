//! Bounded, byte-exact stdin spool.

use serde_json::{json, Value};
use std::io::{IsTerminal, Read, Write};

#[derive(Debug, Clone, Default)]
pub struct Spool {
    bytes: Vec<u8>,
    piped: bool,
}
impl Spool {
    pub fn read(max: usize, first_byte_timeout_ms: u64) -> Result<Self, String> {
        if std::io::stdin().is_terminal() {
            return Ok(Self::default());
        }
        Self::read_piped(
            0,
            std::io::stdin().lock(),
            max,
            first_byte_timeout_ms,
            &mut std::io::stderr().lock(),
        )
    }
    /// Bounded read of a non-terminal descriptor. The producer has
    /// `first_byte_timeout_ms` to deliver its first byte; a descriptor that
    /// stays silent yields the empty spool and one notice line. Once a first
    /// byte is ready the stream is read to EOF under the byte cap with no
    /// further deadline, so a slow streaming producer is never truncated.
    fn read_piped(
        fd: libc::c_int,
        mut source: impl Read,
        max: usize,
        first_byte_timeout_ms: u64,
        notice: &mut impl Write,
    ) -> Result<Self, String> {
        if !first_byte_within(fd, first_byte_timeout_ms)? {
            let _ = writeln!(
                notice,
                "uhm: stdin is open but sent nothing within {} ms; proceeding without piped input (use </dev/null to declare no input)",
                first_byte_timeout_ms
            );
            return Ok(Self::default());
        }
        let mut bytes = Vec::new();
        source
            .by_ref()
            .take((max + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("read stdin: {}", e))?;
        if bytes.len() > max {
            return Err(format!("stdin exceeds configured {} byte limit", max));
        }
        Ok(Self { bytes, piped: true })
    }
    #[cfg(test)]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self { bytes, piped: true }
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn is_piped(&self) -> bool {
        self.piped
    }
    #[cfg(test)]
    pub fn model_value(&self) -> Value {
        self.model_value_for(false, None)
    }
    pub fn model_value_for(&self, local_only: bool, declared_format: Option<&str>) -> Value {
        if local_only {
            return json!({
                "present":self.piped,
                "byte_count":self.bytes.len(),
                "utf8":std::str::from_utf8(&self.bytes).is_ok(),
                "local_only":true,
                "declared_format":declared_format
            });
        }
        match std::str::from_utf8(&self.bytes) {
            Ok(text) => {
                json!({"present":self.piped,"byte_count":self.bytes.len(),"utf8":true,"text":text,"local_only":false,"declared_format":declared_format})
            }
            Err(_) => {
                json!({"present":self.piped,"byte_count":self.bytes.len(),"utf8":false,"local_only":false,"declared_format":declared_format})
            }
        }
    }
}

/// Wait for the descriptor to become readable — a first byte or EOF — within
/// the deadline. `true` means a read can proceed immediately; `false` means the
/// producer stayed silent for the whole bound.
#[cfg(unix)]
fn first_byte_within(fd: libc::c_int, timeout_ms: u64) -> Result<bool, String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let mut request = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let wait = remaining.as_millis().min(i32::MAX as u128) as libc::c_int;
        let ready = unsafe { libc::poll(&mut request, 1, wait) };
        if ready > 0 {
            return Ok(true);
        }
        if ready == 0 {
            return Ok(false);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(format!("poll stdin: {}", error));
        }
    }
}

#[cfg(not(unix))]
fn first_byte_within(_fd: libc::c_int, _timeout_ms: u64) -> Result<bool, String> {
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    fn pipe() -> (libc::c_int, libc::c_int) {
        let mut fds = [0 as libc::c_int; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        (fds[0], fds[1])
    }
    #[cfg(unix)]
    fn write_bytes(fd: libc::c_int, bytes: &[u8]) {
        let written = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
        assert_eq!(written, bytes.len() as isize);
    }
    #[cfg(unix)]
    fn reader(fd: libc::c_int) -> std::fs::File {
        use std::os::unix::io::FromRawFd;
        unsafe { std::fs::File::from_raw_fd(fd) }
    }

    #[cfg(unix)]
    #[test]
    fn an_idle_open_pipe_yields_the_empty_spool_and_the_notice_within_the_deadline() {
        let (read_fd, write_fd) = pipe();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let source = reader(read_fd);
            let mut notice = Vec::new();
            let result = Spool::read_piped(read_fd, source, 1024, 150, &mut notice);
            let _ = sender.send((result, notice));
        });
        let (result, notice) = receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("an idle open pipe must not block past the first-byte deadline");
        let spool = result.unwrap();
        assert!(spool.bytes().is_empty());
        assert!(!spool.is_piped());
        let notice = String::from_utf8(notice).unwrap();
        assert!(notice.contains("150 ms"), "{notice}");
        assert!(notice.contains("without piped input"), "{notice}");
        unsafe { libc::close(write_fd) };
    }

    #[cfg(unix)]
    #[test]
    fn a_producer_that_starts_in_time_streams_to_completion_without_truncation() {
        let (read_fd, write_fd) = pipe();
        let producer = std::thread::spawn(move || {
            write_bytes(write_fd, b"first");
            std::thread::sleep(std::time::Duration::from_millis(400));
            write_bytes(write_fd, b" second");
            unsafe { libc::close(write_fd) };
        });
        let mut notice = Vec::new();
        let spool = Spool::read_piped(read_fd, reader(read_fd), 1024, 150, &mut notice).unwrap();
        assert_eq!(spool.bytes(), b"first second");
        assert!(spool.is_piped());
        assert!(notice.is_empty(), "{}", String::from_utf8_lossy(&notice));
        producer.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_first_byte_after_the_deadline_is_the_accepted_false_negative() {
        let (read_fd, write_fd) = pipe();
        let producer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            // The reader may already have given up; a failed late write is
            // part of the pinned trade-off, so its result is not asserted.
            let late = b"late";
            let _ = unsafe { libc::write(write_fd, late.as_ptr().cast(), late.len()) };
            unsafe { libc::close(write_fd) };
        });
        let mut notice = Vec::new();
        let spool = Spool::read_piped(read_fd, reader(read_fd), 1024, 100, &mut notice).unwrap();
        assert!(spool.bytes().is_empty());
        assert!(!spool.is_piped());
        assert!(
            String::from_utf8_lossy(&notice).contains("without piped input"),
            "{}",
            String::from_utf8_lossy(&notice)
        );
        producer.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_spool_beyond_the_byte_cap_keeps_its_explicit_error() {
        let (read_fd, write_fd) = pipe();
        write_bytes(write_fd, b"123456789");
        unsafe { libc::close(write_fd) };
        let mut notice = Vec::new();
        let error = Spool::read_piped(read_fd, reader(read_fd), 8, 150, &mut notice).unwrap_err();
        assert_eq!(error, "stdin exceeds configured 8 byte limit");
    }

    #[test]
    fn non_utf8_never_enters_json() {
        let s = Spool::from_bytes(vec![0xff, 0]);
        let v = s.model_value();
        assert_eq!(v["utf8"], false);
        assert!(v.get("text").is_none());
        assert_eq!(s.bytes(), &[0xff, 0]);
    }

    #[test]
    fn local_input_never_places_content_in_model_json() {
        let private = "sentinel local content";
        let spool = Spool::from_bytes(private.as_bytes().to_vec());
        let value = spool.model_value_for(true, Some("text/plain"));
        assert_eq!(value["local_only"], true);
        assert_eq!(value["byte_count"], private.len());
        assert_eq!(value["declared_format"], "text/plain");
        assert!(!value.to_string().contains(private));
        assert!(value.get("text").is_none());
    }
}
