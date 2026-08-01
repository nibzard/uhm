//! Bounded result-first job: at most two proposals and, for explicit repair, two executions.

use crate::action::{Effect, ProposalMetadata, ProposedAction, StdinMode};
use crate::args::Args;
use crate::config::Config;
use crate::outcome::Outcome;
use crate::render::{ansi, card, spinner};
use crate::{api, cache, context, history, outcome, parent_shell, prompt, safety, shell, tty};
use serde_json::{json, Value};
use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Replacement {
    Clarification,
    Revision,
    Repair,
    Edit,
}

#[derive(Default)]
struct Budget {
    model_calls: u8,
    executions: u8,
    replacement: Option<Replacement>,
}

impl Budget {
    fn initial_model(&mut self) {
        self.model_calls = 1;
    }
    fn can_replace(&self) -> bool {
        self.replacement.is_none() && self.model_calls < 2
    }
    fn replace_with_model(&mut self, kind: Replacement) -> bool {
        if !self.can_replace() {
            return false;
        }
        self.replacement = Some(kind);
        self.model_calls += 1;
        true
    }
    fn replace_with_edit(&mut self) -> bool {
        if self.replacement.is_some() {
            return false;
        }
        self.replacement = Some(Replacement::Edit);
        true
    }
    fn execute(&mut self) -> bool {
        let allowed = self.executions == 0
            || (self.executions == 1
                && matches!(
                    self.replacement,
                    Some(Replacement::Repair | Replacement::Edit)
                ));
        if allowed {
            self.executions += 1;
        }
        allowed
    }
    fn second_used(&self) -> bool {
        self.replacement.is_some()
    }
}

pub fn handle(
    args: &Args,
    config: &Config,
    api_config: &api::ApiConfig,
    request: &str,
    route: &str,
    stdin: &crate::input::Spool,
) -> i32 {
    let started = Instant::now();
    let shell_name = match target_shell(config, args.shell.as_deref()) {
        Ok(v) => v,
        Err(e) => return app_error(args, outcome::USAGE, "usage_error", &e),
    };
    let mode_text = args.context.as_deref().unwrap_or(&config.context_mode);
    let mode = match context::Mode::parse(mode_text) {
        Ok(v) => v,
        Err(e) => return app_error(args, outcome::USAGE, "usage_error", &e),
    };
    // Aliases are resolved before automatic context probes and do not leave the device.
    let alias = config
        .aliases
        .iter()
        .find(|(name, _)| name.trim() == request.trim())
        .map(|(_, command)| ProposedAction::Shell {
            command: command.clone(),
            metadata: ProposalMetadata {
                summary: "Expanded from a local alias.".into(),
                ..Default::default()
            },
            stdin_mode: StdinMode::None,
        });
    let snapshot = if alias.is_some() {
        context::gather(
            context::Mode::Minimal,
            &shell_name,
            config.context_timeout_ms,
        )
    } else {
        context::gather(mode, &shell_name, config.context_timeout_ms)
    };
    let mut budget = Budget::default();
    let run_id = history::run_id();
    let mut action = match alias {
        Some(v) => v,
        None => match propose(
            args,
            config,
            api_config,
            route,
            request,
            &snapshot,
            stdin.model_value(),
            None,
            &shell_name,
        ) {
            Ok((v, _)) => {
                budget.initial_model();
                v
            }
            Err(e) => return app_error(args, outcome::MODEL, "model_error", &e),
        },
    };
    loop {
        if let ProposedAction::Shell {
            command, metadata, ..
        } = &action
        {
            if parent_shell::required(command) {
                action = ProposedAction::ParentShell {
                    command: command.clone(),
                    metadata: metadata.clone(),
                };
            }
        }
        match action {
            ProposedAction::Answer { text } => {
                if route == "run" {
                    return app_error(
                        args,
                        outcome::NOT_EXECUTED,
                        "not_a_command",
                        "run mode requires an executable action, but the model returned prose",
                    );
                }
                if args.json {
                    println!(
                        "{}",
                        Outcome {
                            namespace: "uhm",
                            outcome: "answer",
                            exit_code: 0,
                            executed: false,
                            command: None,
                            message: Some(&text)
                        }
                        .json()
                    )
                } else {
                    println!("{}", ansi::sanitize_untrusted(&text));
                }
                receipt(
                    config,
                    &run_id,
                    route,
                    mode,
                    route,
                    "answer",
                    false,
                    0,
                    None,
                    started.elapsed(),
                    budget.second_used(),
                    &[],
                    &[],
                );
                return 0;
            }
            ProposedAction::Clarification { question } => {
                if !budget.can_replace() || !tty_available() {
                    return clarification(args, &question);
                }
                eprintln!("{}", ansi::sanitize_untrusted(&question));
                eprint!("uhm› ");
                let _ = std::io::stderr().flush();
                let Some(answer) = tty::read_line_cooked() else {
                    return outcome::CLARIFICATION;
                };
                let _ = budget.replace_with_model(Replacement::Clarification);
                action = match propose(
                    args,
                    config,
                    api_config,
                    route,
                    request,
                    &snapshot,
                    stdin.model_value(),
                    Some(json!({"kind":"clarification","answer":answer})),
                    &shell_name,
                ) {
                    Ok((v, _)) => v,
                    Err(e) => return app_error(args, outcome::MODEL, "model_error", &e),
                };
                continue;
            }
            ProposedAction::ParentShell { command, metadata } => {
                if args.dry_run {
                    return dry_run(args, &command);
                }
                if !args.json {
                    card::preview(
                        &command,
                        &metadata.summary,
                        safety::Tier::Low,
                        &metadata.effects,
                        &[],
                    );
                    eprintln!("This must run in your current shell; uhm generated it but did not apply it. Copy or evaluate the exact command above.");
                }
                receipt(
                    config,
                    &run_id,
                    route,
                    mode,
                    "require_parent_shell",
                    "not_applied",
                    false,
                    0,
                    None,
                    started.elapsed(),
                    budget.second_used(),
                    &metadata.effects,
                    &[],
                );
                return not_executed(
                    args,
                    &command,
                    "parent shell state cannot persist from the uhm child process",
                );
            }
            ProposedAction::Shell {
                mut command,
                metadata,
                stdin_mode,
            } => {
                let missing = context::missing_requirements(&metadata.requirements);
                if !missing.is_empty() {
                    let msg = format!("required executable(s) unavailable: {}", missing.join(", "));
                    if budget.can_replace()
                        && tty_available()
                        && ask("Ask for an available alternative? [y/N] ")
                    {
                        let _ = budget.replace_with_model(Replacement::Revision);
                        action = match propose(
                            args,
                            config,
                            api_config,
                            route,
                            request,
                            &snapshot,
                            stdin.model_value(),
                            Some(json!({"kind":"revision","prior_action":command,"feedback":msg})),
                            &shell_name,
                        ) {
                            Ok((v, _)) => v,
                            Err(e) => return app_error(args, outcome::MODEL, "model_error", &e),
                        };
                        continue;
                    }
                    return app_error(args, outcome::UNAVAILABLE, "requirement_unavailable", &msg);
                }
                let classification = safety::classify(&command);
                let effects = merged_effects(&classification.effects, &metadata.effects);
                let consequential = classification.tier.severity()
                    >= safety::Tier::Destructive.severity()
                    || effects.iter().any(Effect::requires_advisory_pause);
                let review = args.review || consequential;
                if args.dry_run {
                    return dry_run(args, &command);
                }
                if review && !args.json {
                    card::preview(
                        &command,
                        &metadata.summary,
                        classification.tier,
                        &effects,
                        &classification.reasons,
                    );
                    eprintln!(
                        "Shell: {}\nCwd: {}",
                        shell_name,
                        snapshot.machine["working_directory"]
                            .as_str()
                            .unwrap_or("(not disclosed)")
                    );
                    for a in &metadata.assumptions {
                        eprintln!("Assumption: {}", ansi::sanitize_untrusted_inline(a));
                    }
                }
                if args.json && review && !args.force {
                    return not_executed(
                        args,
                        &command,
                        "review is required; automation must use --force or --dry-run",
                    );
                }
                if review && !args.force {
                    if !tty_available() {
                        return not_executed(
                            args,
                            &command,
                            "review or confirmation is required, but no terminal is available; use --force or --dry-run",
                        );
                    };
                    eprint!("Run, revise, edit, copy, cancel? [R/v/e/c/q] ");
                    let _ = std::io::stderr().flush();
                    match tty::read_line_cooked()
                        .unwrap_or_default()
                        .to_lowercase()
                        .as_str()
                    {
                        "" | "r" | "run" => {}
                        "v" | "revise" if budget.can_replace() => {
                            eprint!("Feedback: ");
                            let _ = std::io::stderr().flush();
                            let feedback = tty::read_line_cooked().unwrap_or_default();
                            let _ = budget.replace_with_model(Replacement::Revision);
                            action = match propose(
                                args,
                                config,
                                api_config,
                                route,
                                request,
                                &snapshot,
                                stdin.model_value(),
                                Some(
                                    json!({"kind":"revision","prior_action":command,"feedback":feedback}),
                                ),
                                &shell_name,
                            ) {
                                Ok((v, _)) => v,
                                Err(e) => {
                                    return app_error(args, outcome::MODEL, "model_error", &e)
                                }
                            };
                            continue;
                        }
                        "e" | "edit" if budget.replacement.is_none() => match edit(&command) {
                            Ok(v) => {
                                command = v;
                                let _ = budget.replace_with_edit();
                            }
                            Err(e) => {
                                return app_error(args, outcome::NOT_EXECUTED, "edit_error", &e)
                            }
                        },
                        "c" | "copy" => {
                            let _ = write_command(std::io::stdout(), &command);
                            return outcome::NOT_EXECUTED;
                        }
                        _ => return not_executed(args, &command, "cancelled by user"),
                    }
                } else if consequential && args.force && !args.json {
                    eprintln!(
                        "{}",
                        ansi::yellow("Proceeding because --force was supplied.")
                    );
                }
                let child_stdin = (stdin_mode == StdinMode::Original).then(|| stdin.bytes());
                if !budget.execute() {
                    return app_error(
                        args,
                        outcome::NOT_EXECUTED,
                        "budget_exhausted",
                        "execution budget exhausted",
                    );
                }
                let result = match shell::execute(shell::Request {
                    shell: &shell_name,
                    command: &command,
                    stdin: child_stdin,
                    timeout: Duration::from_secs(config.execution.timeout_secs),
                    diagnostic_bytes: config.execution.diagnostic_bytes,
                    deny_env: &config.execution.deny_env,
                }) {
                    Ok(v) => v,
                    Err(e) => return app_error(args, outcome::NOT_EXECUTED, "spawn_error", &e),
                };
                let detected = classification.effects.clone();
                if result.code != 0
                    && budget.can_replace()
                    && budget.executions < 2
                    && tty_available()
                {
                    let available = result
                        .stderr_tail
                        .as_ref()
                        .map(|v| ansi::sanitize_untrusted(&String::from_utf8_lossy(v)))
                        .unwrap_or_else(|| {
                            "diagnostics unavailable because stderr was attached to the terminal"
                                .into()
                        });
                    eprintln!("uhm: command exited {} ({})", result.code, available);
                    eprint!("Repair, edit, or stop? [r/e/N] ");
                    let _ = std::io::stderr().flush();
                    match tty::read_line_cooked()
                        .unwrap_or_default()
                        .to_lowercase()
                        .as_str()
                    {
                        "r" | "repair" | "y" | "yes" => {
                            let _ = budget.replace_with_model(Replacement::Repair);
                            action = match propose(
                                args,
                                config,
                                api_config,
                                route,
                                request,
                                &snapshot,
                                stdin.model_value(),
                                Some(
                                    json!({"kind":"repair","prior_action":command,"exit_code":result.code,"signal":result.signal,"stderr":available}),
                                ),
                                &shell_name,
                            ) {
                                Ok((v, _)) => v,
                                Err(e) => {
                                    return app_error(args, outcome::MODEL, "model_error", &e)
                                }
                            };
                            continue;
                        }
                        "e" | "edit" => match edit(&command) {
                            Ok(replacement) => {
                                let _ = budget.replace_with_edit();
                                action = ProposedAction::Shell {
                                    command: replacement,
                                    metadata,
                                    stdin_mode,
                                };
                                continue;
                            }
                            Err(e) => {
                                return app_error(args, outcome::NOT_EXECUTED, "edit_error", &e)
                            }
                        },
                        _ => {}
                    }
                }
                let decision = if result.timed_out {
                    "timed_out"
                } else if result.code == 0 {
                    "completed"
                } else {
                    "failed"
                };
                receipt(
                    config,
                    &run_id,
                    route,
                    mode,
                    "run_shell",
                    decision,
                    true,
                    result.code,
                    result.signal,
                    started.elapsed(),
                    budget.second_used(),
                    &metadata.effects,
                    &detected,
                );
                if args.verbose {
                    eprintln!(
                        "uhm: execution {} ms; stdout diagnostics {}",
                        result.duration.as_millis(),
                        result
                            .stdout_tail
                            .as_ref()
                            .map_or("unavailable".into(), |v| format!("{} bytes", v.len()))
                    );
                }
                if args.json {
                    let effect_message = (!effects.is_empty()).then(|| {
                        format!(
                            "detected effects: {}",
                            effects
                                .iter()
                                .map(Effect::label)
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    });
                    eprintln!(
                        "{}",
                        Outcome {
                            namespace: "uhm.child",
                            outcome: "executed",
                            exit_code: result.code,
                            executed: true,
                            command: None,
                            message: effect_message.as_deref()
                        }
                        .json()
                    );
                }
                return result.code;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn propose(
    args: &Args,
    config: &Config,
    api_config: &api::ApiConfig,
    route: &str,
    request: &str,
    snapshot: &context::Snapshot,
    stdin: Value,
    follow_up: Option<Value>,
    shell: &str,
) -> Result<(ProposedAction, bool), String> {
    ensure_disclosure(Some(context::DISCLOSURE_MARKER))?;
    let context_value = serde_json::to_value(snapshot).map_err(|e| e.to_string())?;
    let context_hash = blake3::hash(serde_json::to_string(&context_value).unwrap().as_bytes())
        .to_hex()
        .to_string();
    let key = cache::key_hash(
        &api_config.model,
        shell,
        api_config.max_tokens,
        &api_config.reasoning_effort,
        &snapshot.mode,
        &context_hash,
        request,
    );
    if follow_up.is_none() && !args.fresh {
        if let Some(raw) = cache::get(
            &config.paths.cache_dir,
            config.cache_enabled,
            config.cache_ttl_secs,
            &key,
        ) {
            if let Ok(action) = api::parse_response(&raw) {
                if args.verbose {
                    eprintln!("uhm: cache hit {}", &key[..8]);
                }
                return Ok((action, true));
            }
        }
    }
    let cacheable = follow_up.is_none();
    let input = prompt::proposal_input(route, request, context_value, stdin, follow_up);
    let mut progress = spinner::Spinner::start("thinking");
    let response = api::request_action(
        api_config,
        &input,
        config.stream && !args.no_stream && !args.json,
    );
    progress.stop();
    let (action, raw) = response?;
    if cacheable {
        if let Err(e) = cache::put(&config.paths.cache_dir, config.cache_enabled, &key, &raw) {
            eprintln!("uhm: {}", e)
        }
    }
    Ok((action, false))
}
pub fn ensure_disclosure(marker: Option<&str>) -> Result<(), String> {
    if marker == Some(context::DISCLOSURE_MARKER) {
        Ok(())
    } else {
        Err("context disclosure was not rendered; outbound request blocked".into())
    }
}
fn edit(command: &str) -> Result<String, String> {
    let mut file = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    file.write_all(command.as_bytes())
        .map_err(|e| e.to_string())?;
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into());
    let ok = std::process::Command::new(editor)
        .arg(file.path())
        .status()
        .map_err(|e| e.to_string())?
        .success();
    if !ok {
        return Err("editor exited unsuccessfully".into());
    }
    std::fs::read_to_string(file.path())
        .map(|s| s.trim_end_matches(['\r', '\n']).to_string())
        .map_err(|e| e.to_string())
}
fn ask(prompt: &str) -> bool {
    eprint!("{}", prompt);
    let _ = std::io::stderr().flush();
    tty::read_line_cooked().is_some_and(|v| matches!(v.as_str(), "y" | "yes"))
}
fn tty_available() -> bool {
    std::io::stderr().is_terminal()
}
#[allow(clippy::too_many_arguments)]
fn receipt(
    config: &Config,
    id: &str,
    mode: &str,
    context_mode: context::Mode,
    route: &str,
    decision: &str,
    attempted: bool,
    exit: i32,
    signal: Option<i32>,
    duration: Duration,
    second: bool,
    declared: &[Effect],
    detected: &[Effect],
) {
    if !config.history.enabled {
        return;
    }
    let entry = history::Receipt {
        schema_version: 1,
        run_id: id.into(),
        timestamp: history::now_secs(),
        app_version: env!("CARGO_PKG_VERSION")
            .split('.')
            .take(2)
            .collect::<Vec<_>>()
            .join("."),
        mode: mode.into(),
        context_mode: context_mode.as_str().into(),
        route: route.into(),
        prompt_schema_version: prompt::PROMPT_VERSION,
        declared_effects: declared.iter().map(|e| e.label().into()).collect(),
        detected_effects: detected.iter().map(|e| e.label().into()).collect(),
        decision: decision.into(),
        execution_attempted: attempted,
        exit_category: if !attempted {
            "not_attempted"
        } else if signal.is_some() {
            "signal"
        } else if exit == 0 {
            "success"
        } else {
            "failure"
        }
        .into(),
        signal,
        latency_bucket: if duration.as_secs() < 1 {
            "lt_1s"
        } else if duration.as_secs() < 5 {
            "1_5s"
        } else {
            "gte_5s"
        }
        .into(),
        cache_state: "unknown".into(),
        second_turn_used: second,
    };
    if let Err(e) = history::append(
        &config.paths.data_dir,
        &entry,
        config.history.max_records,
        config.history.max_age_days,
    ) {
        eprintln!("uhm: history: {}", e)
    }
}
fn merged_effects(a: &[Effect], b: &[Effect]) -> Vec<Effect> {
    let mut out = a.to_vec();
    for e in b {
        if !out.contains(e) {
            out.push(e.clone())
        }
    }
    out
}
fn dry_run(args: &Args, command: &str) -> i32 {
    if args.json {
        println!(
            "{}",
            Outcome {
                namespace: "uhm",
                outcome: "dry_run",
                exit_code: 0,
                executed: false,
                command: Some(command),
                message: None
            }
            .json()
        )
    } else {
        let _ = write_command(std::io::stdout(), command);
    }
    0
}
fn write_command(mut out: impl Write, command: &str) -> std::io::Result<()> {
    out.write_all(command.as_bytes())?;
    out.flush()
}
fn clarification(args: &Args, q: &str) -> i32 {
    if args.json {
        println!(
            "{}",
            Outcome {
                namespace: "uhm",
                outcome: "clarification_required",
                exit_code: outcome::CLARIFICATION,
                executed: false,
                command: None,
                message: Some(q)
            }
            .json()
        )
    } else {
        println!("{}", ansi::sanitize_untrusted(q))
    }
    outcome::CLARIFICATION
}
fn not_executed(args: &Args, command: &str, message: &str) -> i32 {
    if args.json {
        println!(
            "{}",
            Outcome {
                namespace: "uhm",
                outcome: "not_executed",
                exit_code: outcome::NOT_EXECUTED,
                executed: false,
                command: Some(command),
                message: Some(message)
            }
            .json()
        )
    } else {
        eprintln!("uhm: {}", message)
    }
    outcome::NOT_EXECUTED
}
fn app_error(args: &Args, code: i32, name: &str, message: &str) -> i32 {
    if args.json {
        println!(
            "{}",
            Outcome {
                namespace: "uhm",
                outcome: name,
                exit_code: code,
                executed: false,
                command: None,
                message: Some(message)
            }
            .json()
        )
    } else {
        eprintln!("uhm: {}", message)
    }
    code
}
fn target_shell(config: &Config, over: Option<&str>) -> Result<String, String> {
    normalize_shell(
        over.unwrap_or(&config.shell),
        &std::env::var("SHELL").unwrap_or_default(),
    )
}
fn normalize_shell(requested: &str, detected: &str) -> Result<String, String> {
    let value = if requested == "auto" || requested.is_empty() {
        if detected.is_empty() {
            "/bin/sh"
        } else {
            detected
        }
    } else {
        requested
    };
    let name = std::path::Path::new(value)
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or(value);
    if matches!(name, "sh" | "bash" | "zsh" | "fish" | "pwsh" | "powershell") {
        Ok(value.into())
    } else {
        Err(format!("unsupported shell '{}'", value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gate_blocks_missing_marker() {
        assert!(ensure_disclosure(None).is_err());
        assert!(ensure_disclosure(Some(context::DISCLOSURE_MARKER)).is_ok());
    }
    #[test]
    fn shell_normalization() {
        assert_eq!(normalize_shell("auto", "/bin/zsh").unwrap(), "/bin/zsh");
        assert!(normalize_shell("nu", "").is_err());
    }

    #[test]
    fn second_turn_consumers_are_mutually_exclusive() {
        for first in [
            Replacement::Clarification,
            Replacement::Revision,
            Replacement::Repair,
        ] {
            let mut budget = Budget::default();
            budget.initial_model();
            assert!(budget.replace_with_model(first));
            assert!(!budget.replace_with_model(Replacement::Revision));
            assert!(!budget.replace_with_edit());
            assert_eq!(budget.model_calls, 2);
        }
    }

    #[test]
    fn only_post_failure_replacement_allows_a_second_execution() {
        let mut normal = Budget::default();
        normal.initial_model();
        assert!(normal.execute());
        assert!(!normal.execute());

        let mut repaired = Budget::default();
        repaired.initial_model();
        assert!(repaired.execute());
        assert!(repaired.replace_with_model(Replacement::Repair));
        assert!(repaired.execute());
        assert!(!repaired.execute());

        let mut clarified = Budget::default();
        clarified.initial_model();
        assert!(clarified.replace_with_model(Replacement::Clarification));
        assert!(clarified.execute());
        assert!(!clarified.execute());
    }
}
