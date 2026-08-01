//! Official OpenAI Responses API client with five strict local proposal tools.

use crate::action::{
    Effect, ParentAction, ParentActionKind, ProgramInput, ProgramProposal, ProgramResultMode,
    ProgramRuntime, ProposalMetadata, ProposedAction, StdinMode,
};
use crate::{http, prompt, sse};
use serde::Deserialize;
use serde_json::{json, Value};

pub const API_FAMILY: &str = "openai_responses_v1";
pub const ENDPOINT: &str = "https://api.openai.com/v1/responses";
pub struct ApiConfig {
    pub model: String,
    pub key: String,
    pub max_tokens: u32,
    pub reasoning_effort: String,
    pub request_max_bytes: usize,
    pub response_max_bytes: usize,
}

pub trait Transport {
    fn post(&self, url: &str, authorization: &str, body: &str) -> Result<http::Response, String>;
}

struct NetworkTransport;

impl Transport for NetworkTransport {
    fn post(&self, url: &str, authorization: &str, body: &str) -> Result<http::Response, String> {
        http::post_stream(url, authorization, body)
    }
}

pub fn request_action(
    config: &ApiConfig,
    input: &str,
    stream: bool,
) -> Result<(ProposedAction, String), String> {
    request_action_with(&NetworkTransport, config, input, stream)
}

fn request_action_with(
    transport: &dyn Transport,
    config: &ApiConfig,
    input: &str,
    stream: bool,
) -> Result<(ProposedAction, String), String> {
    if config.key.trim().is_empty() {
        return Err("No API key was provided to the OpenAI transport".into());
    }
    if input.len() > config.request_max_bytes {
        return Err(format!(
            "model request exceeds configured {} byte limit",
            config.request_max_bytes
        ));
    }
    let body = request_body(config, input, stream);
    let authorization = format!("Bearer {}", config.key);
    let response = transport.post(ENDPOINT, &authorization, &body)?;
    let raw = if stream {
        sse::read_responses_stream(response.reader, config.response_max_bytes)?
    } else {
        read_buffered(response.reader, config.response_max_bytes)?
    };
    let action = parse_response(&raw)?;
    Ok((action, raw))
}

pub fn request_body(config: &ApiConfig, input: &str, stream: bool) -> String {
    serde_json::to_string(&json!({
        "model": config.model,
        "instructions": prompt::DEVELOPER_INSTRUCTIONS,
        "input": input,
        "tools": prompt::tools(),
        "tool_choice": "required",
        "parallel_tool_calls": false,
        "store": false,
        "max_output_tokens": config.max_tokens,
        "reasoning": {"effort": config.reasoning_effort},
        "stream": stream
    }))
    .expect("Responses request is serializable")
}

fn read_buffered(
    mut reader: Box<dyn std::io::Read + Send>,
    max_response_bytes: usize,
) -> Result<String, String> {
    use std::io::Read as _;

    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take((max_response_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("read OpenAI response: {}", e))?;
    if bytes.len() > max_response_bytes {
        return Err(format!(
            "OpenAI response exceeded configured {} byte limit",
            max_response_bytes
        ));
    }
    String::from_utf8(bytes).map_err(|_| "OpenAI response was not valid UTF-8".into())
}

pub fn parse_response(raw: &str) -> Result<ProposedAction, String> {
    let response: Value =
        serde_json::from_str(raw).map_err(|e| format!("invalid Responses API JSON: {}", e))?;
    if let Some(message) = response["error"]["message"].as_str() {
        return Err(format!(
            "OpenAI response error: {}",
            crate::render::ansi::sanitize_untrusted(message)
        ));
    }
    if response["status"] != "completed" {
        return Err(format!(
            "OpenAI response status was {}, not completed",
            response["status"]
        ));
    }
    validate_returned_tools(&response)?;
    let output = response["output"]
        .as_array()
        .ok_or("completed response did not contain an output array")?;
    let mut calls = Vec::new();
    for item in output {
        match item["type"].as_str().unwrap_or("") {
            "reasoning" => {}
            "function_call" => {
                if item["status"] != "completed" {
                    return Err("function call was not completed".into());
                }
                calls.push(item);
            }
            "message" => {
                return Err("plain-text or refusal output is not a valid uhm action".into())
            }
            other => return Err(format!("unsupported Responses output item '{}'", other)),
        }
    }
    if calls.len() != 1 {
        return Err(format!(
            "expected exactly one completed function call, received {}",
            calls.len()
        ));
    }
    parse_call(calls[0])?.validate()
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerArgs {
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShellArgs {
    command: String,
    summary: String,
    assumptions: Vec<String>,
    effects: Vec<Effect>,
    requirements: Vec<String>,
    stdin_mode: StdinMode,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParentArgs {
    kind: ParentActionKind,
    path: Option<String>,
    name: Option<String>,
    value: Option<String>,
    summary: String,
    assumptions: Vec<String>,
    effects: Vec<Effect>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClarificationArgs {
    question: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProgramArgs {
    runtime: ProgramRuntime,
    source: String,
    summary: String,
    assumptions: Vec<String>,
    inputs: Vec<ProgramInput>,
    outputs: Vec<String>,
    effects: Vec<Effect>,
    result_mode: ProgramResultMode,
}

fn parse_call(item: &Value) -> Result<ProposedAction, String> {
    let name = item["name"].as_str().ok_or("function call omitted name")?;
    let arguments = item["arguments"]
        .as_str()
        .ok_or("function call omitted string arguments")?;
    if arguments.len() > 96 * 1024 {
        return Err("function arguments exceeded 98304 bytes".into());
    }
    match name {
        "return_answer" => {
            let args: AnswerArgs = parse_args(arguments, name)?;
            Ok(ProposedAction::Answer { text: args.text })
        }
        "run_shell" => {
            let args: ShellArgs = parse_args(arguments, name)?;
            Ok(ProposedAction::Shell {
                command: args.command,
                metadata: ProposalMetadata {
                    summary: args.summary,
                    assumptions: args.assumptions,
                    effects: args.effects,
                    requirements: args.requirements,
                },
                stdin_mode: args.stdin_mode,
            })
        }
        "require_parent_shell" => {
            let args: ParentArgs = parse_args(arguments, name)?;
            Ok(ProposedAction::ParentShell {
                action: ParentAction {
                    kind: args.kind,
                    path: args.path,
                    name: args.name,
                    value: args.value,
                },
                metadata: ProposalMetadata {
                    summary: args.summary,
                    assumptions: args.assumptions,
                    effects: args.effects,
                    requirements: Vec::new(),
                },
            })
        }
        "request_clarification" => {
            let args: ClarificationArgs = parse_args(arguments, name)?;
            Ok(ProposedAction::Clarification {
                question: args.question,
            })
        }
        "run_program" => {
            let args: ProgramArgs = parse_args(arguments, name)?;
            Ok(ProposedAction::Program {
                program: ProgramProposal {
                    runtime: args.runtime,
                    source: args.source,
                    summary: args.summary,
                    assumptions: args.assumptions,
                    inputs: args.inputs,
                    outputs: args.outputs,
                    effects: args.effects,
                    result_mode: args.result_mode,
                },
            })
        }
        other => Err(format!("unknown proposal function '{}'", other)),
    }
}

fn parse_args<T: for<'de> Deserialize<'de>>(raw: &str, name: &str) -> Result<T, String> {
    serde_json::from_str(raw).map_err(|e| format!("invalid {} arguments: {}", name, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools() -> Value {
        prompt::tools()
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
        let config = ApiConfig {
            model: "gpt-5.6-luna".into(),
            key: "secret".into(),
            max_tokens: 1024,
            reasoning_effort: "low".into(),
            request_max_bytes: 256 * 1024,
            response_max_bytes: 2 * 1024 * 1024,
        };
        let value: Value = serde_json::from_str(&request_body(&config, "input", true)).unwrap();
        assert_eq!(ENDPOINT, "https://api.openai.com/v1/responses");
        assert_eq!(value["store"], false);
        assert_eq!(value["parallel_tool_calls"], false);
        assert_eq!(value["tool_choice"], "required");
        assert_eq!(value["stream"], true);
        assert!(value.get("previous_response_id").is_none());
        assert_eq!(value["tools"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn accepts_each_strict_tool() {
        assert!(matches!(
            parse_response(&response("return_answer", json!({"text":"hello"}))).unwrap(),
            ProposedAction::Answer { .. }
        ));
        assert!(matches!(
            parse_response(&response(
                "request_clarification",
                json!({"question":"which file?"})
            ))
            .unwrap(),
            ProposedAction::Clarification { .. }
        ));
        let shell = response(
            "run_shell",
            json!({
                "command":"ls","summary":"list","assumptions":[],"effects":["read_local"],
                "requirements":["ls"],"stdin_mode":"none"
            }),
        );
        assert!(matches!(
            parse_response(&shell).unwrap(),
            ProposedAction::Shell { .. }
        ));
        let program = response(
            "run_program",
            json!({
                "runtime":"python3",
                "source":"print('ok')",
                "summary":"Print a result.",
                "assumptions":[],
                "inputs":[],
                "outputs":[],
                "effects":[],
                "result_mode":"stdout"
            }),
        );
        assert!(matches!(
            parse_response(&program).unwrap(),
            ProposedAction::Program { .. }
        ));
        let parent = response(
            "require_parent_shell",
            json!({"kind":"set_environment","path":null,"name":"EDITOR","value":"nvim","summary":"Set the editor.","assumptions":[],"effects":["shell_state"]}),
        );
        assert!(matches!(
            parse_response(&parent).unwrap(),
            ProposedAction::ParentShell { .. }
        ));
    }

    #[test]
    fn rejects_zero_multiple_plain_refusal_unknown_incomplete_and_nonstrict() {
        let base = json!({"status":"completed","tools":tools(),"output":[]});
        assert!(parse_response(&base.to_string()).is_err());
        let mut multiple = base.clone();
        multiple["output"] = json!([
            {"type":"function_call","status":"completed","name":"return_answer","arguments":"{\"text\":\"a\"}"},
            {"type":"function_call","status":"completed","name":"return_answer","arguments":"{\"text\":\"b\"}"}
        ]);
        assert!(parse_response(&multiple.to_string()).is_err());
        let mut plain = base.clone();
        plain["output"] = json!([{"type":"message","status":"completed","content":[{"type":"refusal","refusal":"no"}]}]);
        assert!(parse_response(&plain.to_string()).is_err());
        let mut nonstrict = base.clone();
        nonstrict["tools"][0]["strict"] = json!(false);
        assert!(parse_response(&nonstrict.to_string()).is_err());
        let incomplete = json!({"status":"incomplete","tools":tools(),"output":[]});
        assert!(parse_response(&incomplete.to_string()).is_err());
    }
}
