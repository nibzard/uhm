//! Bounded semantic-event parser for streamed Responses API results.

use serde_json::Value;
use std::io::{BufRead, BufReader, Read};

const MAX_EVENT_LINE: usize = 256 * 1024;
const MAX_ARGUMENTS: usize = 64 * 1024;

pub fn read_responses_stream<R: Read>(reader: R, max_stream: usize) -> Result<String, String> {
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let mut total = 0usize;
    let mut argument_bytes = 0usize;

    loop {
        line.clear();
        let count = reader
            .read_line(&mut line)
            .map_err(|e| format!("read Responses stream: {}", e))?;
        if count == 0 {
            return Err("Responses stream ended before response.completed".into());
        }
        total = total.saturating_add(count);
        if count > MAX_EVENT_LINE || total > max_stream {
            return Err("Responses stream exceeded the configured size limit".into());
        }
        let Some(data) = line
            .trim_end_matches(['\n', '\r'])
            .strip_prefix("data:")
            .map(str::trim)
        else {
            continue;
        };
        if data.is_empty() {
            continue;
        }
        let event: Value = serde_json::from_str(data)
            .map_err(|e| format!("invalid Responses stream event: {}", e))?;
        let kind = event["type"].as_str().unwrap_or("");
        match kind {
            "response.created"
            | "response.in_progress"
            | "response.output_item.added"
            | "response.content_part.added"
            | "response.content_part.done" => {}
            "response.function_call_arguments.delta" => {
                argument_bytes =
                    argument_bytes.saturating_add(event["delta"].as_str().unwrap_or("").len());
                if argument_bytes > MAX_ARGUMENTS {
                    return Err("streamed function arguments exceeded 65536 bytes".into());
                }
            }
            "response.function_call_arguments.done" => {
                if event["arguments"].as_str().unwrap_or("").len() > MAX_ARGUMENTS {
                    return Err("completed function arguments exceeded 65536 bytes".into());
                }
            }
            "response.output_item.done" => {
                if event["item"]["type"] == "function_call"
                    && event["item"]["arguments"].as_str().unwrap_or("").len() > MAX_ARGUMENTS
                {
                    return Err("function-call output item exceeded 65536 bytes".into());
                }
            }
            "response.refusal.delta" | "response.refusal.done" => {
                return Err("model refused the requested typed action".into())
            }
            "response.incomplete" => {
                return Err(format!(
                    "OpenAI response was incomplete: {}",
                    event["response"]["incomplete_details"]
                ))
            }
            "response.failed" => {
                return Err(api_error(
                    &event["response"]["error"],
                    "OpenAI response failed",
                ))
            }
            "error" => return Err(api_error(&event, "OpenAI stream error")),
            "response.completed" => {
                let response = event
                    .get("response")
                    .ok_or("response.completed did not include a response")?;
                return serde_json::to_string(response)
                    .map_err(|e| format!("serialize completed response: {}", e));
            }
            other if other.starts_with("response.output_text") => {
                return Err("plain-text model output is not a valid uhm action".into())
            }
            _ => {}
        }
    }
}

fn api_error(value: &Value, prefix: &str) -> String {
    let message = value["message"].as_str().unwrap_or("unknown API error");
    format!(
        "{}: {}",
        prefix,
        crate::render::ansi::sanitize_untrusted(message)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_only_completed_response() {
        let stream = concat!(
            "event: response.created\ndata: {\"type\":\"response.created\"}\n\n",
            "event: response.function_call_arguments.delta\ndata: {\"type\":\"response.function_call_arguments.delta\",\"delta\":\"{}\"}\n\n",
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[]}}\n\n"
        );
        let result = read_responses_stream(stream.as_bytes(), 2 * 1024 * 1024).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&result).unwrap()["status"],
            "completed"
        );
    }

    #[test]
    fn rejects_failure_refusal_interruption_and_oversize() {
        assert!(read_responses_stream(
            b"data: {\"type\":\"response.refusal.done\"}\n".as_slice(),
            2 * 1024 * 1024,
        )
        .unwrap_err()
        .contains("refused"));
        assert!(read_responses_stream(
            b"data: {\"type\":\"response.created\"}\n".as_slice(),
            2 * 1024 * 1024,
        )
        .unwrap_err()
        .contains("ended"));
        let oversized = format!(
            "data: {{\"type\":\"response.function_call_arguments.delta\",\"delta\":\"{}\"}}\n",
            "x".repeat(MAX_ARGUMENTS + 1)
        );
        assert!(read_responses_stream(oversized.as_bytes(), 2 * 1024 * 1024).is_err());
    }
}
