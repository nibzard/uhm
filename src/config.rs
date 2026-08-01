//! Strict config resolution: defaults <- config.yaml <- environment <- CLI.

use crate::dirs::{self, Paths};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct Config {
    pub model: String,
    pub base_url: String,
    pub max_completion_tokens: u32,
    pub reasoning_effort: String,
    pub stream: bool,
    pub shell: String,
    pub include_ls: bool,
    pub context_mode: String,
    pub include_history: bool,
    pub history_lines: usize,
    pub context_timeout_ms: u64,
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
    base_url: Option<String>,
    max_completion_tokens: Option<u32>,
    reasoning_effort: Option<String>,
    stream: Option<bool>,
    shell: Option<String>,
    include_ls: Option<bool>,
    context_mode: Option<String>,
    include_history: Option<bool>,
    history_lines: Option<usize>,
    context_timeout_ms: Option<u64>,
    cache_enabled: Option<bool>,
    cache_ttl_secs: Option<u64>,
    aliases: Option<BTreeMap<String, String>>,
}

impl Config {
    fn defaults(paths: Paths) -> Self {
        let mut sources = BTreeMap::new();
        for key in KEYS {
            sources.insert(*key, "default");
        }
        Self {
            model: "gpt-5.6-luna".into(),
            base_url: "https://api.openai.com/v1".into(),
            max_completion_tokens: 1024,
            reasoning_effort: "low".into(),
            stream: true,
            shell: "auto".into(),
            include_ls: true,
            context_mode: "full".into(),
            include_history: false,
            history_lines: 15,
            context_timeout_ms: 800,
            cache_enabled: true,
            cache_ttl_secs: 86_400,
            aliases: Vec::new(),
            paths,
            sources,
        }
    }

    pub fn source(&self, key: &str) -> &'static str {
        self.sources.get(key).copied().unwrap_or("unknown")
    }

    pub fn show_lines(&self) -> Vec<(&'static str, String, &'static str)> {
        vec![
            ("model", self.model.clone(), self.source("model")),
            ("base_url", self.base_url.clone(), self.source("base_url")),
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
                "context_timeout_ms",
                self.context_timeout_ms.to_string(),
                self.source("context_timeout_ms"),
            ),
            (
                "include_ls",
                self.include_ls.to_string(),
                self.source("include_ls"),
            ),
            (
                "context_mode",
                self.context_mode.clone(),
                self.source("context_mode"),
            ),
            (
                "include_history",
                self.include_history.to_string(),
                self.source("include_history"),
            ),
            (
                "history_lines",
                self.history_lines.to_string(),
                self.source("history_lines"),
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

const KEYS: &[&str] = &[
    "model",
    "base_url",
    "max_completion_tokens",
    "reasoning_effort",
    "stream",
    "shell",
    "include_ls",
    "context_mode",
    "include_history",
    "history_lines",
    "context_timeout_ms",
    "cache_enabled",
    "cache_ttl_secs",
    "aliases",
];

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

    if let Some(value) = nonempty_env("OPENAI_BASE_URL")? {
        config.base_url = value;
        config.sources.insert("base_url", "OPENAI_BASE_URL");
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
    ($config:ident, $file:ident, $field:ident) => {
        if let Some(value) = $file.$field {
            $config.$field = value;
            $config.sources.insert(stringify!($field), "config.yaml");
        }
    };
}

fn apply_file(config: &mut Config, file: FileConfig) {
    apply!(config, file, model);
    apply!(config, file, base_url);
    apply!(config, file, max_completion_tokens);
    apply!(config, file, reasoning_effort);
    apply!(config, file, stream);
    apply!(config, file, shell);
    apply!(config, file, include_ls);
    apply!(config, file, context_mode);
    apply!(config, file, include_history);
    apply!(config, file, history_lines);
    apply!(config, file, context_timeout_ms);
    apply!(config, file, cache_enabled);
    apply!(config, file, cache_ttl_secs);
    if let Some(aliases) = file.aliases {
        config.aliases = aliases.into_iter().collect();
        config.sources.insert("aliases", "config.yaml");
    }
}

fn validate(config: &Config) -> Result<(), String> {
    if config.model.trim().is_empty() {
        return Err("config model must not be empty".into());
    }
    if !(config.base_url.starts_with("https://") || config.base_url.starts_with("http://")) {
        return Err("config base_url must start with https:// or http://".into());
    }
    if !(1..=128_000).contains(&config.max_completion_tokens) {
        return Err("config max_completion_tokens must be between 1 and 128000".into());
    }
    if !matches!(
        config.reasoning_effort.as_str(),
        "none" | "minimal" | "low" | "medium" | "high" | "xhigh"
    ) {
        return Err("config reasoning_effort has an unsupported value".into());
    }
    if !(50..=60_000).contains(&config.context_timeout_ms) {
        return Err("config context_timeout_ms must be between 50 and 60000".into());
    }
    if !matches!(config.context_mode.as_str(), "full" | "request_only") {
        return Err("config context_mode must be full or request_only".into());
    }
    if !(1..=200).contains(&config.history_lines) {
        return Err("config history_lines must be between 1 and 200".into());
    }
    if config.cache_ttl_secs == 0 {
        return Err("config cache_ttl_secs must be greater than zero".into());
    }
    let shell = std::path::Path::new(&config.shell)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&config.shell);
    if !matches!(
        shell,
        "auto" | "sh" | "bash" | "zsh" | "fish" | "pwsh" | "powershell"
    ) {
        return Err("config shell has an unsupported value".into());
    }
    for (name, command) in &config.aliases {
        if name.trim().is_empty() || command.trim().is_empty() {
            return Err("config aliases may not contain empty names or commands".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_keys_are_rejected() {
        let err = serde_yaml_ng::from_str::<FileConfig>("model: x\nmodle: y\n").unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn invalid_types_are_rejected() {
        assert!(serde_yaml_ng::from_str::<FileConfig>("stream: perhaps\n").is_err());
    }
}
