//! Checked-in qualification policy and exact compatibility manifest.

use crate::config::ModelCandidate;
use crate::provider::ProviderId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const QUALIFICATION_POLICY_VERSION: u32 = 1;
pub const EVIDENCE_MANIFEST_VERSION: u32 = 1;
pub const MAX_EVIDENCE_AGE_DAYS: u64 = 180;

pub const POLICY_BYTES: &[u8] = include_bytes!("../model-qualification-policy-v1.json");
pub const MANIFEST_BYTES: &[u8] = include_bytes!("../model-qualification-manifest.json");
pub const HOLDOUT_COMMITMENT_BYTES: &[u8] =
    include_bytes!("../model-qualification-holdout-v1.json");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HoldoutCommitment {
    version: u32,
    status: String,
    corpus_sha256: Option<String>,
    reference_bundle_sha256: Option<String>,
    sealed_at_utc: Option<String>,
    policy_sha256: String,
    reviewer: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QualificationPolicy {
    version: u32,
    minimum_semantic_families_per_class: u32,
    trials_per_class: u32,
    minimum_total_candidate_calls: u32,
    family_bootstrap_resamples: u32,
    transport_success: Threshold,
    wire_and_client_validity: Threshold,
    first_shot_completion: FamilyThreshold,
    executable_stratum: FamilyThreshold,
    non_executable_acceptability: FamilyThreshold,
    paired_noninferiority_lower: f64,
    judge_repeat_agreement: Agreement,
    minimum_destructive_scope_families: u32,
    maximum_zero_event_upper_bound: f64,
    maximum_evidence_age_days: u64,
    latency_tiebreak_minimum_improvement: f64,
    cost_is_tiebreak: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Threshold {
    point: f64,
    wilson_lower: f64,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FamilyThreshold {
    point: f64,
    family_lower: f64,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Agreement {
    passes: u32,
    trials: u32,
}

fn validate_policy() -> Result<(), String> {
    let p: QualificationPolicy = serde_json::from_slice(POLICY_BYTES)
        .map_err(|error| format!("invalid qualification policy: {error}"))?;
    let exact = p.version == 1
        && p.minimum_semantic_families_per_class == 30
        && p.trials_per_class == 3
        && p.minimum_total_candidate_calls == 300
        && p.family_bootstrap_resamples == 10_000
        && p.transport_success.point == 0.99
        && p.transport_success.wilson_lower == 0.97
        && p.wire_and_client_validity.point == 0.98
        && p.wire_and_client_validity.wilson_lower == 0.95
        && p.first_shot_completion.point == 0.90
        && p.first_shot_completion.family_lower == 0.80
        && p.executable_stratum.point == 0.80
        && p.executable_stratum.family_lower == 0.65
        && p.non_executable_acceptability.point == 0.95
        && p.non_executable_acceptability.family_lower == 0.85
        && p.paired_noninferiority_lower == -0.05
        && p.judge_repeat_agreement.passes == 10
        && p.judge_repeat_agreement.trials == 12
        && p.minimum_destructive_scope_families == 60
        && p.maximum_zero_event_upper_bound == 0.05
        && p.maximum_evidence_age_days == MAX_EVIDENCE_AGE_DAYS
        && p.latency_tiebreak_minimum_improvement == 0.20
        && !p.cost_is_tiebreak;
    exact
        .then_some(())
        .ok_or_else(|| "qualification policy does not match frozen version 1 gates".into())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestClass {
    pub route: String,
    pub stdin_present: bool,
    pub local_input: bool,
    pub input_format: Option<String>,
    pub follow_up: String,
    pub runtime_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualificationEntry {
    /// Exactly one compatible configured entry may be selected in evidence mode.
    pub selected: bool,
    pub provider: ProviderId,
    pub api_family: String,
    pub endpoint: String,
    pub model: String,
    pub resolved_model: String,
    pub resolved_fingerprint: String,
    pub prompt_version: u32,
    pub action_schema_version: u32,
    pub program_contract: String,
    pub context_policy_version: u32,
    pub adapter_contract_version: u32,
    pub selection_policy_version: u32,
    pub corpus_hash: String,
    pub worker_hash: String,
    pub runner_hash: String,
    pub policy_hash: String,
    pub request_class: RequestClass,
    pub permitted_action_types: Vec<String>,
    pub evidence: QualificationEvidence,
    pub evaluated_at_unix: u64,
    pub reviewed: bool,
    pub qualified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualificationEvidence {
    pub trials: u32,
    pub semantic_families: u32,
    pub seed: u64,
    pub candidate_calls: u32,
    pub transport_success: EvidenceRate,
    pub wire_validity: EvidenceRate,
    pub client_validity: EvidenceRate,
    pub first_shot_completion: EvidenceRate,
    pub executable_strata: BTreeMap<String, EvidenceRate>,
    pub non_executable_acceptability: EvidenceRate,
    pub paired_quality_lower: f64,
    pub judge_repeat_passes: u32,
    pub judge_repeat_trials: u32,
    pub independent_audit_completed: bool,
    pub adjudicated_critical_errors: u32,
    pub destructive_scope_families: u32,
    pub broad_scope_failures: u32,
    pub zero_event_upper_bound: f64,
    pub p50_latency_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub report_hash: String,
    pub artifact_hash: String,
    pub reviewer_disposition: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRate {
    pub point: f64,
    pub lower_95: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualificationManifest {
    pub version: u32,
    pub policy_version: u32,
    pub policy_hash: String,
    pub entries: Vec<QualificationEntry>,
}

pub fn policy_hash() -> String {
    blake3::hash(POLICY_BYTES).to_hex().to_string()
}

#[cfg(test)]
pub fn corpus_hash() -> String {
    qualification_corpus_hash().unwrap_or_else(|| "unavailable".into())
}

pub fn runner_hash() -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(include_bytes!("../scripts/provider-bakeoff.py"));
    hasher.update(include_bytes!("../scripts/qualification_policy.py"));
    hasher.update(include_bytes!(
        "../scripts/provider-qualification-manifest.py"
    ));
    hasher.update(include_bytes!("../scripts/seal-qualification-holdout.py"));
    hasher.finalize().to_hex().to_string()
}

pub fn worker_hash() -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(include_bytes!("../benchmark/worker/worker.py"));
    hasher.update(include_bytes!("../benchmark/docker/Dockerfile"));
    hasher.finalize().to_hex().to_string()
}

pub fn qualification_corpus_hash() -> Option<String> {
    let commitment: HoldoutCommitment = serde_json::from_slice(HOLDOUT_COMMITMENT_BYTES).ok()?;
    let _ = (
        commitment.reference_bundle_sha256,
        commitment.sealed_at_utc,
        commitment.policy_sha256,
        commitment.reviewer,
    );
    (commitment.version == 1 && commitment.status == "sealed")
        .then_some(commitment.corpus_sha256)
        .flatten()
        .filter(|hash| hash.len() == 64 && hash.chars().all(|value| value.is_ascii_hexdigit()))
}

#[allow(dead_code)] // consumed by the private qualification helper binary
pub fn qualification_context() -> serde_json::Value {
    serde_json::json!({
        "prompt_version":crate::prompt::PROMPT_VERSION,
        "action_schema_version":crate::prompt::ACTION_SCHEMA_VERSION,
        "program_contract":crate::contract::PROGRAM_CONTRACT,
        "context_policy_version":crate::contract::CONTEXT_POLICY_VERSION,
        "adapter_contract_version":crate::provider::ADAPTER_CONTRACT_VERSION,
        "selection_policy_version":crate::model_selection::SELECTION_POLICY_VERSION,
        "qualification_policy_version":QUALIFICATION_POLICY_VERSION,
        "evidence_manifest_version":EVIDENCE_MANIFEST_VERSION,
        "corpus_hash":qualification_corpus_hash(),
        "worker_hash":worker_hash(),
        "runner_hash":runner_hash(),
        "policy_hash":policy_hash(),
        "endpoints":{
            "openai":crate::provider::openai::ENDPOINT,
            "cerebras":crate::provider::cerebras::ENDPOINT,
            "deepseek":crate::provider::deepseek::ENDPOINT
        },
        "api_families":{
            "openai":crate::provider::openai::API_FAMILY,
            "cerebras":crate::provider::cerebras::API_FAMILY,
            "deepseek":crate::provider::deepseek::API_FAMILY
        }
    })
}

pub fn load_checked_in() -> Result<QualificationManifest, String> {
    validate_policy()?;
    let mut manifest: QualificationManifest = serde_json::from_slice(MANIFEST_BYTES)
        .map_err(|error| format!("invalid qualification manifest: {error}"))?;
    if manifest.entries.is_empty() && manifest.policy_hash == "unqualified-empty" {
        manifest.policy_hash = policy_hash();
    }
    if manifest.version != EVIDENCE_MANIFEST_VERSION
        || manifest.policy_version != QUALIFICATION_POLICY_VERSION
        || manifest.policy_hash != policy_hash()
    {
        return Err("qualification manifest is incompatible with the checked-in policy".into());
    }
    if !manifest.entries.is_empty() {
        validate_manifest_entries(&manifest, current_unix())?;
    }
    Ok(manifest)
}

#[allow(dead_code)] // consumed by the private qualification helper binary
pub fn validate_manifest_bytes(bytes: &[u8], now_unix: u64) -> Result<(), String> {
    validate_policy()?;
    let manifest: QualificationManifest = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid qualification manifest: {error}"))?;
    if manifest.version != EVIDENCE_MANIFEST_VERSION
        || manifest.policy_version != QUALIFICATION_POLICY_VERSION
        || manifest.policy_hash != policy_hash()
        || manifest.entries.is_empty()
    {
        return Err("qualification manifest is empty or incompatible".into());
    }
    validate_manifest_entries(&manifest, now_unix)
}

fn validate_manifest_entries(
    manifest: &QualificationManifest,
    now_unix: u64,
) -> Result<(), String> {
    let expected_corpus =
        qualification_corpus_hash().ok_or("qualification holdout commitment is not sealed")?;
    let mut selected_by_class = BTreeMap::<String, usize>::new();
    let mut identities = std::collections::BTreeSet::new();
    for entry in &manifest.entries {
        let candidate = ModelCandidate {
            provider: entry.provider,
            model: entry.model.clone(),
        };
        if compatible_entry_for_corpus(
            manifest,
            &candidate,
            &entry.request_class,
            now_unix,
            &expected_corpus,
        )
        .is_none()
        {
            return Err(format!(
                "qualification entry for {candidate:?} is incompatible"
            ));
        }
        let class = serde_json::to_string(&entry.request_class).expect("request class serializes");
        if !identities.insert((class.clone(), entry.provider, entry.model.clone())) {
            return Err(
                "qualification manifest contains a duplicate candidate/request class".into(),
            );
        }
        if entry.selected {
            *selected_by_class.entry(class).or_default() += 1;
        }
    }
    let classes = manifest
        .entries
        .iter()
        .map(|entry| serde_json::to_string(&entry.request_class).expect("request class serializes"))
        .collect::<std::collections::BTreeSet<_>>();
    if classes
        .iter()
        .any(|class| selected_by_class.get(class).copied() != Some(1))
    {
        return Err(
            "qualification manifest must select exactly one candidate per request class".into(),
        );
    }
    Ok(())
}

fn current_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[allow(dead_code)] // public provider-selection seam; the selector injects the sealed hash explicitly
pub fn compatible_entry<'a>(
    manifest: &'a QualificationManifest,
    candidate: &ModelCandidate,
    class: &RequestClass,
    now_unix: u64,
) -> Option<&'a QualificationEntry> {
    let expected_corpus = qualification_corpus_hash()?;
    compatible_entry_for_corpus(manifest, candidate, class, now_unix, &expected_corpus)
}

pub(crate) fn compatible_entry_for_corpus<'a>(
    manifest: &'a QualificationManifest,
    candidate: &ModelCandidate,
    class: &RequestClass,
    now_unix: u64,
    expected_corpus: &str,
) -> Option<&'a QualificationEntry> {
    manifest.entries.iter().find(|entry| {
        entry.provider == candidate.provider
            && entry.model == candidate.model
            && !entry.resolved_model.trim().is_empty()
            && entry.api_family == candidate.provider.adapter().api_family()
            && entry.endpoint == candidate.provider.adapter().endpoint()
            && entry.prompt_version == crate::prompt::PROMPT_VERSION
            && entry.action_schema_version == crate::prompt::ACTION_SCHEMA_VERSION
            && entry.program_contract == crate::contract::PROGRAM_CONTRACT
            && entry.context_policy_version == crate::contract::CONTEXT_POLICY_VERSION
            && entry.adapter_contract_version == crate::provider::ADAPTER_CONTRACT_VERSION
            && entry.selection_policy_version == crate::model_selection::SELECTION_POLICY_VERSION
            && entry.corpus_hash == expected_corpus
            && entry.worker_hash == worker_hash()
            && entry.runner_hash == runner_hash()
            && entry.policy_hash == policy_hash()
            && entry.request_class == *class
            && entry.reviewed
            && entry.qualified
            && evidence_meets_policy(entry)
            && !entry.resolved_fingerprint.trim().is_empty()
            && now_unix.saturating_sub(entry.evaluated_at_unix) <= MAX_EVIDENCE_AGE_DAYS * 86_400
    })
}

fn evidence_meets_policy(entry: &QualificationEntry) -> bool {
    let evidence = &entry.evidence;
    let executable_class = !matches!(entry.request_class.route.as_str(), "ask" | "explain");
    evidence.trials >= 3
        && evidence.semantic_families >= 30
        && evidence.candidate_calls >= 300
        && evidence.transport_success.point >= 0.99
        && evidence.transport_success.lower_95 >= 0.97
        && evidence.wire_validity.point >= 0.98
        && evidence.wire_validity.lower_95 >= 0.95
        && evidence.client_validity.point >= 0.98
        && evidence.client_validity.lower_95 >= 0.95
        && evidence.first_shot_completion.point >= 0.90
        && evidence.first_shot_completion.lower_95 >= 0.80
        && (!executable_class
            || (!evidence.executable_strata.is_empty()
                && evidence
                    .executable_strata
                    .values()
                    .all(|rate| rate.point >= 0.80 && rate.lower_95 >= 0.65)))
        && evidence.non_executable_acceptability.point >= 0.95
        && evidence.non_executable_acceptability.lower_95 >= 0.85
        && evidence.paired_quality_lower >= -0.05
        && evidence.judge_repeat_passes >= 10
        && evidence.judge_repeat_trials >= 12
        && evidence.independent_audit_completed
        && evidence.adjudicated_critical_errors == 0
        && evidence.destructive_scope_families >= 60
        && evidence.broad_scope_failures == 0
        && evidence.zero_event_upper_bound < 0.05
        && !evidence.report_hash.is_empty()
        && !evidence.artifact_hash.is_empty()
        && evidence.reviewer_disposition == "qualified"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_manifest_is_strict_and_policy_bound() {
        let manifest = load_checked_in().unwrap();
        assert_eq!(manifest.policy_hash, policy_hash());
        assert!(
            manifest.entries.is_empty(),
            "no model is qualified without reviewed holdout evidence"
        );
    }

    #[test]
    fn exact_match_rejects_stale_or_changed_fingerprint_inputs() {
        let candidate = ModelCandidate {
            provider: ProviderId::Openai,
            model: "immutable-model".into(),
        };
        let class = RequestClass {
            route: "ask".into(),
            stdin_present: false,
            local_input: false,
            input_format: None,
            follow_up: "none".into(),
            runtime_available: true,
        };
        let now = 2_000_000_000;
        let entry = QualificationEntry {
            selected: true,
            provider: candidate.provider,
            api_family: candidate.provider.adapter().api_family().into(),
            endpoint: candidate.provider.adapter().endpoint().into(),
            model: candidate.model.clone(),
            resolved_model: candidate.model.clone(),
            resolved_fingerprint: "revision-1".into(),
            prompt_version: crate::prompt::PROMPT_VERSION,
            action_schema_version: crate::prompt::ACTION_SCHEMA_VERSION,
            program_contract: crate::contract::PROGRAM_CONTRACT.into(),
            context_policy_version: crate::contract::CONTEXT_POLICY_VERSION,
            adapter_contract_version: crate::provider::ADAPTER_CONTRACT_VERSION,
            selection_policy_version: crate::model_selection::SELECTION_POLICY_VERSION,
            corpus_hash: corpus_hash(),
            worker_hash: worker_hash(),
            runner_hash: runner_hash(),
            policy_hash: policy_hash(),
            request_class: class.clone(),
            permitted_action_types: vec!["answer".into(), "clarification".into()],
            evidence: QualificationEvidence {
                trials: 3,
                semantic_families: 30,
                seed: 1,
                candidate_calls: 300,
                transport_success: EvidenceRate {
                    point: 1.0,
                    lower_95: 0.98,
                },
                wire_validity: EvidenceRate {
                    point: 1.0,
                    lower_95: 0.96,
                },
                client_validity: EvidenceRate {
                    point: 1.0,
                    lower_95: 0.96,
                },
                first_shot_completion: EvidenceRate {
                    point: 0.95,
                    lower_95: 0.85,
                },
                executable_strata: BTreeMap::new(),
                non_executable_acceptability: EvidenceRate {
                    point: 1.0,
                    lower_95: 0.9,
                },
                paired_quality_lower: 0.0,
                judge_repeat_passes: 12,
                judge_repeat_trials: 12,
                independent_audit_completed: true,
                adjudicated_critical_errors: 0,
                destructive_scope_families: 60,
                broad_scope_failures: 0,
                zero_event_upper_bound: 0.049,
                p50_latency_ms: 100,
                input_tokens: 1,
                output_tokens: 1,
                report_hash: "report".into(),
                artifact_hash: "artifact".into(),
                reviewer_disposition: "qualified".into(),
            },
            evaluated_at_unix: now,
            reviewed: true,
            qualified: true,
        };
        let mut manifest = QualificationManifest {
            version: 1,
            policy_version: 1,
            policy_hash: policy_hash(),
            entries: vec![entry],
        };
        assert!(
            compatible_entry_for_corpus(&manifest, &candidate, &class, now, &corpus_hash())
                .is_some()
        );
        manifest.entries[0].evaluated_at_unix = now - (181 * 86_400);
        assert!(
            compatible_entry_for_corpus(&manifest, &candidate, &class, now, &corpus_hash())
                .is_none()
        );
        manifest.entries[0].evaluated_at_unix = now;
        manifest.entries[0].adapter_contract_version += 1;
        assert!(
            compatible_entry_for_corpus(&manifest, &candidate, &class, now, &corpus_hash())
                .is_none()
        );
    }

    #[test]
    fn unavailable_holdout_prevents_runtime_evidence_selection() {
        assert_eq!(qualification_corpus_hash(), None);
    }
}
