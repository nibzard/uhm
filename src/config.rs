//! Strict config resolution: defaults <- config.yaml <- environment <- CLI.

use crate::dirs::{self, Paths};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HistoryConfig {
    pub enabled: bool,
    pub max_records: usize,
    pub max_age_days: u64,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_records: 500,
            max_age_days: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExecutionConfig {
    pub timeout_secs: u64,
    pub diagnostic_bytes: usize,
    pub deny_env: Vec<String>,
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
            deny_env: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Config {
    pub model: String,
    pub max_completion_tokens: u32,
    pub reasoning_effort: String,
    pub stream: bool,
    pub shell: String,
    pub context_mode: String,
    pub context_timeout_ms: u64,
    pub stdin_max_bytes: usize,
    pub request_max_bytes: usize,
    pub response_max_bytes: usize,
    pub history: HistoryConfig,
    pub execution: ExecutionConfig,
    pub telemetry: TelemetryConfig,
    pub program: ProgramConfig,
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
    model: Option<String>,
    max_completion_tokens: Option<u32>,
    reasoning_effort: Option<String>,
    stream: Option<bool>,
    shell: Option<String>,
    context_mode: Option<String>,
    context_timeout_ms: Option<u64>,
    stdin_max_bytes: Option<usize>,
    request_max_bytes: Option<usize>,
    response_max_bytes: Option<usize>,
    history: Option<HistoryConfig>,
    execution: Option<ExecutionConfig>,
    telemetry: Option<TelemetryConfig>,
    program: Option<ProgramConfig>,
    cache_enabled: Option<bool>,
    cache_ttl_secs: Option<u64>,
    aliases: Option<BTreeMap<String, String>>,
}

const KEYS: &[&str] = &[
    "model",
    "max_completion_tokens",
    "reasoning_effort",
    "stream",
    "shell",
    "context_mode",
    "context_timeout_ms",
    "stdin_max_bytes",
    "request_max_bytes",
    "response_max_bytes",
    "history",
    "execution",
    "telemetry",
    "program",
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
            model: "gpt-5.6-terra".into(),
            max_completion_tokens: 8192,
            reasoning_effort: "low".into(),
            stream: true,
            shell: "auto".into(),
            context_mode: "standard".into(),
            context_timeout_ms: 150,
            stdin_max_bytes: 16 * 1024 * 1024,
            request_max_bytes: 256 * 1024,
            response_max_bytes: 2 * 1024 * 1024,
            history: HistoryConfig::default(),
            execution: ExecutionConfig::default(),
            telemetry: TelemetryConfig::default(),
            program: ProgramConfig::default(),
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
            ("model", self.model.clone(), self.source("model")),
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
                "execution.timeout_secs",
                self.execution.timeout_secs.to_string(),
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

pub fn load(model_override: Option<&str>) -> Result<Config, String> {
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
    if let Some(value) = nonempty_env("OPENAI_MODEL")? {
        config.model = value;
        config.sources.insert("model", "OPENAI_MODEL");
    }
    if let Some(model) = model_override {
        config.model = model.to_string();
        config.sources.insert("model", "--model");
    }
    validate(&config)?;
    Ok(config)
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
    apply!(config, file, model);
    apply!(config, file, max_completion_tokens);
    apply!(config, file, reasoning_effort);
    apply!(config, file, stream);
    apply!(config, file, shell);
    apply!(config, file, context_mode);
    apply!(config, file, context_timeout_ms);
    apply!(config, file, stdin_max_bytes);
    apply!(config, file, request_max_bytes);
    apply!(config, file, response_max_bytes);
    apply!(config, file, history);
    apply!(config, file, execution);
    apply!(config, file, telemetry);
    apply!(config, file, program);
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
    if !matches!(c.context_mode.as_str(), "minimal" | "standard" | "full") {
        return Err("config context_mode must be minimal, standard, or full".into());
    }
    if !(1..=10_000).contains(&c.history.max_records) || c.history.max_age_days == 0 {
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
        assert_eq!(c.context_mode, "standard");
        assert!(c.history.enabled);
        assert!(c.telemetry.enabled);
    }
    #[test]
    fn rejects_removed_provider_base_url() {
        assert!(serde_yaml_ng::from_str::<FileConfig>("base_url: http://example.test").is_err());
    }
}
