use super::*;
use serde_json::{json, Value};

pub const API_FAMILY: &str = "cerebras_chat_completions_v1";
pub const ENDPOINT: &str = "https://api.cerebras.ai/v1/chat/completions";
pub struct CerebrasAdapter;

impl ProviderAdapter for CerebrasAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Cerebras
    }

    fn api_family(&self) -> &'static str {
        API_FAMILY
    }

    fn endpoint(&self) -> &'static str {
        ENDPOINT
    }

    fn credential_env(&self) -> &'static str {
        "CEREBRAS_API_KEY"
    }

    fn capabilities(&self) -> WireCapabilities {
        WireCapabilities {
            streaming: false,
            reasoning_effort: false,
            strict_schema_bounds: false,
        }
    }

    fn build_request(&self, invocation: &Invocation<'_>) -> Result<HttpRequest, ProviderError> {
        let tools = crate::prompt::tools()
            .as_array()
            .expect("canonical tools are an array")
            .iter()
            .map(|tool| {
                let mut parameters = tool["parameters"].clone();
                strip_unsupported_bounds(&mut parameters);
                json!({
                    "type":"function",
                    "function":{
                        "name":tool["name"],
                        "description":tool["description"],
                        "parameters":parameters,
                        "strict":tool["strict"]
                    }
                })
            })
            .collect::<Vec<_>>();
        let body = serde_json::to_string(&json!({
            "model":invocation.model,
            "messages":[
                {"role":"developer","content":crate::prompt::DEVELOPER_INSTRUCTIONS},
                {"role":"user","content":invocation.input}
            ],
            "tools":tools,
            "tool_choice":"required",
            "parallel_tool_calls":false,
            "max_completion_tokens":invocation.max_tokens,
            "stream":false
        }))
        .map_err(|error| {
            ProviderError::before_request(
                ProviderErrorKind::Malformed,
                format!("serialize Cerebras request: {error}"),
            )
        })?;
        Ok(HttpRequest {
            url: ENDPOINT,
            authorization: invocation.authorization.clone(),
            accept: "application/json",
            body,
        })
    }

    fn parse_response(
        &self,
        invocation: &Invocation<'_>,
        response: HttpResponse,
    ) -> Result<ProviderResponse, ProviderError> {
        let raw = read_bounded(
            response.reader,
            invocation.response_max_bytes,
            ProviderId::Cerebras,
        )?;
        parse(invocation.model, &raw, 1)
    }

    fn parse_cached(&self, model: &str, raw: &str) -> Result<ProviderResponse, ProviderError> {
        parse(model, raw, 0)
    }
}

fn strip_unsupported_bounds(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("maxLength");
            map.remove("maxItems");
            // Confirmed by the fixed endpoint's live schema validator; the
            // canonical Rust validator continues enforcing these patterns.
            map.remove("pattern");
            for child in map.values_mut() {
                strip_unsupported_bounds(child);
            }
        }
        Value::Array(values) => {
            for child in values {
                strip_unsupported_bounds(child);
            }
        }
        _ => {}
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
            format!("invalid Chat Completions JSON: {error}"),
        )
    })?;
    if let Some(message) = response["error"]["message"].as_str() {
        return Err(fail(
            ProviderErrorKind::RequestRejected,
            format!("Cerebras response error: {message}"),
        ));
    }
    let choices = response["choices"].as_array().ok_or_else(|| {
        fail(
            ProviderErrorKind::Malformed,
            "Chat Completions response omitted choices".into(),
        )
    })?;
    if choices.len() != 1 {
        return Err(fail(
            ProviderErrorKind::Malformed,
            format!("expected exactly one choice, received {}", choices.len()),
        ));
    }
    let choice = &choices[0];
    let finish = choice["finish_reason"].as_str().unwrap_or("");
    if finish != "tool_calls" && finish != "stop" {
        return Err(fail(
            ProviderErrorKind::Incomplete,
            format!("Cerebras completion did not finish with a tool call ({finish})"),
        ));
    }
    if choice["message"]
        .get("refusal")
        .is_some_and(|value| !value.is_null())
    {
        return Err(fail(
            ProviderErrorKind::Refused,
            "Cerebras refused the request".into(),
        ));
    }
    let calls = choice["message"]["tool_calls"].as_array().ok_or_else(|| {
        let kind = if choice["message"]["content"].as_str().is_some() {
            ProviderErrorKind::Malformed
        } else {
            ProviderErrorKind::Incomplete
        };
        fail(kind, "Cerebras response did not contain a tool call".into())
    })?;
    if calls.len() != 1 {
        return Err(fail(
            ProviderErrorKind::Malformed,
            format!("expected exactly one tool call, received {}", calls.len()),
        ));
    }
    let function = &calls[0]["function"];
    let name = function["name"].as_str().ok_or_else(|| {
        fail(
            ProviderErrorKind::Malformed,
            "Cerebras tool call omitted function name".into(),
        )
    })?;
    let parsed = arguments(name, &function["arguments"])?;
    Ok(ProviderResponse {
        call: DecodedToolCall {
            name: name.into(),
            arguments: parsed,
        },
        provider: ProviderId::Cerebras,
        api_family: API_FAMILY,
        requested_model: model.into(),
        resolved_model: response["model"].as_str().map(bounded),
        resolved_fingerprint: response["system_fingerprint"].as_str().map(bounded),
        request_id: response["id"].as_str().map(bounded),
        finish_reason: Some(finish.into()),
        usage: Usage {
            input_tokens: response["usage"]["prompt_tokens"].as_u64(),
            output_tokens: response["usage"]["completion_tokens"].as_u64(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn invocation<'a>() -> Invocation<'a> {
        Invocation {
            model: "gpt-oss-120b",
            authorization: Authorization::bearer("cerebras-secret"),
            input: "input",
            stream: true,
            max_tokens: 1024,
            reasoning_effort: "high",
            request_max_bytes: 256 * 1024,
            response_max_bytes: 2 * 1024 * 1024,
        }
    }

    #[test]
    fn chat_request_uses_fixed_endpoint_and_omits_unsupported_fields() {
        let request = CerebrasAdapter.build_request(&invocation()).unwrap();
        let value: Value = serde_json::from_str(&request.body).unwrap();
        assert_eq!(request.url, ENDPOINT);
        assert_eq!(value["stream"], false);
        assert!(value.get("reasoning").is_none());
        assert!(request.body.contains("\"role\":\"developer\""));
        assert!(!request.body.contains("maxLength"));
        assert!(!request.body.contains("maxItems"));
        assert!(!request.body.contains("\"pattern\""));
        assert_eq!(value["tools"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn parses_one_buffered_tool_call() {
        let raw = json!({
            "id":"req-1","model":"gpt-oss-120b","choices":[{
                "finish_reason":"tool_calls","message":{"role":"assistant","content":null,
                "tool_calls":[{"type":"function","function":{"name":"return_answer","arguments":"{\"text\":\"ok\"}"}}]}
            }],"usage":{"prompt_tokens":2,"completion_tokens":3}
        }).to_string();
        let response = parse("gpt-oss-120b", &raw, 1).unwrap();
        assert_eq!(response.call.name, "return_answer");
        assert!(matches!(
            validate(&response).unwrap(),
            crate::action::ProposedAction::Answer { .. }
        ));
    }

    #[test]
    fn rejects_zero_or_multiple_choices_and_calls_plain_refusal_and_incomplete() {
        for value in [
            json!({"choices":[]}),
            json!({"choices":[{},{}]}),
            json!({"choices":[{"finish_reason":"tool_calls","message":{"tool_calls":[]}}]}),
            json!({"choices":[{"finish_reason":"tool_calls","message":{"content":"plain"}}]}),
            json!({"choices":[{"finish_reason":"tool_calls","message":{"refusal":"no"}}]}),
            json!({"choices":[{"finish_reason":"length","message":{}}]}),
        ] {
            assert!(parse("model", &value.to_string(), 1).is_err());
        }
        assert_eq!(
            parse("model", r#"{"choices":[{"finish_reason":"tool_calls","message":{"tool_calls":[{"function":{"name":"return_answer","arguments":"not-json"}}]}}]}"#, 1).unwrap_err().kind,
            ProviderErrorKind::Malformed
        );
        let error = parse("model", "{\u{1b}[31m", 1).unwrap_err();
        assert!(!error.message.contains('\u{1b}'));
    }
}
