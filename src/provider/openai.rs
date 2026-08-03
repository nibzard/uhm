use super::*;
use serde_json::{json, Value};

pub const API_FAMILY: &str = "openai_responses_v1";
pub const ENDPOINT: &str = "https://api.openai.com/v1/responses";
pub struct OpenAiAdapter;

impl ProviderAdapter for OpenAiAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Openai
    }

    fn api_family(&self) -> &'static str {
        API_FAMILY
    }

    fn endpoint(&self) -> &'static str {
        ENDPOINT
    }

    fn credential_env(&self) -> &'static str {
        "OPENAI_API_KEY"
    }

    fn capabilities(&self) -> WireCapabilities {
        WireCapabilities {
            streaming: true,
            reasoning_effort: true,
            strict_schema_bounds: true,
        }
    }

    fn build_request(&self, invocation: &Invocation<'_>) -> Result<HttpRequest, ProviderError> {
        let body = serde_json::to_string(&json!({
            "model": invocation.model,
            "instructions": crate::prompt::DEVELOPER_INSTRUCTIONS,
            "input": invocation.input,
            "tools": crate::prompt::tools(),
            "tool_choice": "required",
            "parallel_tool_calls": false,
            "store": false,
            "max_output_tokens": invocation.max_tokens,
            "reasoning": {"effort": invocation.reasoning_effort},
            "stream": invocation.stream
        }))
        .map_err(|error| {
            ProviderError::before_request(
                ProviderErrorKind::Malformed,
                format!("serialize OpenAI request: {error}"),
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
                ProviderId::Openai,
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
            format!("OpenAI response error: {message}"),
        ));
    }
    if response["status"] != "completed" {
        return Err(fail(
            ProviderErrorKind::Incomplete,
            format!(
                "OpenAI response status was {}, not completed",
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
        provider: ProviderId::Openai,
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
    if tools.len() != 5 {
        return Err("response did not resolve exactly five proposal tools".into());
    }
    for tool in tools {
        if tool["type"] != "function" || tool["strict"] != true {
            return Err("response resolved a proposal tool without strict mode".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invocation<'a>(input: &'a str) -> Invocation<'a> {
        Invocation {
            model: "gpt-5.6-terra",
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
    fn golden_request_preserves_responses_contract() {
        let request = OpenAiAdapter.build_request(&invocation("input")).unwrap();
        let value: Value = serde_json::from_str(&request.body).unwrap();
        assert_eq!(request.url, ENDPOINT);
        assert_eq!(value["store"], false);
        assert_eq!(value["parallel_tool_calls"], false);
        assert_eq!(value["tool_choice"], "required");
        assert_eq!(value["stream"], false);
        assert!(value.get("previous_response_id").is_none());
        assert_eq!(value["tools"].as_array().unwrap().len(), 5);
    }
}
