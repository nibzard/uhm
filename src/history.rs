//! Bounded metadata-only execution receipts. Content never enters this schema.

use crate::dirs;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub schema_version: u32,
    pub run_id: String,
    pub timestamp: u64,
    pub app_version: String,
    pub mode: String,
    pub context_mode: String,
    pub route: String,
    pub prompt_schema_version: u32,
    pub declared_effects: Vec<String>,
    pub detected_effects: Vec<String>,
    pub decision: String,
    pub execution_attempted: bool,
    pub exit_category: String,
    pub signal: Option<i32>,
    pub latency_bucket: String,
    pub cache_state: String,
    pub second_turn_used: bool,
}

pub fn run_id() -> String {
    let seed = format!(
        "{}:{}:{:?}",
        now_secs(),
        std::process::id(),
        std::thread::current().id()
    );
    blake3::hash(seed.as_bytes()).to_hex()[..20].to_string()
}
pub fn now_secs() -> u64 {
    use crate::clock::{Clock as _, SystemClock};
    SystemClock.unix_seconds()
}

fn paths(data: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    (data.join("history.jsonl"), data.join("history.lock"))
}
fn lock(data: &Path) -> Result<std::fs::File, String> {
    dirs::ensure_private_dir(data)?;
    let (_, p) = paths(data);
    let mut o = std::fs::OpenOptions::new();
    o.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let f = o
        .open(&p)
        .map_err(|e| format!("open history lock: {}", e))?;
    f.lock_exclusive()
        .map_err(|e| format!("lock history: {}", e))?;
    Ok(f)
}

pub fn append(
    data: &Path,
    receipt: &Receipt,
    max_records: usize,
    max_age_days: u64,
) -> Result<(), String> {
    let _guard = lock(data)?;
    let (path, _) = paths(data);
    let mut records = read_valid(&path);
    records.push(serde_json::to_value(receipt).map_err(|e| format!("serialize receipt: {}", e))?);
    let cutoff = now_secs().saturating_sub(max_age_days * 86_400);
    records.retain(|v| v["timestamp"].as_u64().unwrap_or(0) >= cutoff);
    if records.len() > max_records {
        records.drain(..records.len() - max_records);
    }
    write_all(&path, &records)
}
fn write_all(path: &Path, records: &[Value]) -> Result<(), String> {
    let parent = path.parent().ok_or("history path has no parent")?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("create history temp: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    for value in records {
        serde_json::to_writer(&mut tmp, value).map_err(|e| e.to_string())?;
        tmp.write_all(b"\n").map_err(|e| e.to_string())?;
    }
    tmp.as_file().sync_all().map_err(|e| e.to_string())?;
    tmp.persist(path).map_err(|e| e.error.to_string())?;
    Ok(())
}
fn read_valid(path: &Path) -> Vec<Value> {
    let mut bytes = Vec::new();
    if std::fs::File::open(path)
        .and_then(|mut f| f.read_to_end(&mut bytes))
        .is_err()
    {
        return vec![];
    }
    bytes
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .filter_map(|l| serde_json::from_slice(l).ok())
        .collect()
}
pub fn recent(data: &Path, n: usize) -> Vec<Value> {
    let _guard = lock(data).ok();
    let (path, _) = paths(data);
    let all = read_valid(&path);
    all[all.len().saturating_sub(n)..].to_vec()
}
pub fn clear(data: &Path) -> Result<(), String> {
    let guard = lock(data)?;
    let (path, _) = paths(data);
    let mut o = std::fs::OpenOptions::new();
    o.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(path).map_err(|e| e.to_string())?;
    f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    drop(guard);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn receipt() -> Receipt {
        Receipt {
            schema_version: 1,
            run_id: "opaque".into(),
            timestamp: now_secs(),
            app_version: "0.1".into(),
            mode: "auto".into(),
            context_mode: "minimal".into(),
            route: "run_shell".into(),
            prompt_schema_version: 1,
            declared_effects: vec![],
            detected_effects: vec![],
            decision: "run".into(),
            execution_attempted: true,
            exit_category: "success".into(),
            signal: None,
            latency_bucket: "lt_1s".into(),
            cache_state: "miss".into(),
            second_turn_used: false,
        }
    }
    #[test]
    fn bounded_and_content_free() {
        let d = tempfile::tempdir().unwrap();
        for _ in 0..4 {
            append(d.path(), &receipt(), 2, 30).unwrap();
        }
        let v = recent(d.path(), 10);
        assert_eq!(v.len(), 2);
        for item in v {
            let o = item.as_object().unwrap();
            for key in [
                "intent",
                "command",
                "cwd",
                "context",
                "clarification",
                "feedback",
                "answer",
                "stdout",
                "stderr",
                "diagnostics",
            ] {
                assert!(!o.contains_key(key));
            }
        }
    }
    #[test]
    fn ignores_interrupted_final_line() {
        let d = tempfile::tempdir().unwrap();
        append(d.path(), &receipt(), 10, 30).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(d.path().join("history.jsonl"))
            .unwrap()
            .write_all(b"{broken")
            .unwrap();
        assert_eq!(recent(d.path(), 10).len(), 1);
    }

    #[test]
    fn concurrent_writers_share_the_dedicated_lock() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().to_path_buf();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let root = root.clone();
                std::thread::spawn(move || append(&root, &receipt(), 20, 30).unwrap())
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(recent(&root, 20).len(), 8);
        clear(&root).unwrap();
        assert!(recent(&root, 20).is_empty());
    }
}
