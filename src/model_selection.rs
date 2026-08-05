//! Conservative fixed/evidence selection and sequential pre-proposal fallback.

use crate::capabilities::{self, QualificationEntry, RequestClass};
use crate::config::{Config, ModelCandidate, SelectionMode};
use crate::provider::{ProviderErrorKind, ProviderId};

pub const SELECTION_POLICY_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct ResolvedSelection {
    pub initial: ModelCandidate,
    pub alternate: Option<ModelCandidate>,
    pub mode: SelectionMode,
    pub fallback_on: Vec<ProviderErrorKind>,
    pub permitted_action_types: Option<Vec<String>>,
    pub resolved_fingerprint: Option<String>,
    pub resolved_model: Option<String>,
    pub alternate_fingerprint: Option<String>,
    pub alternate_resolved_model: Option<String>,
}

pub fn resolve(config: &Config, class: &RequestClass) -> Result<ResolvedSelection, String> {
    let primary = ModelCandidate {
        provider: config.provider,
        model: config.model.clone(),
    };
    if config.selection.mode == SelectionMode::Fixed {
        return Ok(ResolvedSelection {
            initial: primary,
            alternate: config.selection.alternate.clone(),
            mode: SelectionMode::Fixed,
            fallback_on: config.selection.fallback_on.clone(),
            permitted_action_types: None,
            resolved_fingerprint: None,
            resolved_model: None,
            alternate_fingerprint: None,
            alternate_resolved_model: None,
        });
    }
    let manifest = capabilities::load_checked_in()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    resolve_evidence(config, class, &manifest, now)
}

fn resolve_evidence(
    config: &Config,
    class: &RequestClass,
    manifest: &capabilities::QualificationManifest,
    now: u64,
) -> Result<ResolvedSelection, String> {
    let expected_corpus = capabilities::qualification_corpus_hash().ok_or(
        "evidence selection is unavailable: no sealed qualification holdout is checked in",
    )?;
    resolve_evidence_for_corpus(config, class, manifest, now, &expected_corpus)
}

fn resolve_evidence_for_corpus(
    config: &Config,
    class: &RequestClass,
    manifest: &capabilities::QualificationManifest,
    now: u64,
    expected_corpus: &str,
) -> Result<ResolvedSelection, String> {
    let primary = ModelCandidate {
        provider: config.provider,
        model: config.model.clone(),
    };
    let mut configured = vec![primary.clone()];
    if let Some(alternate) = &config.selection.alternate {
        configured.push(alternate.clone());
    }
    let matched: Vec<(&ModelCandidate, &QualificationEntry)> = configured
        .iter()
        .filter_map(|candidate| {
            capabilities::compatible_entry_for_corpus(
                manifest,
                candidate,
                class,
                now,
                expected_corpus,
            )
            .map(|entry| (candidate, entry))
        })
        .collect();
    let selected_matches = matched
        .iter()
        .filter(|(_, entry)| entry.selected)
        .collect::<Vec<_>>();
    if selected_matches.len() != 1 {
        return Err("evidence selection is unavailable: no unique current reviewed qualification matches this request class and configured candidates; use selection.mode=fixed for an explicit choice".into());
    }
    let (selected, entry) = *selected_matches[0];
    let alternate = configured
        .iter()
        .find(|candidate| *candidate != selected)
        .cloned();
    let alternate_fingerprint = alternate.as_ref().and_then(|candidate| {
        matched
            .iter()
            .find(|(matched_candidate, _)| *matched_candidate == candidate)
            .map(|(_, entry)| entry.resolved_fingerprint.clone())
    });
    let alternate_resolved_model = alternate.as_ref().and_then(|candidate| {
        matched
            .iter()
            .find(|(matched_candidate, _)| *matched_candidate == candidate)
            .map(|(_, entry)| entry.resolved_model.clone())
    });
    if alternate.is_some()
        && alternate_fingerprint.is_none()
        && !config.selection.fallback_on.is_empty()
    {
        return Err("evidence selection is unavailable: the configured fallback lacks current reviewed qualification for this request class".into());
    }
    Ok(ResolvedSelection {
        initial: selected.clone(),
        alternate,
        mode: SelectionMode::Evidence,
        fallback_on: config.selection.fallback_on.clone(),
        permitted_action_types: Some(entry.permitted_action_types.clone()),
        resolved_fingerprint: Some(entry.resolved_fingerprint.clone()),
        resolved_model: Some(entry.resolved_model.clone()),
        alternate_fingerprint,
        alternate_resolved_model,
    })
}

#[cfg(test)]
pub fn fallback_allowed(
    selection: &ResolvedSelection,
    kind: ProviderErrorKind,
    attempts_consumed: u8,
    accepted_proposal: bool,
    execution_started: bool,
) -> bool {
    attempts_consumed < 2
        && !accepted_proposal
        && !execution_started
        && selection.alternate.is_some()
        && selection.fallback_on.contains(&kind)
}

pub fn action_type(action: &crate::action::ProposedAction) -> &'static str {
    match action {
        crate::action::ProposedAction::Answer { .. } => "answer",
        crate::action::ProposedAction::Clarification { .. } => "clarification",
        crate::action::ProposedAction::Shell { .. } => "shell",
        crate::action::ProposedAction::Program { .. } => "program",
        crate::action::ProposedAction::ParentShell { .. } => "parent_shell",
        // A routing step, not an executable action; never subject to an
        // evidence profile (the command loop bypasses the profile check for it).
        crate::action::ProposedAction::ProbeSubcommand { .. } => "probe_subcommand",
    }
}

pub fn provider_status(provider: ProviderId, mode: SelectionMode) -> &'static str {
    match (provider, mode) {
        (_, SelectionMode::Evidence) => "evidence-qualified",
        (ProviderId::Openai, SelectionMode::Fixed) => "fixed-default-or-explicit",
        (ProviderId::Cerebras, SelectionMode::Fixed) => "experimental-unqualified",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn request_class() -> RequestClass {
        RequestClass {
            route: "ask".into(),
            stdin_present: false,
            local_input: false,
            input_format: None,
            follow_up: "none".into(),
            runtime_available: true,
        }
    }

    fn evidence_entry(
        candidate: &ModelCandidate,
        class: &RequestClass,
        now: u64,
        selected: bool,
    ) -> QualificationEntry {
        let rate = |point, lower_95| capabilities::EvidenceRate { point, lower_95 };
        QualificationEntry {
            selected,
            provider: candidate.provider,
            api_family: candidate.provider.adapter().api_family().into(),
            endpoint: candidate.provider.adapter().endpoint().into(),
            model: candidate.model.clone(),
            resolved_model: candidate.model.clone(),
            resolved_fingerprint: format!("{}-fingerprint", candidate.model),
            prompt_version: crate::prompt::PROMPT_VERSION,
            action_schema_version: crate::prompt::ACTION_SCHEMA_VERSION,
            program_contract: crate::contract::PROGRAM_CONTRACT.into(),
            context_policy_version: crate::contract::CONTEXT_POLICY_VERSION,
            adapter_contract_version: crate::provider::ADAPTER_CONTRACT_VERSION,
            selection_policy_version: SELECTION_POLICY_VERSION,
            corpus_hash: capabilities::corpus_hash(),
            worker_hash: capabilities::worker_hash(),
            runner_hash: capabilities::runner_hash(),
            policy_hash: capabilities::policy_hash(),
            request_class: class.clone(),
            permitted_action_types: vec!["answer".into(), "clarification".into()],
            evidence: capabilities::QualificationEvidence {
                trials: 3,
                semantic_families: 30,
                seed: 7,
                candidate_calls: 300,
                transport_success: rate(1.0, 0.98),
                wire_validity: rate(1.0, 0.96),
                client_validity: rate(1.0, 0.96),
                first_shot_completion: rate(0.95, 0.85),
                executable_strata: BTreeMap::new(),
                non_executable_acceptability: rate(1.0, 0.9),
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
        }
    }

    #[test]
    fn fallback_is_typed_sequential_and_preproposal_only() {
        let selection = ResolvedSelection {
            initial: ModelCandidate {
                provider: ProviderId::Openai,
                model: "a".into(),
            },
            alternate: Some(ModelCandidate {
                provider: ProviderId::Cerebras,
                model: "b".into(),
            }),
            mode: SelectionMode::Fixed,
            fallback_on: vec![ProviderErrorKind::RateLimited],
            permitted_action_types: None,
            resolved_fingerprint: None,
            resolved_model: None,
            alternate_fingerprint: None,
            alternate_resolved_model: None,
        };
        assert!(fallback_allowed(
            &selection,
            ProviderErrorKind::RateLimited,
            1,
            false,
            false
        ));
        assert!(!fallback_allowed(
            &selection,
            ProviderErrorKind::Auth,
            1,
            false,
            false
        ));
        assert!(!fallback_allowed(
            &selection,
            ProviderErrorKind::RateLimited,
            2,
            false,
            false
        ));
        assert!(!fallback_allowed(
            &selection,
            ProviderErrorKind::RateLimited,
            1,
            true,
            false
        ));
        assert!(!fallback_allowed(
            &selection,
            ProviderErrorKind::RateLimited,
            1,
            false,
            true
        ));
    }

    #[test]
    fn evidence_mode_uses_manifest_selected_configured_candidate() {
        let mut config = Config::test(crate::dirs::Paths {
            config_file: "/tmp/c".into(),
            data_dir: "/tmp/d".into(),
            cache_dir: "/tmp/x".into(),
        });
        config.selection.mode = SelectionMode::Evidence;
        config.selection.alternate = Some(ModelCandidate {
            provider: ProviderId::Cerebras,
            model: "alternate".into(),
        });
        config.selection.fallback_on = vec![ProviderErrorKind::Timeout];
        let primary = ModelCandidate {
            provider: config.provider,
            model: config.model.clone(),
        };
        let alternate = config.selection.alternate.clone().unwrap();
        let class = request_class();
        let now = 2_000_000_000;
        let manifest = capabilities::QualificationManifest {
            version: capabilities::EVIDENCE_MANIFEST_VERSION,
            policy_version: capabilities::QUALIFICATION_POLICY_VERSION,
            policy_hash: capabilities::policy_hash(),
            entries: vec![
                evidence_entry(&primary, &class, now, false),
                evidence_entry(&alternate, &class, now, true),
            ],
        };
        let resolved = resolve_evidence_for_corpus(
            &config,
            &class,
            &manifest,
            now,
            &capabilities::corpus_hash(),
        )
        .unwrap();
        assert_eq!(resolved.initial, alternate);
        assert_eq!(resolved.alternate, Some(primary));
        assert!(resolved.resolved_fingerprint.is_some());
        assert!(resolved.alternate_fingerprint.is_some());
    }

    #[test]
    fn checked_in_empty_manifest_makes_evidence_mode_unavailable() {
        let mut config = Config::test(crate::dirs::Paths {
            config_file: "/tmp/c".into(),
            data_dir: "/tmp/d".into(),
            cache_dir: "/tmp/x".into(),
        });
        config.selection.mode = SelectionMode::Evidence;
        assert!(resolve(&config, &request_class()).is_err());
    }
}
