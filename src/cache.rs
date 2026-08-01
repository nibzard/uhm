//! Response cache with explicit provenance and local-policy reclassification.

use crate::clock::{Clock as _, SystemClock};
use crate::dirs;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

#[derive(Serialize)]
struct Provenance<'a> {
    api_family: &'static str,
    model: &'a str,
    base_url: &'a str,
    shell: &'a str,
    prompt_version: u32,
    policy_version: u32,
    context_policy_version: u32,
    context_mode: &'a str,
    max_tokens: u32,
    reasoning_effort: &'a str,
    context_hash: &'a str,
    request: &'a str,
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    written_at: u64,
    key: String,
    proposal: String,
}

fn now() -> u64 {
    SystemClock.unix_seconds()
}

#[allow(clippy::too_many_arguments)]
pub fn key_hash(
    model: &str,
    base_url: &str,
    shell: &str,
    max_tokens: u32,
    reasoning_effort: &str,
    context_mode: &str,
    context_hash: &str,
    request: &str,
) -> String {
    let value = Provenance {
        api_family: "chat_completions",
        model,
        base_url,
        shell,
        prompt_version: crate::prompt::PROMPT_VERSION,
        policy_version: crate::safety::DENY_VERSION,
        context_policy_version: 1,
        context_mode,
        max_tokens,
        reasoning_effort,
        context_hash,
        request,
    };
    let bytes = serde_json::to_vec(&value).expect("cache provenance is serializable");
    blake3::hash(&bytes).to_hex().to_string()
}

pub fn get(cache_dir: &Path, enabled: bool, ttl_secs: u64, key: &str) -> Option<String> {
    if !enabled {
        return None;
    }
    let text = std::fs::read_to_string(cache_dir.join(format!("{}.json", key))).ok()?;
    let entry: CacheEntry = serde_json::from_str(&text).ok()?;
    if entry.key != key || now().saturating_sub(entry.written_at) > ttl_secs {
        return None;
    }
    Some(entry.proposal)
}

pub fn put(cache_dir: &Path, enabled: bool, key: &str, proposal: &str) -> Result<(), String> {
    if !enabled {
        return Ok(());
    }
    dirs::ensure_private_dir(cache_dir)?;
    let path = cache_dir.join(format!("{}.json", key));
    let entry = CacheEntry {
        written_at: now(),
        key: key.into(),
        proposal: proposal.into(),
    };
    let bytes = serde_json::to_vec(&entry).map_err(|e| format!("serialize cache entry: {}", e))?;
    write_private(&path, &bytes)
}

fn write_private(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("cache path {} has no parent", path.display()))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("create cache temporary file in {}: {}", parent.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("set cache permissions: {}", e))?;
    }
    file.write_all(contents)
        .map_err(|e| format!("write cache file: {}", e))?;
    file.as_file()
        .sync_all()
        .map_err(|e| format!("sync cache file: {}", e))?;
    file.persist(path)
        .map_err(|e| format!("publish cache file {}: {}", path.display(), e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_changes_with_semantic_inputs() {
        let a = key_hash("m", "url", "sh", 10, "low", "full", "ctx", "request");
        let b = key_hash("m", "url", "sh", 11, "low", "full", "ctx", "request");
        assert_ne!(a, b);
        let c = key_hash(
            "m",
            "url",
            "sh",
            10,
            "low",
            "request_only",
            "ctx",
            "request",
        );
        assert_ne!(a, c);
        assert_ne!(
            a,
            key_hash("other", "url", "sh", 10, "low", "full", "ctx", "request")
        );
        assert_ne!(
            a,
            key_hash("m", "other-url", "sh", 10, "low", "full", "ctx", "request")
        );
        assert_ne!(
            a,
            key_hash("m", "url", "sh", 10, "high", "full", "ctx", "request")
        );
        assert_ne!(
            a,
            key_hash("m", "url", "sh", 10, "low", "full", "other", "request")
        );
        assert_ne!(
            a,
            key_hash("m", "url", "sh", 10, "low", "full", "ctx", "other")
        );
    }

    #[cfg(unix)]
    #[test]
    fn cache_publish_is_private_and_replaces_atomically() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("entry.json");
        write_private(&path, b"first").unwrap();
        write_private(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
