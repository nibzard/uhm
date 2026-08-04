//! Strict config resolution: defaults <- config.yaml <- environment <- CLI.

use crate::dirs::{self, Paths};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::provider::{ProviderErrorKind, ProviderId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionMode {
    Fixed,
    Evidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCandidate {
    pub provider: ProviderId,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SelectionConfig {
    pub mode: SelectionMode,
    pub alternate: Option<ModelCandidate>,
    pub fallback_on: Vec<ProviderErrorKind>,
}

impl Default for SelectionConfig {
    fn default() -> Self {
        Self {
            mode: SelectionMode::Fixed,
            alternate: None,
            fallback_on: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryDetail {
    Metadata,
    Diagnostic,
    Full,
}

impl HistoryDetail {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::Diagnostic => "diagnostic",
            Self::Full => "full",
        }
    }

    pub fn retains_proposals(self) -> bool {
        matches!(self, Self::Diagnostic | Self::Full)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HistoryConfig {
    pub enabled: bool,
    pub detail: HistoryDetail,
    pub capture_output: bool,
    pub redact_paths: bool,
    pub max_records: usize,
    pub max_age_days: u64,
    pub max_bytes: u64,
    pub artifact_max_bytes: usize,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            detail: HistoryDetail::Metadata,
            capture_output: false,
            redact_paths: true,
            max_records: 500,
            max_age_days: 30,
            max_bytes: 256 * 1024 * 1024,
            artifact_max_bytes: 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExecutionConfig {
    pub timeout_secs: u64,
    pub diagnostic_bytes: usize,
    pub deny_common_env: bool,
    pub deny_env: Vec<String>,
    pub containment: crate::containment::Mode,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShellContextConfig {
    pub last_history_entry: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TelemetryConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProgramConfig {
    pub enabled: bool,
    pub source_max_bytes: usize,
    pub input_max_paths: usize,
    pub output_max_paths: usize,
    pub workspace_max_bytes: u64,
    pub timeout_secs: u64,
    pub cpu_secs: u64,
    pub address_space_bytes: u64,
    pub open_files: u64,
    pub child_processes: u64,
    pub output_max_bytes: usize,
    pub diagnostic_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RecoveryConfig {
    pub enabled: bool,
    pub max_age_days: u64,
    pub max_total_bytes: u64,
    pub max_file_bytes: u64,
    pub scan_limit: usize,
    pub prune_batch: usize,
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_age_days: 14,
            max_total_bytes: 128 * 1024 * 1024,
            max_file_bytes: 8 * 1024 * 1024,
            scan_limit: 1_000,
            prune_batch: 100,
        }
    }
}

impl Default for ProgramConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            source_max_bytes: 64 * 1024,
            input_max_paths: 64,
            output_max_paths: 16,
            workspace_max_bytes: 64 * 1024 * 1024,
            timeout_secs: 10,
            cpu_secs: 5,
            address_space_bytes: 256 * 1024 * 1024,
            open_files: 64,
            child_processes: 16,
            output_max_bytes: 16 * 1024 * 1024,
            diagnostic_bytes: 1024 * 1024,
        }
    }
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 300,
            diagnostic_bytes: 65_536,
            deny_common_env: false,
            deny_env: Vec::new(),
            containment: crate::containment::Mode::Off,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Config {
    pub provider: ProviderId,
    pub model: String,
    pub selection: SelectionConfig,
    pub max_completion_tokens: u32,
    pub reasoning_effort: String,
    pub stream: bool,
    pub shell: String,
    pub context_mode: String,
    pub context_timeout_ms: u64,
    pub stdin_max_bytes: usize,
    pub stdin_first_byte_timeout_ms: u64,
    pub request_max_bytes: usize,
    pub response_max_bytes: usize,
    pub history: HistoryConfig,
    pub execution: ExecutionConfig,
    pub telemetry: TelemetryConfig,
    pub program: ProgramConfig,
    pub recovery: RecoveryConfig,
    pub shell_context: ShellContextConfig,
    pub cache_enabled: bool,
    pub cache_ttl_secs: u64,
    pub aliases: Vec<(String, String)>,
    #[serde(skip)]
    pub paths: Paths,
    #[serde(skip)]
    sources: BTreeMap<&'static str, &'static str>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    provider: Option<ProviderId>,
    model: Option<String>,
    selection: Option<SelectionConfig>,
    max_completion_tokens: Option<u32>,
    reasoning_effort: Option<String>,
    stream: Option<bool>,
    shell: Option<String>,
    context_mode: Option<String>,
    context_timeout_ms: Option<u64>,
    stdin_max_bytes: Option<usize>,
    stdin_first_byte_timeout_ms: Option<u64>,
    request_max_bytes: Option<usize>,
    response_max_bytes: Option<usize>,
    history: Option<HistoryConfig>,
    execution: Option<ExecutionConfig>,
    telemetry: Option<TelemetryConfig>,
    program: Option<ProgramConfig>,
    recovery: Option<RecoveryConfig>,
    shell_context: Option<ShellContextConfig>,
    cache_enabled: Option<bool>,
    cache_ttl_secs: Option<u64>,
    aliases: Option<BTreeMap<String, String>>,
}

const KEYS: &[&str] = &[
    "provider",
    "model",
    "selection",
    "max_completion_tokens",
    "reasoning_effort",
    "stream",
    "shell",
    "context_mode",
    "context_timeout_ms",
    "stdin_max_bytes",
    "stdin_first_byte_timeout_ms",
    "request_max_bytes",
    "response_max_bytes",
    "history",
    "execution",
    "telemetry",
    "program",
    "recovery",
    "shell_context",
    "cache_enabled",
    "cache_ttl_secs",
    "aliases",
];

impl Config {
    fn defaults(paths: Paths) -> Self {
        let mut sources = BTreeMap::new();
        for key in KEYS {
            sources.insert(*key, "default");
        }
        Self {
            provider: ProviderId::Openai,
            model: "gpt-5.6-terra".into(),
            selection: SelectionConfig::default(),
            max_completion_tokens: 8192,
            reasoning_effort: "low".into(),
            stream: true,
            shell: "auto".into(),
            context_mode: "standard".into(),
            context_timeout_ms: 150,
            stdin_max_bytes: 16 * 1024 * 1024,
            stdin_first_byte_timeout_ms: 1_000,
            request_max_bytes: 256 * 1024,
            response_max_bytes: 2 * 1024 * 1024,
            history: HistoryConfig::default(),
            execution: ExecutionConfig::default(),
            telemetry: TelemetryConfig::default(),
            program: ProgramConfig::default(),
            recovery: RecoveryConfig::default(),
            shell_context: ShellContextConfig::default(),
            cache_enabled: true,
            cache_ttl_secs: 86_400,
            aliases: Vec::new(),
            paths,
            sources,
        }
    }

    #[cfg(test)]
    pub(crate) fn test(paths: Paths) -> Self {
        Self::defaults(paths)
    }

    pub fn source(&self, key: &str) -> &'static str {
        self.sources.get(key).copied().unwrap_or("unknown")
    }

    pub fn show_lines(&self) -> Vec<(&'static str, String, &'static str)> {
        vec![
            (
                "provider",
                self.provider.to_string(),
                self.source("provider"),
            ),
            ("model", self.model.clone(), self.source("model")),
            (
                "selection.mode",
                match self.selection.mode {
                    SelectionMode::Fixed => "fixed",
                    SelectionMode::Evidence => "evidence",
                }
                .into(),
                self.source("selection"),
            ),
            (
                "selection.alternate",
                self.selection
                    .alternate
                    .as_ref()
                    .map(|value| format!("{}:{}", value.provider, value.model))
                    .unwrap_or_else(|| "none".into()),
                self.source("selection"),
            ),
            (
                "selection.fallback_on",
                if self.selection.fallback_on.is_empty() {
                    "none".into()
                } else {
                    self.selection
                        .fallback_on
                        .iter()
                        .map(|value| format!("{value:?}").to_ascii_lowercase())
                        .collect::<Vec<_>>()
                        .join(",")
                },
                self.source("selection"),
            ),
            (
                "qualification_status",
                crate::model_selection::provider_status(self.provider, self.selection.mode).into(),
                "checked manifest",
            ),
            ("shell", self.shell.clone(), self.source("shell")),
            ("stream", self.stream.to_string(), self.source("stream")),
            (
                "max_completion_tokens",
                self.max_completion_tokens.to_string(),
                self.source("max_completion_tokens"),
            ),
            (
                "reasoning_effort",
                self.reasoning_effort.clone(),
                self.source("reasoning_effort"),
            ),
            (
                "context_mode",
                self.context_mode.clone(),
                self.source("context_mode"),
            ),
            (
                "context_timeout_ms",
                self.context_timeout_ms.to_string(),
                self.source("context_timeout_ms"),
            ),
            (
                "stdin_max_bytes",
                self.stdin_max_bytes.to_string(),
                self.source("stdin_max_bytes"),
            ),
            (
                "stdin_first_byte_timeout_ms",
                self.stdin_first_byte_timeout_ms.to_string(),
                self.source("stdin_first_byte_timeout_ms"),
            ),
            (
                "request_max_bytes",
                self.request_max_bytes.to_string(),
                self.source("request_max_bytes"),
            ),
            (
                "response_max_bytes",
                self.response_max_bytes.to_string(),
                self.source("response_max_bytes"),
            ),
            (
                "history.enabled",
                self.history.enabled.to_string(),
                self.source("history"),
            ),
            (
                "history.detail",
                self.history.detail.as_str().into(),
                self.source("history"),
            ),
            (
                "history.capture_output",
                self.history.capture_output.to_string(),
                self.source("history"),
            ),
            (
                "history.redact_paths",
                self.history.redact_paths.to_string(),
                self.source("history"),
            ),
            (
                "shell_context.last_history_entry",
                self.shell_context.last_history_entry.to_string(),
                self.source("shell_context"),
            ),
            (
                "execution.timeout_secs",
                self.execution.timeout_secs.to_string(),
                self.source("execution"),
            ),
            (
                "execution.deny_common_env",
                self.execution.deny_common_env.to_string(),
                self.source("execution"),
            ),
            (
                "execution.containment",
                self.execution.containment.as_str().into(),
                self.source("execution"),
            ),
            (
                "telemetry.enabled",
                self.telemetry.enabled.to_string(),
                self.source("telemetry"),
            ),
            (
                "program.enabled",
                self.program.enabled.to_string(),
                self.source("program"),
            ),
            (
                "program.timeout_secs",
                self.program.timeout_secs.to_string(),
                self.source("program"),
            ),
            (
                "program.output_max_bytes",
                self.program.output_max_bytes.to_string(),
                self.source("program"),
            ),
            (
                "recovery.enabled",
                self.recovery.enabled.to_string(),
                self.source("recovery"),
            ),
            (
                "recovery.max_age_days",
                self.recovery.max_age_days.to_string(),
                self.source("recovery"),
            ),
            (
                "recovery.max_total_bytes",
                self.recovery.max_total_bytes.to_string(),
                self.source("recovery"),
            ),
            (
                "recovery.max_file_bytes",
                self.recovery.max_file_bytes.to_string(),
                self.source("recovery"),
            ),
            (
                "cache_enabled",
                self.cache_enabled.to_string(),
                self.source("cache_enabled"),
            ),
            (
                "cache_ttl_secs",
                self.cache_ttl_secs.to_string(),
                self.source("cache_ttl_secs"),
            ),
            (
                "aliases",
                format!("{} configured", self.aliases.len()),
                self.source("aliases"),
            ),
        ]
    }
}

pub fn load(
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Result<Config, String> {
    let paths = dirs::resolve()?;
    let mut config = Config::defaults(paths.clone());
    match std::fs::read_to_string(&paths.config_file) {
        Ok(text) => {
            let file: FileConfig = serde_yaml_ng::from_str(&text)
                .map_err(|e| format!("invalid config {}: {}", paths.config_file.display(), e))?;
            apply_file(&mut config, file);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(format!(
                "cannot read config {}: {}",
                paths.config_file.display(),
                e
            ))
        }
    }
    apply_provider_environment(&mut config)?;
    if let Some(provider) = provider_override {
        config.provider = ProviderId::parse(provider)?;
        config.sources.insert("provider", "--provider");
    }
    apply_model_environment(&mut config)?;
    if let Some(model) = model_override {
        config.model = model.to_string();
        config.sources.insert("model", "--model");
    }
    validate(&config)?;
    Ok(config)
}

fn apply_provider_environment(config: &mut Config) -> Result<(), String> {
    if let Some(value) = nonempty_env("UHM_PROVIDER")? {
        config.provider = ProviderId::parse(&value)?;
        config.sources.insert("provider", "UHM_PROVIDER");
    }
    Ok(())
}

fn apply_model_environment(config: &mut Config) -> Result<(), String> {
    let uhm_model = nonempty_env("UHM_MODEL")?;
    let openai_model = nonempty_env("OPENAI_MODEL")?;
    apply_model_environment_values(config, uhm_model.as_deref(), openai_model.as_deref());
    Ok(())
}

fn apply_model_environment_values(
    config: &mut Config,
    uhm_model: Option<&str>,
    openai_model: Option<&str>,
) {
    if let Some(value) = uhm_model {
        config.model = value.into();
        config.sources.insert("model", "UHM_MODEL");
    } else if config.provider == ProviderId::Openai {
        if let Some(value) = openai_model {
            config.model = value.into();
            config.sources.insert("model", "OPENAI_MODEL");
        }
    }
}

fn nonempty_env(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(e) => Err(format!("${} is not valid UTF-8: {}", name, e)),
    }
}

macro_rules! apply {
    ($c:ident, $f:ident, $field:ident) => {
        if let Some(v) = $f.$field {
            $c.$field = v;
            $c.sources.insert(stringify!($field), "config.yaml");
        }
    };
}
fn apply_file(config: &mut Config, file: FileConfig) {
    apply!(config, file, provider);
    apply!(config, file, model);
    apply!(config, file, selection);
    apply!(config, file, max_completion_tokens);
    apply!(config, file, reasoning_effort);
    apply!(config, file, stream);
    apply!(config, file, shell);
    apply!(config, file, context_mode);
    apply!(config, file, context_timeout_ms);
    apply!(config, file, stdin_max_bytes);
    apply!(config, file, stdin_first_byte_timeout_ms);
    apply!(config, file, request_max_bytes);
    apply!(config, file, response_max_bytes);
    apply!(config, file, history);
    apply!(config, file, execution);
    apply!(config, file, telemetry);
    apply!(config, file, program);
    apply!(config, file, recovery);
    apply!(config, file, shell_context);
    apply!(config, file, cache_enabled);
    apply!(config, file, cache_ttl_secs);
    if let Some(aliases) = file.aliases {
        config.aliases = aliases.into_iter().collect();
        config.sources.insert("aliases", "config.yaml");
    }
}

fn validate(c: &Config) -> Result<(), String> {
    if c.model.trim().is_empty() {
        return Err("config model must not be empty".into());
    }
    if let Some(alternate) = &c.selection.alternate {
        if alternate.model.trim().is_empty() {
            return Err("config selection.alternate.model must not be empty".into());
        }
        if alternate.provider == c.provider && alternate.model == c.model {
            return Err("config selection alternate must differ from the primary".into());
        }
    }
    if c.selection.alternate.is_none() && !c.selection.fallback_on.is_empty() {
        return Err("config fallback_on requires selection.alternate".into());
    }
    if c.selection.fallback_on.iter().any(|kind| {
        !matches!(
            kind,
            ProviderErrorKind::RateLimited
                | ProviderErrorKind::Transient
                | ProviderErrorKind::Timeout
                | ProviderErrorKind::Incomplete
                | ProviderErrorKind::Malformed
        )
    }) {
        return Err("config fallback_on contains a disallowed trigger".into());
    }
    if !(1..=128_000).contains(&c.max_completion_tokens) {
        return Err("config max_completion_tokens must be between 1 and 128000".into());
    }
    if !matches!(
        c.reasoning_effort.as_str(),
        "none" | "minimal" | "low" | "medium" | "high" | "xhigh"
    ) {
        return Err("config reasoning_effort has an unsupported value".into());
    }
    if !(50..=5_000).contains(&c.context_timeout_ms) {
        return Err("config context_timeout_ms must be between 50 and 5000".into());
    }
    if !(50..=60_000).contains(&c.stdin_first_byte_timeout_ms) {
        return Err("config stdin_first_byte_timeout_ms must be between 50 and 60000".into());
    }
    if !matches!(c.context_mode.as_str(), "minimal" | "standard" | "full") {
        return Err("config context_mode must be minimal, standard, or full".into());
    }
    if !(1..=100_000).contains(&c.history.max_records)
        || c.history.max_age_days == 0
        || c.history.max_bytes == 0
        || c.history.artifact_max_bytes == 0
    {
        return Err("config history bounds must be positive".into());
    }
    if c.stdin_max_bytes == 0
        || c.request_max_bytes == 0
        || c.response_max_bytes == 0
        || c.execution.timeout_secs == 0
        || c.execution.diagnostic_bytes == 0
    {
        return Err("configured byte and time limits must be positive".into());
    }
    if c.program.source_max_bytes == 0
        || c.program.input_max_paths == 0
        || c.program.output_max_paths == 0
        || c.program.workspace_max_bytes == 0
        || c.program.timeout_secs == 0
        || c.program.cpu_secs == 0
        || c.program.address_space_bytes == 0
        || c.program.open_files < 16
        || c.program.child_processes == 0
        || c.program.output_max_bytes == 0
        || c.program.diagnostic_bytes == 0
    {
        return Err(
            "configured program limits must be positive and open_files must be at least 16".into(),
        );
    }
    if c.program.source_max_bytes > 64 * 1024
        || c.program.input_max_paths > 64
        || c.program.output_max_paths > 16
    {
        return Err("configured program manifest limits exceed the supported schema".into());
    }
    if c.recovery.max_age_days == 0
        || c.recovery.max_age_days > 3_650
        || c.recovery.max_total_bytes == 0
        || c.recovery.max_total_bytes > 1_099_511_627_776
        || c.recovery.max_file_bytes == 0
        || c.recovery.max_file_bytes > 1_073_741_824
        || c.recovery.max_file_bytes > c.recovery.max_total_bytes
        || !(1..=10_000).contains(&c.recovery.scan_limit)
        || !(1..=1_000).contains(&c.recovery.prune_batch)
    {
        return Err("config recovery limits are invalid or unbounded".into());
    }
    let shell = std::path::Path::new(&c.shell)
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or(&c.shell);
    if !matches!(
        shell,
        "auto" | "sh" | "bash" | "zsh" | "fish" | "pwsh" | "powershell"
    ) {
        return Err("config shell has an unsupported value".into());
    }
    for name in &c.execution.deny_env {
        if name.is_empty() || name.contains('=') {
            return Err("execution.deny_env entries must be environment names".into());
        }
    }
    for (name, command) in &c.aliases {
        if name.trim().is_empty() || command.trim().is_empty() {
            return Err("config aliases may not contain empty names or commands".into());
        }
    }
    if c.cache_ttl_secs == 0 {
        return Err("config cache_ttl_secs must be greater than zero".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn paths() -> Paths {
        Paths {
            config_file: "/tmp/c".into(),
            data_dir: "/tmp/d".into(),
            cache_dir: "/tmp/x".into(),
        }
    }
    #[test]
    fn defaults_are_bounded_and_private_by_policy() {
        let c = Config::defaults(paths());
        assert_eq!(c.provider, ProviderId::Openai);
        assert_eq!(c.selection.mode, SelectionMode::Fixed);
        assert_eq!(c.context_mode, "standard");
        assert!(c.history.enabled);
        assert!(c.telemetry.enabled);
        assert!(!c.shell_context.last_history_entry);
        assert!(!c.recovery.enabled);
    }
    #[test]
    fn stdin_first_byte_deadline_is_bounded_and_surfaced() {
        let c = Config::defaults(paths());
        assert_eq!(c.stdin_first_byte_timeout_ms, 1_000);
        assert!(c
            .show_lines()
            .iter()
            .any(|(key, value, _)| *key == "stdin_first_byte_timeout_ms" && value == "1000"));
        validate(&c).unwrap();
        let mut unbounded = Config::defaults(paths());
        unbounded.stdin_first_byte_timeout_ms = 0;
        assert!(validate(&unbounded).is_err());
        let mut excessive = Config::defaults(paths());
        excessive.stdin_first_byte_timeout_ms = 120_000;
        assert!(validate(&excessive).is_err());
    }
    #[test]
    fn rejects_removed_provider_base_url() {
        assert!(serde_yaml_ng::from_str::<FileConfig>("base_url: http://example.test").is_err());
        assert!(serde_yaml_ng::from_str::<FileConfig>("api_key: secret").is_err());
    }

    #[test]
    fn provider_configuration_is_strict_and_does_not_infer_from_model() {
        let file: FileConfig = serde_yaml_ng::from_str(
            "provider: cerebras\nmodel: gpt-5.6-terra\nselection:\n  mode: fixed\n  alternate: null\n  fallback_on: []\n",
        )
        .unwrap();
        let mut config = Config::defaults(paths());
        apply_file(&mut config, file);
        validate(&config).unwrap();
        assert_eq!(config.provider, ProviderId::Cerebras);
        assert_eq!(config.model, "gpt-5.6-terra");

        let invalid: FileConfig = serde_yaml_ng::from_str(
            "selection:\n  mode: fixed\n  alternate: null\n  fallback_on: [auth]\n",
        )
        .unwrap();
        let mut config = Config::defaults(paths());
        apply_file(&mut config, invalid);
        assert!(validate(&config).is_err());
    }

    #[test]
    fn provider_scopes_legacy_alias_and_uhm_model_wins() {
        let mut openai = Config::defaults(paths());
        apply_model_environment_values(&mut openai, None, Some("legacy"));
        assert_eq!(openai.model, "legacy");
        assert_eq!(openai.source("model"), "OPENAI_MODEL");

        let mut cerebras = Config::defaults(paths());
        cerebras.provider = ProviderId::Cerebras;
        apply_model_environment_values(&mut cerebras, None, Some("must-not-cross"));
        assert_eq!(cerebras.model, "gpt-5.6-terra");
        apply_model_environment_values(&mut cerebras, Some("explicit-env"), Some("legacy"));
        assert_eq!(cerebras.model, "explicit-env");
        assert_eq!(cerebras.source("model"), "UHM_MODEL");
    }
}
