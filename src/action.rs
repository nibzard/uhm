//! Typed boundary between OpenAI function calls, local policy, and execution.

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StdinMode {
    None,
    Original,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalMetadata {
    pub summary: String,
    pub assumptions: Vec<String>,
    pub effects: Vec<Effect>,
    pub requirements: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProposedAction {
    Answer {
        text: String,
    },
    Shell {
        command: String,
        metadata: ProposalMetadata,
        stdin_mode: StdinMode,
    },
    ParentShell {
        command: String,
        metadata: ProposalMetadata,
    },
    Clarification {
        question: String,
    },
}

impl ProposedAction {
    pub fn validate(self) -> Result<Self, String> {
        const MAX_COMMAND: usize = 32 * 1024;
        const MAX_TEXT: usize = 64 * 1024;
        const MAX_ITEMS: usize = 32;
        const MAX_ITEM: usize = 1024;

        fn text(value: &str, label: &str, max: usize) -> Result<(), String> {
            if value.trim().is_empty() {
                return Err(format!("{} is empty", label));
            }
            if value.len() > max {
                return Err(format!("{} exceeds {} bytes", label, max));
            }
            if value
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
            {
                return Err(format!("{} contains unsafe control bytes", label));
            }
            Ok(())
        }
        fn metadata(value: &ProposalMetadata) -> Result<(), String> {
            text(&value.summary, "summary", MAX_ITEM)?;
            for (label, items) in [
                ("assumptions", &value.assumptions),
                ("requirements", &value.requirements),
            ] {
                if items.len() > MAX_ITEMS {
                    return Err(format!("{} contains too many items", label));
                }
                for item in items {
                    text(item, label, MAX_ITEM)?;
                }
            }
            if value.effects.len() > MAX_ITEMS {
                return Err("effects contains too many items".into());
            }
            for requirement in &value.requirements {
                if requirement.contains('/')
                    || requirement.chars().any(char::is_whitespace)
                    || requirement.starts_with('-')
                {
                    return Err(format!("invalid executable requirement '{}'", requirement));
                }
            }
            Ok(())
        }

        match &self {
            Self::Answer { text: value } => text(value, "answer", MAX_TEXT)?,
            Self::Clarification { question } => text(question, "clarification", MAX_ITEM)?,
            Self::Shell {
                command,
                metadata: value,
                ..
            }
            | Self::ParentShell {
                command,
                metadata: value,
            } => {
                text(command, "command", MAX_COMMAND)?;
                metadata(value)?;
            }
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_terminal_control_bytes_and_invalid_requirements() {
        let action = ProposedAction::Shell {
            command: "printf '\u{1b}[2J'".into(),
            metadata: ProposalMetadata {
                summary: "print".into(),
                requirements: vec!["/bin/printf".into()],
                ..ProposalMetadata::default()
            },
            stdin_mode: StdinMode::None,
        };
        assert!(action.validate().is_err());
    }
}
