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
    /// Stable, content-free identifier used by history and telemetry schemas.
    pub fn wire_name(&self) -> &'static str {
        match self {
            Self::ReadLocal => "read_local",
            Self::WriteLocal => "write_local",
            Self::DeleteLocal => "delete_local",
            Self::NetworkRead => "network_read",
            Self::RemoteMutation => "remote_mutation",
            Self::PrivilegeElevation => "privilege_elevation",
            Self::ProcessControl => "process_control",
            Self::ShellState => "shell_state",
            Self::Unknown => "unknown",
        }
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramRuntime {
    Python3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramInputAccess {
    ReadOnly,
    Replace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramInput {
    pub path: String,
    pub access: ProgramInputAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramResultMode {
    Stdout,
    Artifacts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramProposal {
    pub runtime: ProgramRuntime,
    pub source: String,
    pub summary: String,
    pub assumptions: Vec<String>,
    pub inputs: Vec<ProgramInput>,
    pub outputs: Vec<String>,
    pub effects: Vec<Effect>,
    pub result_mode: ProgramResultMode,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalMetadata {
    pub summary: String,
    pub assumptions: Vec<String>,
    pub effects: Vec<Effect>,
    pub requirements: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParentActionKind {
    ChangeDirectory,
    SetEnvironment,
    UnsetEnvironment,
    SourceFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParentAction {
    pub kind: ParentActionKind,
    pub path: Option<String>,
    pub name: Option<String>,
    pub value: Option<String>,
}

impl ParentAction {
    pub fn validate(&self) -> Result<(), String> {
        fn bounded(value: &str, label: &str, max: usize) -> Result<(), String> {
            if value.is_empty() || value.len() > max || value.contains('\0') {
                return Err(format!(
                    "parent-shell {} is empty, oversized, or contains NUL",
                    label
                ));
            }
            Ok(())
        }
        let legal = match self.kind {
            ParentActionKind::ChangeDirectory | ParentActionKind::SourceFile => {
                self.path.is_some() && self.name.is_none() && self.value.is_none()
            }
            ParentActionKind::SetEnvironment => {
                self.path.is_none() && self.name.is_some() && self.value.is_some()
            }
            ParentActionKind::UnsetEnvironment => {
                self.path.is_none() && self.name.is_some() && self.value.is_none()
            }
        };
        if !legal {
            return Err("parent-shell action fields do not match its kind".into());
        }
        if let Some(path) = &self.path {
            bounded(path, "path", 4096)?;
        }
        if let Some(name) = &self.name {
            if name.len() > 255
                || name.is_empty()
                || !name.bytes().enumerate().all(|(index, byte)| {
                    byte == b'_'
                        || byte.is_ascii_alphabetic()
                        || (index > 0 && byte.is_ascii_digit())
                })
            {
                return Err("parent-shell environment name is invalid".into());
            }
        }
        if let Some(value) = &self.value {
            if value.len() > 16 * 1024 || value.contains('\0') {
                return Err("parent-shell environment value is oversized or contains NUL".into());
            }
        }
        Ok(())
    }
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
        action: ParentAction,
        metadata: ProposalMetadata,
    },
    Program {
        program: ProgramProposal,
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
        const MAX_SOURCE: usize = 64 * 1024;
        const MAX_PATH: usize = 4096;

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
        fn path(value: &str, label: &str) -> Result<(), String> {
            text(value, label, MAX_PATH)?;
            if value == "stdout" || value.contains('\0') {
                return Err(format!("invalid {}", label));
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
            } => {
                text(command, "command", MAX_COMMAND)?;
                metadata(value)?;
            }
            Self::ParentShell {
                action,
                metadata: value,
            } => {
                action.validate()?;
                metadata(value)?;
            }
            Self::Program { program } => {
                text(&program.source, "program source", MAX_SOURCE)?;
                text(&program.summary, "program summary", MAX_ITEM)?;
                if program.assumptions.len() > MAX_ITEMS {
                    return Err("program assumptions contains too many items".into());
                }
                for assumption in &program.assumptions {
                    text(assumption, "program assumption", MAX_ITEM)?;
                }
                if program.inputs.len() > 64 {
                    return Err("program inputs contains too many paths".into());
                }
                if program.outputs.len() > 16 {
                    return Err("program outputs contains too many paths".into());
                }
                let mut seen = std::collections::BTreeSet::new();
                for input in &program.inputs {
                    path(&input.path, "program input path")?;
                    if !seen.insert(format!("input:{}", input.path)) {
                        return Err("program inputs contains a duplicate path".into());
                    }
                }
                let mut output_seen = std::collections::BTreeSet::new();
                for output in &program.outputs {
                    path(output, "program output path")?;
                    if output == "stdin" {
                        return Err("stdin is a special input path, not an artifact output".into());
                    }
                    if !output_seen.insert(output) {
                        return Err("program outputs contains a duplicate path".into());
                    }
                }
                if program.effects.len() > MAX_ITEMS {
                    return Err("program effects contains too many items".into());
                }
                match program.result_mode {
                    ProgramResultMode::Stdout if !program.outputs.is_empty() => {
                        return Err("stdout programs must not declare artifact outputs".into())
                    }
                    ProgramResultMode::Artifacts if program.outputs.is_empty() => {
                        return Err("artifact programs must declare at least one output".into())
                    }
                    _ => {}
                }
                for input in &program.inputs {
                    if input.access == ProgramInputAccess::Replace
                        && !program.outputs.contains(&input.path)
                    {
                        return Err(format!(
                            "replacement input '{}' must also be a declared output",
                            input.path
                        ));
                    }
                }
                for manifest_path in program
                    .inputs
                    .iter()
                    .map(|input| input.path.as_str())
                    .chain(program.outputs.iter().map(String::as_str))
                    .filter(|value| *value != "stdin")
                {
                    if program.source.contains(manifest_path) {
                        return Err("program source must receive manifest paths through UHM_PROGRAM_INPUTS/UHM_PROGRAM_OUTPUTS, not embed them".into());
                    }
                }
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

    #[test]
    fn program_manifest_is_bounded_and_paths_are_not_interpolated() {
        let valid = ProposedAction::Program {
            program: ProgramProposal {
                runtime: ProgramRuntime::Python3,
                source: "import os, json\nprint(len(json.loads(os.environ['UHM_PROGRAM_INPUTS'])))"
                    .into(),
                summary: "Count declared inputs.".into(),
                assumptions: vec![],
                inputs: vec![ProgramInput {
                    path: "stdin".into(),
                    access: ProgramInputAccess::ReadOnly,
                }],
                outputs: vec![],
                effects: vec![Effect::ReadLocal],
                result_mode: ProgramResultMode::Stdout,
            },
        };
        assert!(valid.validate().is_ok());

        let embedded = ProposedAction::Program {
            program: ProgramProposal {
                runtime: ProgramRuntime::Python3,
                source: "open('private.txt').read()".into(),
                summary: "Read a file.".into(),
                assumptions: vec![],
                inputs: vec![ProgramInput {
                    path: "private.txt".into(),
                    access: ProgramInputAccess::ReadOnly,
                }],
                outputs: vec![],
                effects: vec![Effect::ReadLocal],
                result_mode: ProgramResultMode::Stdout,
            },
        };
        assert!(embedded.validate().is_err());
    }
}
