//! Cooked terminal interaction seam, independent of piped stdin.

use std::io::IsTerminal;

pub trait Interaction {
    fn interactive(&self) -> bool;
    fn confirm(&self) -> bool;
}

pub struct SystemInteraction;

impl Interaction for SystemInteraction {
    fn interactive(&self) -> bool {
        std::io::stderr().is_terminal()
    }

    fn confirm(&self) -> bool {
        read_line_cooked().is_some_and(|answer| matches!(answer.as_str(), "y" | "yes"))
    }
}

pub fn read_line_cooked() -> Option<String> {
    use std::io::BufRead as _;

    let tty = std::fs::OpenOptions::new()
        .read(true)
        .open("/dev/tty")
        .ok()?;
    let mut line = String::new();
    match std::io::BufReader::new(tty).read_line(&mut line) {
        Ok(0) => None,
        Ok(_) => Some(line.trim().to_string()),
        Err(_) => None,
    }
}
