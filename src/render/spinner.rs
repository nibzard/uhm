//! Braille "dots" spinner on stderr, no-op when stderr isn't a terminal.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub struct Spinner {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Spinner {
    pub fn start(msg: &str) -> Spinner {
        if !std::io::stderr().is_terminal() || !crate::render::ansi::motion_enabled() {
            return Spinner {
                stop: Arc::new(AtomicBool::new(true)),
                handle: None,
            };
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let msg = msg.to_string();
        let handle = thread::spawn(move || {
            let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
            let mut i = 0usize;
            let mut err = std::io::stderr();
            while !stop2.load(Ordering::Relaxed) {
                let _ = write!(err, "\r{} {} ", frames[i % frames.len()], msg);
                let _ = err.flush();
                i = i.wrapping_add(1);
                thread::sleep(Duration::from_millis(80));
            }
            let _ = write!(err, "\r\x1b[K");
            let _ = err.flush();
        });
        Spinner {
            stop,
            handle: Some(handle),
        }
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.stop();
    }
}
