//! Proposal -> local effect detection -> invocation policy -> execution.

use std::io::Write;

use crate::action::{Effect, ProposalMetadata, ProposedAction, ShellAction};
use crate::api;
use crate::args::Args;
use crate::config::Config;
use crate::context;
use crate::context::Provider as _;
use crate::history;
use crate::history::ReceiptWriter as _;
use crate::outcome::{self, Outcome};
use crate::prompt;
use crate::render::{ansi, card, spinner};
use crate::shell::Executor as _;
use crate::tty::Interaction as _;
use crate::{cache, safety, shell, tty};

pub fn handle(
    args: &Args,
    config: &Config,
    api_config: &api::ApiConfig,
    request: &str,
    require_command: bool,
) -> i32 {
    let shell_name = match target_shell(config, args.shell.as_deref()) {
        Ok(shell) => shell,
        Err(error) => return app_error(args, outcome::USAGE, "usage_error", &error),
    };
    let bundle = if config.context_mode == "full" {
        context::SystemProvider.gather(&shell_name, config.include_ls, config.context_timeout_ms)
    } else {
        context::Bundle::default()
    };
    let os = if bundle.os.is_empty() {
        std::env::consts::OS.to_string()
    } else {
        bundle.os.clone()
    };
    let context_text = bundle.render();
    let context_hash = blake3::hash(context_text.as_bytes()).to_hex().to_string();

    let alias = config
        .aliases
        .iter()
        .find(|(name, _)| name.trim() == request.trim())
        .map(|(_, command)| {
            ProposedAction::Shell(ShellAction {
                command: command.clone(),
                metadata: ProposalMetadata {
                    summary: "Expanded from a local alias.".into(),
                    ..ProposalMetadata::default()
                },
            })
        });

    let proposal = match alias {
        Some(action) => Ok(action),
        None => proposal(
            args,
            config,
            api_config,
            &os,
            &shell_name,
            &context_text,
            &context_hash,
            request,
        ),
    };
    let proposal = match proposal {
        Ok(action) => action,
        Err(error) => return app_error(args, outcome::MODEL, "model_error", &error),
    };

    match proposal {
        ProposedAction::Answer { text, .. } => {
            if require_command {
                app_error(
                    args,
                    outcome::NOT_EXECUTED,
                    "not_a_command",
                    "the request produced an answer, not a command",
                )
            } else {
                if args.json {
                    println!(
                        "{}",
                        Outcome {
                            namespace: "uhm",
                            outcome: "answer",
                            exit_code: 0,
                            executed: false,
                            command: None,
                            message: Some(&text),
                        }
                        .json()
                    );
                } else {
                    println!("{}", ansi::sanitize_untrusted(&text));
                }
                0
            }
        }
        ProposedAction::Clarification { question, .. } => {
            if args.json {
                println!(
                    "{}",
                    Outcome {
                        namespace: "uhm",
                        outcome: "clarification_required",
                        exit_code: outcome::CLARIFICATION,
                        executed: false,
                        command: None,
                        message: Some(&question),
                    }
                    .json()
                );
            } else {
                println!("{}", ansi::sanitize_untrusted(&question));
            }
            outcome::CLARIFICATION
        }
        ProposedAction::ParentShell(action) => {
            if args.dry_run {
                return dry_run(args, &action.command);
            }
            app_error(
                args,
                outcome::NOT_EXECUTED,
                "parent_shell_required",
                "this action must change the current shell; uhm cannot make cd/export/alias persist from a child process yet. Use --dry-run and apply the command in your shell.",
            )
        }
        ProposedAction::Shell(action) => {
            execute_shell(args, config, api_config, &shell_name, action)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn proposal(
    args: &Args,
    config: &Config,
    api_config: &api::ApiConfig,
    os: &str,
    shell_name: &str,
    context_text: &str,
    context_hash: &str,
    request: &str,
) -> Result<ProposedAction, String> {
    let key = cache::key_hash(
        &api_config.model,
        &api_config.base_url,
        shell_name,
        api_config.max_tokens,
        &api_config.reasoning_effort,
        &config.context_mode,
        context_hash,
        request,
    );
    if !args.fresh {
        if let Some(raw) = cache::get(
            &config.paths.cache_dir,
            config.cache_enabled,
            config.cache_ttl_secs,
            &key,
        ) {
            if args.verbose {
                eprintln!("uhm: cache hit {}", &key[..8]);
            }
            return api::parse_proposal(&raw);
        }
    }

    let input = prompt::proposal_input(
        os,
        shell_name,
        context_text,
        request,
        config.context_mode == "full",
    );
    let mut spinner = spinner::Spinner::start("thinking");
    let raw =
        api::request_envelope_raw(api_config, prompt::proposal_system(), &input, config.stream);
    spinner.stop();
    let raw = raw?;
    if let Err(error) = cache::put(&config.paths.cache_dir, config.cache_enabled, &key, &raw) {
        eprintln!("uhm: {}", error);
    }
    api::parse_proposal(&raw)
}

fn execute_shell(
    args: &Args,
    config: &Config,
    api_config: &api::ApiConfig,
    shell_name: &str,
    action: ShellAction,
) -> i32 {
    let ShellAction { command, metadata } = action;
    if args.dry_run {
        return dry_run(args, &command);
    }

    let classification = safety::classify(&command);
    let effects = merged_effects(&classification.effects, &metadata.effects);
    let consequential = classification.tier.severity() >= safety::Tier::Destructive.severity()
        || effects.iter().any(Effect::requires_advisory_pause);
    let needs_review = args.review || consequential;

    if needs_review && !args.json {
        card::preview(
            &command,
            &metadata.summary,
            classification.tier,
            &effects,
            &classification.reasons,
        );
    }
    if args.json && needs_review && !args.force {
        return not_executed(
            args,
            &command,
            "review or confirmation is required; automation must use --force or --dry-run",
        );
    }
    if consequential && args.force && !args.json {
        eprintln!(
            "{}",
            ansi::yellow("Proceeding because --force was supplied.")
        );
    }
    if needs_review && !args.force {
        if !tty::SystemInteraction.interactive() {
            return not_executed(
                args,
                &command,
                "review or confirmation is required, but no terminal is available; use --force to execute or --dry-run to print",
            );
        }
        card::confirmation_hint();
        let _ = std::io::stderr().flush();
        let accepted = tty::SystemInteraction.confirm();
        if !accepted {
            return not_executed(args, &command, "cancelled by user");
        }
    }

    let code = match shell::SystemExecutor.execute(shell_name, &command) {
        Ok(code) => code,
        Err(error) => return app_error(args, outcome::NOT_EXECUTED, "spawn_error", &error),
    };
    if config.include_history {
        let entry = history::Entry {
            ts: history::now_secs(),
            model: api_config.model.clone(),
            kind: "shell".into(),
            command: command.clone(),
            effects: effects
                .iter()
                .map(|effect| format!("{:?}", effect))
                .collect(),
            ran: true,
            exit: code,
        };
        if let Err(error) = history::JsonlReceiptWriter.append(&config.paths.data_dir, &entry) {
            eprintln!("uhm: {}", error);
        }
    }
    if args.json {
        let detected = consequential.then(|| {
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
                exit_code: code,
                executed: true,
                command: Some(&command),
                message: detected.as_deref(),
            }
            .json()
        );
    }
    code
}

fn merged_effects(local: &[Effect], declared: &[Effect]) -> Vec<Effect> {
    let mut out = local.to_vec();
    for effect in declared {
        if !out.contains(effect) {
            out.push(effect.clone());
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
                message: None,
            }
            .json()
        );
    } else {
        let _ = write_command(&mut std::io::stdout(), command);
    }
    0
}

fn write_command(mut output: impl Write, command: &str) -> std::io::Result<()> {
    output.write_all(command.as_bytes())?;
    output.flush()
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
                message: Some(message),
            }
            .json()
        );
    } else {
        eprintln!("uhm: {}", message);
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
                message: Some(message),
            }
            .json()
        );
    } else {
        eprintln!("uhm: {}", message);
    }
    code
}

fn target_shell(config: &Config, override_shell: Option<&str>) -> Result<String, String> {
    normalize_shell(
        override_shell.unwrap_or(&config.shell),
        &std::env::var("SHELL").unwrap_or_default(),
    )
}

fn normalize_shell(requested: &str, detected: &str) -> Result<String, String> {
    let requested = requested.trim();
    if requested.is_empty() || requested == "auto" {
        return Ok(if detected.trim().is_empty() {
            "/bin/sh".into()
        } else {
            detected.trim().to_string()
        });
    }
    let name = std::path::Path::new(requested)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(requested);
    if matches!(name, "sh" | "bash" | "zsh" | "fish" | "pwsh" | "powershell") {
        Ok(requested.to_string())
    } else {
        Err(format!(
            "unsupported shell '{}'; expected auto, bash, zsh, fish, or pwsh",
            requested
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_auto_uses_detected_shell_and_rejects_unknown_values() {
        assert_eq!(normalize_shell("auto", "/bin/bash"), Ok("/bin/bash".into()));
        assert_eq!(normalize_shell("fish", "/bin/bash"), Ok("fish".into()));
        assert!(normalize_shell("cmd.exe", "/bin/bash").is_err());
    }

    #[test]
    fn local_and_declared_effects_are_both_retained() {
        assert_eq!(
            merged_effects(
                &[Effect::ReadLocal],
                &[Effect::NetworkRead, Effect::ReadLocal]
            ),
            vec![Effect::ReadLocal, Effect::NetworkRead]
        );
    }

    #[test]
    fn command_channel_is_byte_exact() {
        let command = "printf  '%s\\n'\n\t'雪 --force'";
        let mut bytes = Vec::new();
        write_command(&mut bytes, command).unwrap();
        assert_eq!(bytes, command.as_bytes());
    }
}
