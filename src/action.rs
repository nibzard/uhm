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
pub enum ProgramFileAccess {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramFile {
    pub id: String,
    pub path: String,
    pub access: ProgramFileAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramStdinMode {
    None,
    LocalPath,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelperProgramProposalV2 {
    pub runtime: ProgramRuntime,
    pub contract: String,
    pub source: String,
    pub summary: String,
    pub assumptions: Vec<String>,
    pub stdin_mode: ProgramStdinMode,
    pub files: Vec<ProgramFile>,
    pub effects: Vec<Effect>,
}

pub type ProgramProposal = HelperProgramProposalV2;

/// Exact schema-v3 program shape retained solely for history compatibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyProgramProposalV1 {
    pub runtime: ProgramRuntime,
    pub source: String,
    pub summary: String,
    pub assumptions: Vec<String>,
    pub inputs: Vec<LegacyProgramInputV1>,
    pub outputs: Vec<String>,
    pub effects: Vec<Effect>,
    pub result_mode: LegacyProgramResultModeV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyProgramInputV1 {
    pub path: String,
    pub access: LegacyProgramInputAccessV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProgramInputAccessV1 {
    ReadOnly,
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProgramResultModeV1 {
    Stdout,
    Artifacts,
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
    pub fn validate(mut self) -> Result<Self, String> {
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
        fn metadata(value: &mut ProposalMetadata) -> Result<(), String> {
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
            value.requirements.retain(|requirement| {
                !is_builtin_label(requirement) && !is_absolute_shell_requirement(requirement)
            });
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
        fn is_builtin_label(value: &str) -> bool {
            let Some((shell, name)) = value.split_once(" builtin: ") else {
                return false;
            };
            matches!(
                shell,
                "sh" | "bash" | "zsh" | "fish" | "pwsh" | "powershell"
            ) && !name.is_empty()
                && !name.starts_with('-')
                && !name.contains('/')
                && !name.chars().any(char::is_whitespace)
        }
        fn is_absolute_shell_requirement(value: &str) -> bool {
            let path = std::path::Path::new(value);
            path.is_absolute()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        matches!(name, "sh" | "bash" | "zsh" | "fish" | "pwsh" | "powershell")
                    })
        }
        fn path(value: &str, label: &str) -> Result<(), String> {
            text(value, label, MAX_PATH)?;
            if value == "stdout" || value.contains('\0') {
                return Err(format!("invalid {}", label));
            }
            Ok(())
        }

        match &mut self {
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
                if program.contract != "uhm_helper_v1" {
                    return Err("new programs must name contract uhm_helper_v1".into());
                }
                text(&program.source, "program source", MAX_SOURCE)?;
                text(&program.summary, "program summary", MAX_ITEM)?;
                if program.assumptions.len() > MAX_ITEMS {
                    return Err("program assumptions contains too many items".into());
                }
                for assumption in &program.assumptions {
                    text(assumption, "program assumption", MAX_ITEM)?;
                }
                if program.files.len() > 64 {
                    return Err("program files contains too many resources".into());
                }
                for file in &program.files {
                    path(&file.path, "program file path")?;
                    let valid_id = file.id.len() <= 32
                        && file.id.bytes().enumerate().all(|(index, byte)| {
                            byte.is_ascii_lowercase()
                                || (index > 0 && (byte.is_ascii_digit() || byte == b'_'))
                        });
                    if !valid_id || file.id.is_empty() {
                        return Err("program resource id is invalid".into());
                    }
                }
                if program.effects.len() > MAX_ITEMS {
                    return Err("program effects contains too many items".into());
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
                requirements: vec!["bin/printf".into()],
                ..ProposalMetadata::default()
            },
            stdin_mode: StdinMode::None,
        };
        assert!(action.validate().is_err());
    }

    #[test]
    fn normalizes_shell_builtins_and_absolute_requirements() {
        let action = ProposedAction::Shell {
            command: "print ok".into(),
            metadata: ProposalMetadata {
                summary: "print".into(),
                requirements: vec!["zsh builtin: print".into(), "/bin/zsh".into()],
                ..ProposalMetadata::default()
            },
            stdin_mode: StdinMode::None,
        }
        .validate()
        .unwrap();
        let ProposedAction::Shell { metadata, .. } = action else {
            unreachable!()
        };
        assert!(metadata.requirements.is_empty());

        let external_shell = ProposedAction::Shell {
            command: "zsh script.zsh".into(),
            metadata: ProposalMetadata {
                summary: "run a script".into(),
                requirements: vec!["zsh".into()],
                ..ProposalMetadata::default()
            },
            stdin_mode: StdinMode::None,
        }
        .validate()
        .unwrap();
        let ProposedAction::Shell { metadata, .. } = external_shell else {
            unreachable!()
        };
        assert_eq!(metadata.requirements, ["zsh"]);
    }

    #[test]
    fn program_manifest_is_bounded_and_paths_are_not_interpolated() {
        let valid = ProposedAction::Program {
            program: ProgramProposal {
                runtime: ProgramRuntime::Python3,
                contract: "uhm_helper_v1".into(),
                source: "from uhm_runtime import stdin_path\nprint(stdin_path.stat().st_size)"
                    .into(),
                summary: "Count declared inputs.".into(),
                assumptions: vec![],
                stdin_mode: ProgramStdinMode::LocalPath,
                files: vec![],
                effects: vec![Effect::ReadLocal],
            },
        };
        assert!(valid.validate().is_ok());

        let embedded = ProposedAction::Program {
            program: ProgramProposal {
                runtime: ProgramRuntime::Python3,
                contract: "uhm_helper_v1".into(),
                source: "open('private.txt').read()".into(),
                summary: "Read a file.".into(),
                assumptions: vec![],
                stdin_mode: ProgramStdinMode::None,
                files: vec![ProgramFile {
                    id: "source".into(),
                    path: "private.txt".into(),
                    access: ProgramFileAccess::ReadOnly,
                }],
                effects: vec![Effect::ReadLocal],
            },
        };
        assert!(embedded.validate().is_ok());
    }

    #[test]
    fn helper_resources_require_contract_ids_and_unique_semantics_are_preflighted() {
        for access in [
            ProgramFileAccess::ReadOnly,
            ProgramFileAccess::WriteOnly,
            ProgramFileAccess::ReadWrite,
        ] {
            let action = ProposedAction::Program {
                program: ProgramProposal {
                    runtime: ProgramRuntime::Python3,
                    contract: "uhm_helper_v1".into(),
                    source: "from uhm_runtime import resource\nprint(resource('item'))".into(),
                    summary: "Use one resource".into(),
                    assumptions: vec![],
                    stdin_mode: ProgramStdinMode::None,
                    files: vec![ProgramFile {
                        id: "item_1".into(),
                        path: "-- café/file.txt".into(),
                        access,
                    }],
                    effects: vec![Effect::ReadLocal],
                },
            };
            assert!(action.validate().is_ok());
        }
        let wrong_contract = ProposedAction::Program {
            program: ProgramProposal {
                runtime: ProgramRuntime::Python3,
                contract: "manifest_env_v1".into(),
                source: "print('x')".into(),
                summary: "Print".into(),
                assumptions: vec![],
                stdin_mode: ProgramStdinMode::None,
                files: vec![],
                effects: vec![],
            },
        };
        assert!(wrong_contract.validate().is_err());
    }
}
