use super::*;
use serde_json::{json, Value};

pub const API_FAMILY: &str = "deepseek_responses_v1";
pub const ENDPOINT: &str = "https://api.deepseek.com/v1/responses";
pub struct DeepSeekAdapter;

impl ProviderAdapter for DeepSeekAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Deepseek
    }

    fn api_family(&self) -> &'static str {
        API_FAMILY
    }

    fn endpoint(&self) -> &'static str {
        ENDPOINT
    }

    fn credential_env(&self) -> &'static str {
        "DEEPSEEK_API_KEY"
    }

    fn capabilities(&self) -> WireCapabilities {
        WireCapabilities {
            streaming: true,
            reasoning_effort: true,
            strict_schema_bounds: true,
        }
    }

    fn build_request(&self, invocation: &Invocation<'_>) -> Result<HttpRequest, ProviderError> {
        // DeepSeek speaks the OpenAI Responses API format and echoes the resolved
        // strict tool set, so the wire contract matches OpenAI. One divergence:
        // deepseek-v4-flash runs in thinking mode by default, and thinking mode
        // rejects `tool_choice: "required"` (HTTP 400). Use "auto" instead; the
        // parser still requires exactly one completed function call, so a plain
        // prose response is rejected downstream exactly as for OpenAI.
        let body = serde_json::to_string(&json!({
            "model": invocation.model,
            "instructions": crate::prompt::DEVELOPER_INSTRUCTIONS,
            "input": invocation.input,
            "tools": crate::prompt::tools_for_input(invocation.input),
            "tool_choice": "auto",
            "parallel_tool_calls": false,
            "store": false,
            "max_output_tokens": invocation.max_tokens,
            "reasoning": {"effort": invocation.reasoning_effort},
            "stream": invocation.stream
        }))
        .map_err(|error| {
            ProviderError::before_request(
                ProviderErrorKind::Malformed,
                format!("serialize DeepSeek request: {error}"),
            )
        })?;
        Ok(HttpRequest {
            url: ENDPOINT,
            authorization: invocation.authorization.clone(),
            accept: if invocation.stream {
                "text/event-stream"
            } else {
                "application/json"
            },
            body,
        })
    }

    fn parse_response(
        &self,
        invocation: &Invocation<'_>,
        response: HttpResponse,
    ) -> Result<ProviderResponse, ProviderError> {
        let raw = if invocation.stream {
            crate::sse::read_responses_stream(response.reader, invocation.response_max_bytes)
                .map_err(|error| {
                    ProviderError::after_request(ProviderErrorKind::Malformed, error)
                })?
        } else {
            read_bounded(
                response.reader,
                invocation.response_max_bytes,
                ProviderId::Deepseek,
            )?
        };
        parse(invocation.model, &raw, 1)
    }

    fn parse_cached(&self, model: &str, raw: &str) -> Result<ProviderResponse, ProviderError> {
        parse(model, raw, 0)
    }
}

fn parse(model: &str, raw: &str, attempts: u8) -> Result<ProviderResponse, ProviderError> {
    let fail = |kind, message: String| ProviderError {
        kind,
        message: crate::render::ansi::sanitize_untrusted(&message),
        attempts_consumed: attempts,
    };
    let response: Value = serde_json::from_str(raw).map_err(|error| {
        fail(
            ProviderErrorKind::Malformed,
            format!("invalid Responses API JSON: {error}"),
        )
    })?;
    if let Some(message) = response["error"]["message"].as_str() {
        return Err(fail(
            ProviderErrorKind::RequestRejected,
            format!("DeepSeek response error: {message}"),
        ));
    }
    if response["status"] != "completed" {
        return Err(fail(
            ProviderErrorKind::Incomplete,
            format!(
                "DeepSeek response status was {}, not completed",
                response["status"]
            ),
        ));
    }
    validate_returned_tools(&response)
        .map_err(|message| fail(ProviderErrorKind::Malformed, message))?;
    let output = response["output"].as_array().ok_or_else(|| {
        fail(
            ProviderErrorKind::Malformed,
            "completed response did not contain an output array".into(),
        )
    })?;
    let mut calls = Vec::new();
    for item in output {
        match item["type"].as_str().unwrap_or("") {
            "reasoning" => {}
            "function_call" => {
                if item["status"] != "completed" {
                    return Err(fail(
                        ProviderErrorKind::Incomplete,
                        "function call was not completed".into(),
                    ));
                }
                calls.push(item);
            }
            "message" => {
                let kind = if item.to_string().contains("refusal") {
                    ProviderErrorKind::Refused
                } else {
                    ProviderErrorKind::Malformed
                };
                return Err(fail(
                    kind,
                    "plain-text or refusal output is not a valid uhm action".into(),
                ));
            }
            other => {
                return Err(fail(
                    ProviderErrorKind::Malformed,
                    format!("unsupported Responses output item '{other}'"),
                ))
            }
        }
    }
    if calls.len() != 1 {
        return Err(fail(
            ProviderErrorKind::Malformed,
            format!(
                "expected exactly one completed function call, received {}",
                calls.len()
            ),
        ));
    }
    let call = calls[0];
    let name = call["name"].as_str().ok_or_else(|| {
        fail(
            ProviderErrorKind::Malformed,
            "function call omitted name".into(),
        )
    })?;
    let parsed = arguments(name, &call["arguments"])?;
    Ok(ProviderResponse {
        call: DecodedToolCall {
            name: name.into(),
            arguments: parsed,
        },
        provider: ProviderId::Deepseek,
        api_family: API_FAMILY,
        requested_model: model.into(),
        resolved_model: response["model"].as_str().map(bounded),
        resolved_fingerprint: response["system_fingerprint"].as_str().map(bounded),
        request_id: response["id"].as_str().map(bounded),
        finish_reason: Some("completed".into()),
        usage: Usage {
            input_tokens: response["usage"]["input_tokens"].as_u64(),
            output_tokens: response["usage"]["output_tokens"].as_u64(),
        },
        adapter_contract_version: ADAPTER_CONTRACT_VERSION,
        raw: raw.into(),
    })
}

fn bounded(value: &str) -> String {
    crate::render::ansi::sanitize_untrusted(value)
        .chars()
        .take(128)
        .collect()
}

fn validate_returned_tools(response: &Value) -> Result<(), String> {
    let tools = response["tools"]
        .as_array()
        .ok_or("response omitted resolved strict tool metadata")?;
    if !matches!(tools.len(), 2 | 5 | 6) {
        return Err("response did not resolve a canonical proposal tool set".into());
    }
    let mut names = Vec::new();
    for tool in tools {
        if tool["type"] != "function" || tool["strict"] != true {
            return Err("response resolved a proposal tool without strict mode".into());
        }
        names.push(tool["name"].as_str().unwrap_or_default());
    }
    let prose = ["return_answer", "request_clarification"];
    let complete = [
        "return_answer",
        "run_program",
        "run_shell",
        "require_parent_shell",
        "request_clarification",
    ];
    // Plan 18: the first call of an executable job also offers probe_subcommand,
    // so the resolved set has six entries in that case. It is omitted from the
    // follow-up call after a probe, so nesting never reaches this check.
    let complete_with_probe = [
        "return_answer",
        "run_program",
        "run_shell",
        "require_parent_shell",
        "request_clarification",
        "probe_subcommand",
    ];
    if names != prose && names != complete && names != complete_with_probe {
        return Err("response resolved a noncanonical proposal tool set".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invocation<'a>(input: &'a str) -> Invocation<'a> {
        Invocation {
            model: "deepseek-v4-flash",
            authorization: Authorization::bearer("secret"),
            input,
            stream: false,
            max_tokens: 1024,
            reasoning_effort: "low",
            request_max_bytes: 256 * 1024,
            response_max_bytes: 2 * 1024 * 1024,
        }
    }

    #[test]
    fn request_uses_auto_tool_choice_because_thinking_mode_rejects_required() {
        let request = DeepSeekAdapter.build_request(&invocation("input")).unwrap();
        let value: Value = serde_json::from_str(&request.body).unwrap();
        assert_eq!(request.url, ENDPOINT);
        // DeepSeek thinking mode rejects "required"; the parser still enforces one call.
        assert_eq!(value["tool_choice"], "auto");
        assert_eq!(value["store"], false);
        assert_eq!(value["parallel_tool_calls"], false);
        assert_eq!(value["stream"], false);
        assert!(value.get("previous_response_id").is_none());
        assert_eq!(value["tools"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn prose_request_exposes_only_answer_and_clarification() {
        let input = crate::prompt::proposal_input(
            "ask",
            "summarize",
            json!({}),
            json!({"present": true, "text": "hello"}),
            None,
        );
        let request = DeepSeekAdapter.build_request(&invocation(&input)).unwrap();
        let value: Value = serde_json::from_str(&request.body).unwrap();
        let names = value["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["return_answer", "request_clarification"]);
    }

    #[test]
    fn parses_live_responses_shape_with_reasoning_then_function_call() {
        // Fixture built from a captured deepseek-v4-flash Responses payload: the
        // model emits a reasoning output item followed by exactly one completed
        // function_call, echoes the canonical strict tool set, and reports usage
        // with the same input_tokens/output_tokens fields as OpenAI.
        let prose_tools = crate::prompt::tools_for_input(&crate::prompt::proposal_input(
            "ask",
            "summarize",
            json!({}),
            json!({}),
            None,
        ));
        let raw = json!({
            "id": "d633df6f-3529-4c4b-a8d1-e84f266b30b7",
            "object": "response",
            "status": "completed",
            "model": "deepseek-v4-flash",
            "tools": prose_tools,
            "output": [
                {"type": "reasoning", "id": "b2b77327", "status": "completed",
                 "content": [{"type": "reasoning_text", "text": "planning the answer"}]},
                {"type": "function_call", "status": "completed", "name": "return_answer",
                 "arguments": "{\"text\":\"Use: echo Hello world\"}"}
            ],
            "usage": {"input_tokens": 387, "output_tokens": 183}
        });
        let response = parse("deepseek-v4-flash", &raw.to_string(), 1).unwrap();
        assert_eq!(response.provider, ProviderId::Deepseek);
        assert_eq!(response.api_family, API_FAMILY);
        assert_eq!(response.call.name, "return_answer");
        assert_eq!(
            response.usage,
            Usage {
                input_tokens: Some(387),
                output_tokens: Some(183),
            }
        );
        assert!(matches!(
            validate(&response).unwrap(),
            crate::action::ProposedAction::Answer { .. }
        ));
    }

    #[test]
    fn run_route_accepts_the_six_tool_set_that_includes_probe_subcommand() {
        // Plan 18: the first executable call offers probe_subcommand, so the
        // echoed strict tool set has six entries on the run route. DeepSeek
        // echoes the set the way OpenAI does, so its validator must accept it.
        let run_tools = crate::prompt::tools_for_input(&crate::prompt::proposal_input(
            "run",
            "list files",
            json!({}),
            json!({}),
            None,
        ));
        assert_eq!(run_tools.as_array().unwrap().len(), 6);
        let raw = json!({
            "status": "completed",
            "model": "deepseek-v4-flash",
            "tools": run_tools,
            "output": [{"type": "function_call", "status": "completed",
                        "name": "run_shell",
                        "arguments": "{\"command\":\"ls\",\"summary\":\"list\",\"assumptions\":[],\"effects\":[\"read_local\"],\"requirements\":[],\"stdin_mode\":\"none\"}"}]
        });
        let response = parse("deepseek-v4-flash", &raw.to_string(), 1).unwrap();
        assert_eq!(response.call.name, "run_shell");
        assert!(matches!(
            validate(&response).unwrap(),
            crate::action::ProposedAction::Shell { .. }
        ));
    }

    #[test]
    fn rejects_status_not_completed_and_non_string_arguments() {
        let tools = crate::prompt::tools_for_input(&crate::prompt::proposal_input(
            "ask",
            "summarize",
            json!({}),
            json!({}),
            None,
        ));
        let incomplete = json!({
            "status": "incomplete",
            "model": "deepseek-v4-flash",
            "tools": tools,
            "output": [{"type": "function_call", "status": "completed",
                        "name": "return_answer", "arguments": "{}"}]
        });
        assert_eq!(
            parse("deepseek-v4-flash", &incomplete.to_string(), 1)
                .unwrap_err()
                .kind,
            ProviderErrorKind::Incomplete
        );

        // DeepSeek returns function_call.arguments as a JSON string, matching
        // OpenAI; a non-string value must be rejected by the shared decoder.
        let non_string = json!({
            "status": "completed",
            "model": "deepseek-v4-flash",
            "tools": tools,
            "output": [{"type": "function_call", "status": "completed",
                        "name": "return_answer", "arguments": {"text": "oops"}}]
        });
        assert_eq!(
            parse("deepseek-v4-flash", &non_string.to_string(), 1)
                .unwrap_err()
                .kind,
            ProviderErrorKind::Malformed
        );
    }
}
