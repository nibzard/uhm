//! Optional append-only command history. Requests are never recorded.

use crate::clock::{Clock as _, SystemClock};
use crate::dirs;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Write;
use std::path::Path;

#[derive(Serialize, Deserialize)]
pub struct Entry {
    pub ts: f64,
    pub model: String,
    pub kind: String,
    pub command: String,
    pub effects: Vec<String>,
    pub ran: bool,
    pub exit: i32,
}

pub trait ReceiptWriter {
    fn append(&self, data_dir: &Path, entry: &Entry) -> Result<(), String>;
}

pub struct JsonlReceiptWriter;

impl ReceiptWriter for JsonlReceiptWriter {
    fn append(&self, data_dir: &Path, entry: &Entry) -> Result<(), String> {
        append(data_dir, entry)
    }
}

pub fn now_secs() -> f64 {
    SystemClock.unix_seconds() as f64
}

pub fn append(data_dir: &Path, entry: &Entry) -> Result<(), String> {
    dirs::ensure_private_dir(data_dir)?;
    append_at(&data_dir.join("history.jsonl"), entry)
}

fn append_at(path: &Path, entry: &Entry) -> Result<(), String> {
    let mut line = serde_json::to_string(entry).map_err(|e| format!("serialize history: {}", e))?;
    line.push('\n');
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("open history {}: {}", path.display(), e))?;
    fs2::FileExt::lock_exclusive(&file)
        .map_err(|e| format!("lock history {}: {}", path.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("set history permissions: {}", e))?;
    }
    file.write_all(line.as_bytes())
        .map_err(|e| format!("append history: {}", e))?;
    fs2::FileExt::unlock(&file).map_err(|e| format!("unlock history {}: {}", path.display(), e))
}

pub fn recent(data_dir: &Path, n: usize) -> Vec<Value> {
    let text = match std::fs::read_to_string(data_dir.join("history.jsonl")) {
        Ok(text) => text,
        Err(_) => return vec![],
    };
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..]
        .iter()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn history_file_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        append_at(
            &path,
            &Entry {
                ts: 1.0,
                model: "test".into(),
                kind: "shell".into(),
                command: "true".into(),
                effects: vec!["read_local".into()],
                ran: true,
                exit: 0,
            },
        )
        .unwrap();
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
