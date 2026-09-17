//! Canonical decoding and validation for every model-authored action.

use crate::action::{
    Effect, ParentAction, ParentActionKind, ProgramFile, ProgramProposal, ProgramRuntime,
    ProgramStdinMode, ProposalMetadata, ProposedAction, StdinMode,
};
use serde::Deserialize;
use serde_json::{json, Value};

pub const CONTEXT_POLICY_VERSION: u32 = 4;
pub const PROGRAM_CONTRACT: &str = "uhm_helper_v1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerArgs {
    text: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClarificationArgs {
    question: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeSubcommandArgs {
    tool: String,
    subcommand: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShellArgs {
    command: String,
    summary: String,
    assumptions: Vec<String>,
    effects: Vec<Effect>,
    requirements: Vec<String>,
    stdin_mode: StdinMode,
}
#[derive(Debug, Deserialize)]
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
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProgramArgs {
    runtime: ProgramRuntime,
    contract: String,
    source: String,
    summary: String,
    assumptions: Vec<String>,
    stdin_mode: ProgramStdinMode,
    files: Vec<ProgramFile>,
    effects: Vec<Effect>,
}

fn decode<T: for<'de> Deserialize<'de>>(value: Value, tool: &str) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("invalid {tool} arguments: {error}"))
}

/// Decode one provider-neutral tool envelope and apply the production semantic validator.
pub fn decode_and_validate(tool: &str, arguments: Value) -> Result<ProposedAction, String> {
    let action = match tool {
        "return_answer" => {
            let args: AnswerArgs = decode(arguments, tool)?;
            ProposedAction::Answer { text: args.text }
        }
        "request_clarification" => {
            let args: ClarificationArgs = decode(arguments, tool)?;
            ProposedAction::Clarification {
                question: args.question,
            }
        }
        "probe_subcommand" => {
            let args: ProbeSubcommandArgs = decode(arguments, tool)?;
            ProposedAction::ProbeSubcommand {
                tool: args.tool,
                subcommand: args.subcommand,
            }
        }
        "run_shell" => {
            let args: ShellArgs = decode(arguments, tool)?;
            ProposedAction::Shell {
                command: args.command,
                metadata: ProposalMetadata {
                    summary: args.summary,
                    assumptions: args.assumptions,
                    effects: args.effects,
                    requirements: args.requirements,
                },
                stdin_mode: args.stdin_mode,
            }
        }
        "require_parent_shell" => {
            let args: ParentArgs = decode(arguments, tool)?;
            ProposedAction::ParentShell {
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
            }
        }
        "run_program" => {
            let args: ProgramArgs = decode(arguments, tool)?;
            ProposedAction::Program {
                program: ProgramProposal {
                    runtime: args.runtime,
                    contract: args.contract,
                    source: args.source,
                    summary: args.summary,
                    assumptions: args.assumptions,
                    stdin_mode: args.stdin_mode,
                    files: args.files,
                    effects: args.effects,
                },
            }
        }
        _ => return Err(format!("unknown proposal function '{tool}'")),
    };
    action.validate()
}

/// The closed mapping from wire tool names to canonical runtime action
/// kinds. Rust is authoritative: external tooling consumes this table from
/// `describe` instead of keeping a second handwritten one. `probe_subcommand`
/// is a routing step, never an executable evidence profile entry, and is
/// deliberately absent.
pub fn tool_to_action_kind(tool: &str) -> Option<&'static str> {
    match tool {
        "return_answer" => Some("answer"),
        "request_clarification" => Some("clarification"),
        "run_shell" => Some("shell"),
        "run_program" => Some("program"),
        "require_parent_shell" => Some("parent_shell"),
        _ => None,
    }
}

pub fn description() -> Value {
    json!({
        "contract_version": 2,
        "prompt_version": crate::prompt::PROMPT_VERSION,
        "action_schema_version": crate::prompt::ACTION_SCHEMA_VERSION,
        "context_policy_version": CONTEXT_POLICY_VERSION,
        "program_contract": PROGRAM_CONTRACT,
        "developer_instructions": crate::prompt::DEVELOPER_INSTRUCTIONS,
        "tools": crate::prompt::tools(),
        "tool_to_action_kind": {
            "return_answer": "answer",
            "request_clarification": "clarification",
            "run_shell": "shell",
            "run_program": "program",
            "require_parent_shell": "parent_shell",
        },
    })
}

pub fn rejection_code(message: &str) -> &'static str {
    if message.starts_with("unknown proposal function") {
        "unknown_tool"
    } else if message.contains("unknown field") {
        "unknown_field"
    } else if message.contains("missing field") {
        "missing_field"
    } else if message.contains("duplicate") {
        "duplicate_resource"
    } else if message.contains("resource id") {
        "invalid_resource"
    } else if message.contains("unknown variant")
        && message.contains("read_only")
        && message.contains("write_only")
    {
        "invalid_resource_access"
    } else if message.contains("contract uhm_helper_v1") {
        "invalid_program_contract"
    } else if message.contains("parent-shell") {
        "invalid_parent_shell"
    } else if message.contains("executable requirement") {
        "invalid_requirement"
    } else if message.contains("manifest paths") {
        "embedded_logical_path"
    } else if message.contains("replacement input") {
        "replacement_output_mismatch"
    } else if message.contains("stdout programs") || message.contains("artifact programs") {
        "result_mode_mismatch"
    } else if message.contains("exceeds") || message.contains("too many") {
        "bounds_exceeded"
    } else if message.contains("control") || message.contains("NUL") {
        "unsafe_control_bytes"
    } else {
        "invalid_action"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_decoder_rejects_semantically_invalid_wire_valid_action() {
        let error = decode_and_validate(
            "run_shell",
            json!({
                "command":"ls", "summary":"List", "assumptions":[], "effects":["read_local"],
                "requirements":["/bin/ls"], "stdin_mode":"none"
            }),
        )
        .unwrap_err();
        assert_eq!(rejection_code(&error), "invalid_requirement");
    }

    // Plan 19 W10: Rust owns the wire-tool to action-kind mapping; external
    // tooling consumes it from describe instead of a second table.

    #[test]
    fn audit19_qualification_tool_to_action_kind_matches_canonical_actions() {
        for (tool, expected) in [
            ("return_answer", "answer"),
            ("request_clarification", "clarification"),
            ("run_shell", "shell"),
            ("run_program", "program"),
            ("require_parent_shell", "parent_shell"),
        ] {
            let kind = tool_to_action_kind(tool).unwrap_or_else(|| panic!("{tool} unmapped"));
            assert_eq!(kind, expected);
            // Every mapped pair ties to the decoded canonical action and the
            // runtime action type of that action.
            let envelope = match tool {
                "return_answer" => serde_json::json!({"text":"done"}),
                "request_clarification" => serde_json::json!({"question":"which?"}),
                "run_shell" => {
                    serde_json::json!({"command":"true","summary":"s","assumptions":[],"effects":["read_local"],"requirements":[],"stdin_mode":"none"})
                }
                "run_program" => {
                    serde_json::json!({"runtime":"python3","contract":"uhm_helper_v1","source":"pass","summary":"s","assumptions":[],"stdin_mode":"none","files":[],"effects":["read_local"]})
                }
                _ => {
                    serde_json::json!({"kind":"set_environment","name":"UHM_X","value":"1","summary":"s","assumptions":[],"effects":["shell_state"]})
                }
            };
            let action = decode_and_validate(tool, envelope)
                .unwrap_or_else(|error| panic!("{tool} must decode: {error}"));
            assert_eq!(
                crate::model_selection::action_type(&action),
                kind,
                "{tool} decoded action must map to its kind"
            );
        }
    }

    #[test]
    fn audit19_qualification_unknown_wire_names_are_rejected() {
        for tool in ["run_shell_command", "probe_subcommand", "run_shell ", ""] {
            assert!(
                tool_to_action_kind(tool).is_none(),
                "{tool:?} must not map to an action kind"
            );
        }
        let described = description();
        let mapping = &described["tool_to_action_kind"];
        assert_eq!(described["contract_version"], 2);
        assert_eq!(mapping.as_object().unwrap().len(), 5);
        assert!(mapping.get("probe_subcommand").is_none());
    }
}
