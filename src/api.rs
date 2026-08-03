//! Backward-compatible API facade over the provider-neutral adapter layer.

use crate::action::ProposedAction;
use crate::provider;

pub struct ApiConfig {
    pub provider: provider::ProviderId,
    pub model: String,
    pub key: String,
    pub max_tokens: u32,
    pub reasoning_effort: String,
    pub request_max_bytes: usize,
    pub response_max_bytes: usize,
    pub alternate: Option<ApiCandidate>,
    pub fallback_on: Vec<provider::ProviderErrorKind>,
    pub selection_mode: crate::config::SelectionMode,
    pub permitted_action_types: Option<Vec<String>>,
    pub resolved_fingerprint: Option<String>,
    pub resolved_model: Option<String>,
}

pub struct ApiCandidate {
    pub provider: provider::ProviderId,
    pub model: String,
    pub key: Option<String>,
    pub resolved_fingerprint: Option<String>,
    pub resolved_model: Option<String>,
}

#[derive(Debug)]
pub struct ActionResponse {
    pub action: ProposedAction,
    pub raw: String,
    pub attempts_consumed: u8,
    pub attempts: Vec<SafeAttempt>,
    pub profile_allowed: bool,
}

#[derive(Debug)]
pub struct ApiError {
    pub message: String,
    pub attempts_consumed: u8,
    pub kind: Option<provider::ProviderErrorKind>,
    pub attempts: Vec<SafeAttempt>,
}

#[derive(Debug, Clone)]
pub struct SafeAttempt {
    pub index: u8,
    pub provider: provider::ProviderId,
    pub api_family: &'static str,
    pub requested_model: String,
    pub resolved_model: Option<String>,
    pub resolved_fingerprint: Option<String>,
    pub adapter_contract_version: u32,
    pub outcome: &'static str,
    pub error_kind: Option<provider::ProviderErrorKind>,
    pub fallback_reason: Option<provider::ProviderErrorKind>,
    pub accepted: bool,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

fn invocation<'a>(config: &'a ApiConfig, input: &'a str, stream: bool) -> provider::Invocation<'a> {
    provider::Invocation {
        model: &config.model,
        authorization: provider::Authorization::bearer(&config.key),
        input,
        stream,
        max_tokens: config.max_tokens,
        reasoning_effort: &config.reasoning_effort,
        request_max_bytes: config.request_max_bytes,
        response_max_bytes: config.response_max_bytes,
    }
}

pub fn request_action(
    config: &ApiConfig,
    input: &str,
    stream: bool,
    allow_fallback: bool,
) -> Result<ActionResponse, ApiError> {
    request_action_with(
        config,
        input,
        stream,
        allow_fallback,
        &provider::NetworkTransport,
    )
}

fn request_action_with(
    config: &ApiConfig,
    input: &str,
    stream: bool,
    allow_fallback: bool,
    transport: &dyn provider::Transport,
) -> Result<ActionResponse, ApiError> {
    let first = provider::invoke_with(
        config.provider.adapter(),
        transport,
        &invocation(config, input, stream),
    )
    .and_then(|response| {
        check_identity(
            response,
            config.resolved_model.as_deref(),
            config.resolved_fingerprint.as_deref(),
        )
    });
    let (response, attempts, fallback_reason, mut safe_attempts) = match first {
        Ok(response) => (response, 1, None, Vec::new()),
        Err(error)
            if error.attempts_consumed == 1
                && allow_fallback
                && config.fallback_on.contains(&error.kind)
                && config.alternate.is_some() =>
        {
            let alternate = config.alternate.as_ref().expect("checked above");
            let first_attempt = failed_attempt(1, config.provider, &config.model, error.kind, None);
            let key = alternate.key.as_deref().ok_or_else(|| ApiError {
                message: format!(
                    "fallback to {} is authorized but its credential is unavailable",
                    alternate.provider
                ),
                attempts_consumed: 1,
                kind: Some(provider::ProviderErrorKind::Credential),
                attempts: vec![first_attempt.clone()],
            })?;
            let alternate_config = ApiConfig {
                provider: alternate.provider,
                model: alternate.model.clone(),
                key: key.into(),
                max_tokens: config.max_tokens,
                reasoning_effort: config.reasoning_effort.clone(),
                request_max_bytes: config.request_max_bytes,
                response_max_bytes: config.response_max_bytes,
                alternate: None,
                fallback_on: Vec::new(),
                selection_mode: config.selection_mode,
                permitted_action_types: config.permitted_action_types.clone(),
                resolved_fingerprint: alternate.resolved_fingerprint.clone(),
                resolved_model: alternate.resolved_model.clone(),
            };
            match provider::invoke_with(
                alternate.provider.adapter(),
                transport,
                &invocation(&alternate_config, input, false),
            )
            .and_then(|response| {
                check_identity(
                    response,
                    alternate_config.resolved_model.as_deref(),
                    alternate_config.resolved_fingerprint.as_deref(),
                )
            }) {
                Ok(response) => (response, 2, Some(error.kind), vec![first_attempt]),
                Err(second) => {
                    return Err(ApiError {
                        message: second.message,
                        attempts_consumed: 1 + second.attempts_consumed,
                        kind: Some(second.kind),
                        attempts: vec![
                            first_attempt,
                            failed_attempt(
                                2,
                                alternate.provider,
                                &alternate.model,
                                second.kind,
                                Some(error.kind),
                            ),
                        ],
                    })
                }
            }
        }
        Err(error) => {
            return Err(ApiError {
                message: error.message,
                attempts_consumed: error.attempts_consumed,
                kind: Some(error.kind),
                attempts: (error.attempts_consumed > 0)
                    .then(|| failed_attempt(1, config.provider, &config.model, error.kind, None))
                    .into_iter()
                    .collect(),
            })
        }
    };
    let action = provider::validate(&response).map_err(|message| ApiError {
        message,
        attempts_consumed: attempts,
        kind: None,
        attempts: {
            safe_attempts.push(response_attempt(
                attempts,
                &response,
                fallback_reason,
                "client_rejected",
                false,
            ));
            safe_attempts.clone()
        },
    })?;
    let profile_allowed = !config
        .permitted_action_types
        .as_ref()
        .is_some_and(|allowed| {
            !allowed
                .iter()
                .any(|value| value == crate::model_selection::action_type(&action))
        });
    safe_attempts.push(response_attempt(
        attempts,
        &response,
        fallback_reason,
        if profile_allowed {
            "accepted"
        } else {
            "out_of_profile"
        },
        profile_allowed,
    ));
    Ok(ActionResponse {
        action,
        raw: response.raw,
        attempts_consumed: attempts,
        attempts: safe_attempts,
        profile_allowed,
    })
}

fn check_identity(
    response: provider::ProviderResponse,
    expected_model: Option<&str>,
    expected: Option<&str>,
) -> Result<provider::ProviderResponse, provider::ProviderError> {
    if expected_model.is_some_and(|value| response.resolved_model.as_deref() != Some(value))
        || expected.is_some_and(|value| response.resolved_fingerprint.as_deref() != Some(value))
    {
        Err(provider::ProviderError::after_request(
            provider::ProviderErrorKind::Incomplete,
            "provider returned a model identity that does not match qualification evidence",
        ))
    } else {
        Ok(response)
    }
}

fn failed_attempt(
    index: u8,
    provider: provider::ProviderId,
    model: &str,
    kind: provider::ProviderErrorKind,
    fallback_reason: Option<provider::ProviderErrorKind>,
) -> SafeAttempt {
    SafeAttempt {
        index,
        provider,
        api_family: provider.adapter().api_family(),
        requested_model: model.into(),
        resolved_model: None,
        resolved_fingerprint: None,
        adapter_contract_version: provider::ADAPTER_CONTRACT_VERSION,
        outcome: "provider_error",
        error_kind: Some(kind),
        fallback_reason,
        accepted: false,
    }
}

fn response_attempt(
    index: u8,
    response: &provider::ProviderResponse,
    fallback_reason: Option<provider::ProviderErrorKind>,
    outcome: &'static str,
    accepted: bool,
) -> SafeAttempt {
    SafeAttempt {
        index,
        provider: response.provider,
        api_family: response.api_family,
        requested_model: response.requested_model.clone(),
        resolved_model: response.resolved_model.clone(),
        resolved_fingerprint: response.resolved_fingerprint.clone(),
        adapter_contract_version: response.adapter_contract_version,
        outcome,
        error_kind: None,
        fallback_reason,
        accepted,
    }
}

pub fn parse_response(config: &ApiConfig, raw: &str) -> Result<ProposedAction, String> {
    let response = config
        .provider
        .adapter()
        .parse_cached(&config.model, raw)
        .map_err(|error| error.to_string())?;
    provider::validate(&response)
}

/// Retained for golden compatibility tests and external prompt snapshots.
#[cfg(test)]
pub fn request_body(config: &ApiConfig, input: &str, stream: bool) -> String {
    config
        .provider
        .adapter()
        .build_request(&invocation(config, input, stream))
        .expect("bounded API request is serializable")
        .body
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    fn config() -> ApiConfig {
        ApiConfig {
            provider: provider::ProviderId::Openai,
            model: "gpt-5.6-luna".into(),
            key: "secret".into(),
            max_tokens: 1024,
            reasoning_effort: "low".into(),
            request_max_bytes: 256 * 1024,
            response_max_bytes: 2 * 1024 * 1024,
            alternate: None,
            fallback_on: Vec::new(),
            selection_mode: crate::config::SelectionMode::Fixed,
            permitted_action_types: None,
            resolved_fingerprint: None,
            resolved_model: None,
        }
    }

    fn tools() -> Value {
        crate::prompt::tools()
    }

    fn response(name: &str, arguments: Value) -> String {
        json!({
            "status":"completed",
            "tools": tools(),
            "output":[{"type":"reasoning"},{
                "type":"function_call","status":"completed","name":name,
                "arguments":serde_json::to_string(&arguments).unwrap()
            }]
        })
        .to_string()
    }

    #[test]
    fn request_contract_is_responses_only_and_private() {
        let value: Value = serde_json::from_str(&request_body(&config(), "input", true)).unwrap();
        assert_eq!(
            provider::openai::ENDPOINT,
            "https://api.openai.com/v1/responses"
        );
        assert_eq!(value["store"], false);
        assert_eq!(value["parallel_tool_calls"], false);
        assert_eq!(value["tool_choice"], "required");
        assert_eq!(value["stream"], true);
        assert!(value.get("previous_response_id").is_none());
        assert_eq!(value["tools"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn accepts_each_strict_tool() {
        let c = config();
        assert!(matches!(
            parse_response(&c, &response("return_answer", json!({"text":"hello"}))).unwrap(),
            ProposedAction::Answer { .. }
        ));
        assert!(matches!(
            parse_response(
                &c,
                &response("request_clarification", json!({"question":"which file?"}))
            )
            .unwrap(),
            ProposedAction::Clarification { .. }
        ));
        assert!(matches!(
            parse_response(&c, &response("run_shell", json!({"command":"ls","summary":"list","assumptions":[],"effects":["read_local"],"requirements":["ls"],"stdin_mode":"none"}))).unwrap(),
            ProposedAction::Shell { .. }
        ));
        assert!(matches!(
            parse_response(&c, &response("run_program", json!({"runtime":"python3","contract":"uhm_helper_v1","source":"print('ok')","summary":"Print a result.","assumptions":[],"stdin_mode":"none","files":[],"effects":[]}))).unwrap(),
            ProposedAction::Program { .. }
        ));
        assert!(matches!(
            parse_response(&c, &response("require_parent_shell", json!({"kind":"set_environment","path":null,"name":"EDITOR","value":"nvim","summary":"Set the editor.","assumptions":[],"effects":["shell_state"]}))).unwrap(),
            ProposedAction::ParentShell { .. }
        ));
    }

    #[test]
    fn rejects_zero_multiple_plain_refusal_unknown_incomplete_and_nonstrict() {
        let c = config();
        let base = json!({"status":"completed","tools":tools(),"output":[]});
        assert!(parse_response(&c, &base.to_string()).is_err());
        let mut multiple = base.clone();
        multiple["output"] = json!([
            {"type":"function_call","status":"completed","name":"return_answer","arguments":"{\"text\":\"a\"}"},
            {"type":"function_call","status":"completed","name":"return_answer","arguments":"{\"text\":\"b\"}"}
        ]);
        assert!(parse_response(&c, &multiple.to_string()).is_err());
        let mut plain = base.clone();
        plain["output"] = json!([{"type":"message","status":"completed","content":[{"type":"refusal","refusal":"no"}]}]);
        assert!(parse_response(&c, &plain.to_string()).is_err());
        let mut nonstrict = base.clone();
        nonstrict["tools"][0]["strict"] = json!(false);
        assert!(parse_response(&c, &nonstrict.to_string()).is_err());
        let incomplete = json!({"status":"incomplete","tools":tools(),"output":[]});
        assert!(parse_response(&c, &incomplete.to_string()).is_err());
    }

    #[test]
    fn production_parser_matches_canonical_conformance_vectors() {
        let c = config();
        let fixture: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/action-validation-cases-v2.json"
        ))
        .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let envelope = &case["envelope"];
            let direct = crate::contract::decode_and_validate(
                envelope["tool"].as_str().unwrap(),
                envelope["arguments"].clone(),
            );
            let through_api = parse_response(
                &c,
                &response(
                    envelope["tool"].as_str().unwrap(),
                    envelope["arguments"].clone(),
                ),
            );
            assert_eq!(
                direct.is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}",
                case["id"]
            );
            assert_eq!(through_api.is_ok(), direct.is_ok(), "{}", case["id"]);
            if let (Ok(left), Ok(right)) = (direct, through_api) {
                assert_eq!(left, right, "{}", case["id"]);
            }
        }
    }

    struct FakeTransport {
        replies: Mutex<VecDeque<Result<String, provider::ProviderError>>>,
        requests: Mutex<Vec<(String, String)>>,
    }

    impl provider::Transport for FakeTransport {
        fn post(
            &self,
            request: provider::HttpRequest,
        ) -> Result<provider::HttpResponse, provider::ProviderError> {
            self.requests
                .lock()
                .unwrap()
                .push((request.url.into(), request.authorization.expose().into()));
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("one fake response per request")
                .map(|body| provider::HttpResponse {
                    status: 200,
                    reader: Box::new(std::io::Cursor::new(body.into_bytes())),
                })
        }
    }

    #[test]
    fn typed_fallback_is_sequential_and_consumes_two_attempts() {
        let mut c = config();
        c.alternate = Some(ApiCandidate {
            provider: provider::ProviderId::Cerebras,
            model: "gpt-oss-120b".into(),
            key: Some("cerebras-secret".into()),
            resolved_fingerprint: None,
            resolved_model: None,
        });
        c.fallback_on = vec![provider::ProviderErrorKind::RateLimited];
        let chat = json!({
            "model":"gpt-oss-120b","choices":[{"finish_reason":"tool_calls","message":{
                "tool_calls":[{"function":{"name":"return_answer","arguments":"{\"text\":\"ok\"}"}}]
            }}]
        })
        .to_string();
        let transport = FakeTransport {
            replies: Mutex::new(VecDeque::from([
                Err(provider::ProviderError::after_request(
                    provider::ProviderErrorKind::RateLimited,
                    "rate limited",
                )),
                Ok(chat),
            ])),
            requests: Mutex::new(Vec::new()),
        };
        let result = request_action_with(&c, "input", false, true, &transport).unwrap();
        assert_eq!(result.attempts_consumed, 2);
        assert_eq!(
            result.attempts.last().unwrap().provider,
            provider::ProviderId::Cerebras
        );
        assert_eq!(
            result.attempts.last().unwrap().fallback_reason,
            Some(provider::ProviderErrorKind::RateLimited)
        );
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests[0].0, provider::openai::ENDPOINT);
        assert_eq!(requests[1].0, provider::cerebras::ENDPOINT);
        assert_eq!(requests[0].1, "Bearer secret");
        assert_eq!(requests[1].1, "Bearer cerebras-secret");
        assert!(!format!("{result:?}").contains("secret"));
    }

    #[test]
    fn auth_is_not_a_fallback_trigger_and_missing_alternate_key_fails_closed() {
        let mut c = config();
        c.alternate = Some(ApiCandidate {
            provider: provider::ProviderId::Cerebras,
            model: "gpt-oss-120b".into(),
            key: None,
            resolved_fingerprint: None,
            resolved_model: None,
        });
        c.fallback_on = vec![provider::ProviderErrorKind::RateLimited];
        let transport = FakeTransport {
            replies: Mutex::new(VecDeque::from([Err(
                provider::ProviderError::after_request(
                    provider::ProviderErrorKind::RateLimited,
                    "provider-controlled\u{1b}[31m failure",
                ),
            )])),
            requests: Mutex::new(Vec::new()),
        };
        let error = request_action_with(&c, "input", false, true, &transport).unwrap_err();
        assert_eq!(error.attempts_consumed, 1);
        assert_eq!(error.kind, Some(provider::ProviderErrorKind::Credential));
        assert!(!error.message.contains('\u{1b}'));
    }

    #[test]
    fn evidence_fingerprint_mismatch_can_use_one_qualified_fallback() {
        let mut c = config();
        c.resolved_fingerprint = Some("expected-primary".into());
        c.resolved_model = Some("gpt-5.6-luna".into());
        c.alternate = Some(ApiCandidate {
            provider: provider::ProviderId::Cerebras,
            model: "gpt-oss-120b".into(),
            key: Some("alternate-key".into()),
            resolved_fingerprint: Some("expected-alternate".into()),
            resolved_model: Some("gpt-oss-120b".into()),
        });
        c.fallback_on = vec![provider::ProviderErrorKind::Incomplete];
        let primary = json!({
            "status":"completed", "system_fingerprint":"changed", "tools":tools(),
            "output":[{"type":"function_call","status":"completed","name":"return_answer","arguments":"{\"text\":\"wrong revision\"}"}]
        }).to_string();
        let alternate = json!({
            "model":"gpt-oss-120b", "system_fingerprint":"expected-alternate",
            "choices":[{"finish_reason":"tool_calls","message":{"tool_calls":[{"function":{"name":"return_answer","arguments":"{\"text\":\"ok\"}"}}]}}]
        }).to_string();
        let transport = FakeTransport {
            replies: Mutex::new(VecDeque::from([Ok(primary), Ok(alternate)])),
            requests: Mutex::new(Vec::new()),
        };
        let result = request_action_with(&c, "input", false, true, &transport).unwrap();
        assert_eq!(result.attempts_consumed, 2);
        assert_eq!(transport.requests.lock().unwrap().len(), 2);
        assert_eq!(
            result.attempts[0].error_kind,
            Some(provider::ProviderErrorKind::Incomplete)
        );
        assert!(result.attempts[1].accepted);
    }
}
