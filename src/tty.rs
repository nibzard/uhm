//! Cooked terminal interaction seam, independent of piped stdin.

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
