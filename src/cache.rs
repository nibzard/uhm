//! Response cache with explicit provenance and local-policy reclassification.

use crate::clock::{Clock as _, SystemClock};
use crate::dirs;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

#[derive(Serialize)]
struct Provenance<'a> {
    provider: &'a str,
    api_family: &'static str,
    endpoint: &'static str,
    adapter_contract_version: u32,
    selection_policy_version: u32,
    qualification_policy_version: u32,
    evidence_manifest_hash: String,
    selection_mode: &'a str,
    model: &'a str,
    shell: &'a str,
    prompt_version: u32,
    action_schema_version: u32,
    program_contract: &'static str,
    policy_version: u32,
    context_policy_version: u32,
    context_mode: &'a str,
    max_tokens: u32,
    reasoning_effort: &'a str,
    context_hash: &'a str,
    route: &'a str,
    input_hash: &'a str,
    request: &'a str,
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    version: u32,
    provider: crate::provider::ProviderId,
    api_family: String,
    written_at: u64,
    key: String,
    proposal: String,
}

fn now() -> u64 {
    SystemClock.unix_seconds()
}

#[allow(clippy::too_many_arguments)]
pub fn key_hash(
    provider: crate::provider::ProviderId,
    selection_mode: crate::config::SelectionMode,
    model: &str,
    shell: &str,
    max_tokens: u32,
    reasoning_effort: &str,
    context_mode: &str,
    context_hash: &str,
    route: &str,
    input_hash: &str,
    request: &str,
) -> String {
    key_hash_with_versions(
        model,
        shell,
        max_tokens,
        reasoning_effort,
        context_mode,
        context_hash,
        route,
        input_hash,
        request,
        provider.as_str(),
        provider.adapter().endpoint(),
        crate::provider::ADAPTER_CONTRACT_VERSION,
        crate::model_selection::SELECTION_POLICY_VERSION,
        crate::capabilities::QUALIFICATION_POLICY_VERSION,
        blake3::hash(crate::capabilities::MANIFEST_BYTES)
            .to_hex()
            .as_ref(),
        match selection_mode {
            crate::config::SelectionMode::Fixed => "fixed",
            crate::config::SelectionMode::Evidence => "evidence",
        },
        crate::prompt::PROMPT_VERSION,
        crate::prompt::ACTION_SCHEMA_VERSION,
        crate::contract::PROGRAM_CONTRACT,
        provider.adapter().api_family(),
    )
}

#[allow(clippy::too_many_arguments)]
fn key_hash_with_versions(
    model: &str,
    shell: &str,
    max_tokens: u32,
    reasoning_effort: &str,
    context_mode: &str,
    context_hash: &str,
    route: &str,
    input_hash: &str,
    request: &str,
    provider: &str,
    endpoint: &'static str,
    adapter_contract_version: u32,
    selection_policy_version: u32,
    qualification_policy_version: u32,
    evidence_manifest_hash: &str,
    selection_mode: &str,
    prompt_version: u32,
    action_schema_version: u32,
    program_contract: &'static str,
    api_family: &'static str,
) -> String {
    let value = Provenance {
        provider,
        api_family,
        endpoint,
        adapter_contract_version,
        selection_policy_version,
        qualification_policy_version,
        evidence_manifest_hash: evidence_manifest_hash.into(),
        selection_mode,
        model,
        shell,
        prompt_version,
        action_schema_version,
        program_contract,
        policy_version: crate::safety::DENY_VERSION,
        context_policy_version: crate::context::POLICY_VERSION,
        context_mode,
        max_tokens,
        reasoning_effort,
        context_hash,
        route,
        input_hash,
        request,
    };
    let bytes = serde_json::to_vec(&value).expect("cache provenance is serializable");
    blake3::hash(&bytes).to_hex().to_string()
}

pub fn get(
    cache_dir: &Path,
    enabled: bool,
    ttl_secs: u64,
    key: &str,
    provider: crate::provider::ProviderId,
) -> Option<String> {
    if !enabled {
        return None;
    }
    let text = std::fs::read_to_string(cache_dir.join(format!("{}.json", key))).ok()?;
    let entry: CacheEntry = serde_json::from_str(&text).ok()?;
    if entry.version != 2
        || entry.provider != provider
        || entry.api_family != provider.adapter().api_family()
        || entry.key != key
        || now().saturating_sub(entry.written_at) > ttl_secs
    {
        return None;
    }
    Some(entry.proposal)
}

pub fn put(
    cache_dir: &Path,
    enabled: bool,
    key: &str,
    proposal: &str,
    provider: crate::provider::ProviderId,
) -> Result<(), String> {
    if !enabled {
        return Ok(());
    }
    dirs::ensure_private_dir(cache_dir)?;
    let path = cache_dir.join(format!("{}.json", key));
    let entry = CacheEntry {
        version: 2,
        provider,
        api_family: provider.adapter().api_family().into(),
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

    #[allow(clippy::too_many_arguments)]
    fn key(
        model: &str,
        shell: &str,
        max_tokens: u32,
        reasoning_effort: &str,
        context_mode: &str,
        context_hash: &str,
        route: &str,
        input_hash: &str,
        request: &str,
    ) -> String {
        key_hash(
            crate::provider::ProviderId::Openai,
            crate::config::SelectionMode::Fixed,
            model,
            shell,
            max_tokens,
            reasoning_effort,
            context_mode,
            context_hash,
            route,
            input_hash,
            request,
        )
    }

    #[test]
    fn cache_key_changes_with_semantic_inputs() {
        let a = key(
            "m", "sh", 10, "low", "full", "ctx", "auto", "input", "request",
        );
        let b = key(
            "m", "sh", 11, "low", "full", "ctx", "auto", "input", "request",
        );
        assert_ne!(a, b);
        let c = key(
            "m", "sh", 10, "low", "minimal", "ctx", "auto", "input", "request",
        );
        assert_ne!(a, c);
        assert_ne!(
            a,
            key("other", "sh", 10, "low", "full", "ctx", "auto", "input", "request")
        );
        assert_ne!(
            a,
            key("m", "sh", 10, "high", "full", "ctx", "auto", "input", "request")
        );
        assert_ne!(
            a,
            key("m", "sh", 10, "low", "full", "other", "auto", "input", "request")
        );
        assert_ne!(
            a,
            key("m", "sh", 10, "low", "full", "ctx", "auto", "input", "other")
        );
        assert_ne!(
            a,
            key("m", "sh", 10, "low", "full", "ctx", "run", "input", "request")
        );
        assert_ne!(
            a,
            key("m", "sh", 10, "low", "full", "ctx", "auto", "other", "request")
        );
        assert_ne!(
            a,
            key_hash(
                crate::provider::ProviderId::Cerebras,
                crate::config::SelectionMode::Fixed,
                "m",
                "sh",
                10,
                "low",
                "full",
                "ctx",
                "auto",
                "input",
                "request",
            )
        );
        assert_ne!(
            a,
            key_hash(
                crate::provider::ProviderId::Openai,
                crate::config::SelectionMode::Evidence,
                "m",
                "sh",
                10,
                "low",
                "full",
                "ctx",
                "auto",
                "input",
                "request",
            )
        );
        let versioned = |prompt_version, action_version, program_contract, api_family| {
            key_hash_with_versions(
                "m",
                "sh",
                10,
                "low",
                "full",
                "ctx",
                "auto",
                "input",
                "request",
                "openai",
                crate::provider::openai::ENDPOINT,
                crate::provider::ADAPTER_CONTRACT_VERSION,
                crate::model_selection::SELECTION_POLICY_VERSION,
                crate::capabilities::QUALIFICATION_POLICY_VERSION,
                "manifest",
                "fixed",
                prompt_version,
                action_version,
                program_contract,
                api_family,
            )
        };
        for changed in [
            versioned(
                crate::prompt::PROMPT_VERSION + 1,
                crate::prompt::ACTION_SCHEMA_VERSION,
                crate::contract::PROGRAM_CONTRACT,
                crate::provider::openai::API_FAMILY,
            ),
            versioned(
                crate::prompt::PROMPT_VERSION,
                crate::prompt::ACTION_SCHEMA_VERSION + 1,
                crate::contract::PROGRAM_CONTRACT,
                crate::provider::openai::API_FAMILY,
            ),
            versioned(
                crate::prompt::PROMPT_VERSION,
                crate::prompt::ACTION_SCHEMA_VERSION,
                "other_contract",
                crate::provider::openai::API_FAMILY,
            ),
            versioned(
                crate::prompt::PROMPT_VERSION,
                crate::prompt::ACTION_SCHEMA_VERSION,
                crate::contract::PROGRAM_CONTRACT,
                "other_api",
            ),
        ] {
            assert_ne!(a, changed);
        }
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

    #[test]
    fn old_or_cross_provider_envelopes_miss() {
        let dir = tempfile::tempdir().unwrap();
        let key = "abc";
        std::fs::write(
            dir.path().join("abc.json"),
            r#"{"written_at":9999999999,"key":"abc","proposal":"old"}"#,
        )
        .unwrap();
        assert!(get(
            dir.path(),
            true,
            u64::MAX,
            key,
            crate::provider::ProviderId::Openai
        )
        .is_none());
        put(
            dir.path(),
            true,
            key,
            "new",
            crate::provider::ProviderId::Openai,
        )
        .unwrap();
        assert_eq!(
            get(
                dir.path(),
                true,
                u64::MAX,
                key,
                crate::provider::ProviderId::Openai
            )
            .as_deref(),
            Some("new")
        );
        assert!(get(
            dir.path(),
            true,
            u64::MAX,
            key,
            crate::provider::ProviderId::Cerebras
        )
        .is_none());
    }
}
