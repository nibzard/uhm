//! Typed boundary between model proposals, local policy, and execution.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    ReadLocal,
    WriteLocal,
    DeleteLocal,
    NetworkRead,
    RemoteMutation,
    PrivilegeElevation,
    ProcessControl,
    ShellState,
    Unknown,
}

impl Effect {
    pub fn label(&self) -> &'static str {
        match self {
            Self::ReadLocal => "reads local data",
            Self::WriteLocal => "writes local data",
            Self::DeleteLocal => "deletes local data",
            Self::NetworkRead => "uses the network",
            Self::RemoteMutation => "changes remote state",
            Self::PrivilegeElevation => "uses elevated privileges",
            Self::ProcessControl => "controls processes",
            Self::ShellState => "changes the parent shell",
            Self::Unknown => "has effects uhm could not classify",
        }
    }

    pub fn requires_advisory_pause(&self) -> bool {
        matches!(
            self,
            Self::DeleteLocal
                | Self::RemoteMutation
                | Self::PrivilegeElevation
                | Self::ProcessControl
                | Self::ShellState
                | Self::Unknown
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalMetadata {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default)]
    pub effects: Vec<Effect>,
    #[serde(default)]
    pub requirements: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposedAction {
    Answer {
        text: String,
        metadata: ProposalMetadata,
    },
    Shell(ShellAction),
    ParentShell(ParentShellAction),
    Clarification {
        question: String,
        metadata: ProposalMetadata,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellAction {
    pub command: String,
    pub metadata: ProposalMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentShellAction {
    pub command: String,
    pub metadata: ProposalMetadata,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireProposal {
    pub kind: String,
    pub command: Option<String>,
    pub text: Option<String>,
    pub question: Option<String>,
    pub summary: String,
    pub assumptions: Vec<String>,
    pub effects: Vec<Effect>,
    pub requirements: Vec<String>,
}

impl TryFrom<WireProposal> for ProposedAction {
    type Error = String;

    fn try_from(value: WireProposal) -> Result<Self, Self::Error> {
        fn checked_command(command: String) -> Result<String, String> {
            if command.trim().is_empty() {
                return Err("model returned an empty command".into());
            }
            if command
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
            {
                return Err("model returned a command containing unsafe control bytes".into());
            }
            Ok(command)
        }

        let metadata = ProposalMetadata {
            summary: value.summary,
            assumptions: value.assumptions,
            effects: value.effects,
            requirements: value.requirements,
        };
        match value.kind.as_str() {
            "answer" => Ok(Self::Answer {
                text: value.text.ok_or("answer proposal is missing text")?,
                metadata,
            }),
            "shell" => Ok(Self::Shell(ShellAction {
                command: checked_command(
                    value.command.ok_or("shell proposal is missing command")?,
                )?,
                metadata,
            })),
            "parent_shell" => Ok(Self::ParentShell(ParentShellAction {
                command: checked_command(
                    value
                        .command
                        .ok_or("parent-shell proposal is missing command")?,
                )?,
                metadata,
            })),
            "clarification" => Ok(Self::Clarification {
                question: value
                    .question
                    .ok_or("clarification proposal is missing question")?,
                metadata,
            }),
            other => Err(format!(
                "model returned unsupported action kind '{}'",
                other
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_terminal_control_bytes_in_commands() {
        let wire = WireProposal {
            kind: "shell".into(),
            command: Some("printf '\u{1b}[2J'".into()),
            text: None,
            question: None,
            summary: String::new(),
            assumptions: vec![],
            effects: vec![],
            requirements: vec![],
        };
        assert!(ProposedAction::try_from(wire).is_err());
    }
}
