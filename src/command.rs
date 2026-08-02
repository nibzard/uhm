//! Bounded result-first job: at most two proposals and, for explicit repair, two executions.

use crate::action::{Effect, ProgramResultMode, ProposalMetadata, ProposedAction, StdinMode};
use crate::args::Args;
use crate::config::Config;
use crate::outcome::Outcome;
use crate::render::{ansi, card, spinner};
use crate::{
    api, cache, context, history, outcome, parent_shell, program, prompt, recovery, safety, shell,
    telemetry, tty,
};
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

#[allow(clippy::too_many_arguments)]
pub fn handle(
    args: &Args,
    config: &Config,
    api_config: &api::ApiConfig,
    request: &str,
    route: &str,
    stdin: &crate::input::Spool,
    disclosure_marker: &str,
    interaction: &mut telemetry::Interaction,
    preset_action: Option<ProposedAction>,
    related_run_id: Option<&str>,
    integration: Option<&crate::shell_integration::Session>,
    approved_history: Option<&str>,
) -> i32 {
    let started = Instant::now();
    if args.local_input && !stdin.is_piped() {
        return app_error(
            args,
            outcome::USAGE,
            "input_error",
            "--local-input requires piped input",
        );
    }
    let shell_name = match target_shell(config, args.shell.as_deref()) {
        Ok(v) => v,
        Err(e) => return app_error(args, outcome::USAGE, "usage_error", &e),
    };
    interaction.shell(&shell_name);
    if let Err(e) = ensure_disclosure(Some(disclosure_marker)) {
        return app_error(args, outcome::CONFIG, "notice_error", &e);
    }
    let mode_text = args.context.as_deref().unwrap_or(&config.context_mode);
    let mode = match context::Mode::parse(mode_text) {
        Ok(v) => v,
        Err(e) => return app_error(args, outcome::USAGE, "usage_error", &e),
    };
    // Aliases are resolved before automatic context probes and do not leave the device.
    let alias = if preset_action.is_some() {
        None
    } else {
        config
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
            })
    };
    let mut snapshot = if alias.is_some() {
        interaction.suppress();
        context::gather(
            context::Mode::Minimal,
            &shell_name,
            config.context_timeout_ms,
        )
    } else {
        context::gather(mode, &shell_name, config.context_timeout_ms)
    };
    if let Some(session) = integration {
        context::add_shell_invocation(&mut snapshot, session, approved_history);
    }
    let mut budget = Budget::default();
    let run_id = interaction.run_id.clone();
    if let Err(e) = history::record_request(
        &config.paths.data_dir,
        &config.history,
        &run_id,
        route,
        route,
        mode.as_str(),
        request,
        related_run_id,
    ) {
        eprintln!("uhm: history: {}", e);
    }
    if let Err(e) = history::record_context(
        &config.paths.data_dir,
        &config.history,
        &run_id,
        route,
        route,
        mode.as_str(),
        related_run_id,
    ) {
        eprintln!("uhm: history: {}", e);
    }
    if route == "recover" {
        let _ = history::record_recovery_event(
            &config.paths.data_dir,
            &config.history,
            &run_id,
            route,
            mode.as_str(),
            history::EventKind::BestEffortInverseRequested,
            "best_effort_inverse",
            Some("a reviewed inverse proposal is not verified restoration"),
            0,
            related_run_id,
        );
    }
    let mut action = match preset_action {
        Some(v) => v,
        None => match alias {
            Some(v) => v,
            None => match propose(
                args,
                config,
                api_config,
                route,
                request,
                &snapshot,
                stdin.model_value_for(args.local_input, args.input_format.as_deref()),
                None,
                &shell_name,
            ) {
                Ok((v, cache_hit)) => {
                    budget.initial_model();
                    interaction.proposal(true, cache_hit);
                    v
                }
                Err(e) => {
                    interaction.proposal(false, false);
                    return app_error(args, outcome::MODEL, "model_error", &e);
                }
            },
        },
    };
    let mut recovery_label_shown = false;
    loop {
        if matches!(route, "ask" | "explain")
            && !matches!(
                &action,
                ProposedAction::Answer { .. } | ProposedAction::Clarification { .. }
            )
        {
            interaction.decision("unavailable");
            return app_error(
                args,
                outcome::MODEL,
                "route_contract_error",
                "ask and explain are prose-only; the model proposed local execution",
            );
        }
        if route == "recover" && !recovery_label_shown && !args.json {
            eprintln!("Best-effort inverse: execution success does not verify that the original state was recovered.");
            recovery_label_shown = true;
        }
        if let Err(e) = history::record_proposal(
            &config.paths.data_dir,
            &config.history,
            &run_id,
            route,
            route,
            mode.as_str(),
            &action,
            related_run_id,
        ) {
            eprintln!("uhm: history: {}", e);
        }
        match action {
            ProposedAction::Answer { text } => {
                interaction.route("answer");
                interaction.decision("returned");
                if matches!(route, "run" | "recover") {
                    interaction.decision("unavailable");
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
                interaction.route("clarification");
                interaction.decision("not_run");
                if !budget.can_replace() || !tty_available() {
                    return clarification(args, &question);
                }
                eprintln!("{}", ansi::sanitize_untrusted(&question));
                eprint!("uhm› ");
                let _ = std::io::stderr().flush();
                let Some(answer) = tty::read_line_cooked() else {
                    interaction.decision("cancelled");
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
                    stdin.model_value_for(args.local_input, args.input_format.as_deref()),
                    Some(json!({"kind":"clarification","answer":answer})),
                    &shell_name,
                ) {
                    Ok((v, _)) => v,
                    Err(e) => return app_error(args, outcome::MODEL, "model_error", &e),
                };
                continue;
            }
            ProposedAction::ParentShell {
                action: parent_action,
                metadata,
            } => {
                interaction.route("parent_shell");
                let effects = merged_effects(&metadata.effects, &[Effect::ShellState]);
                interaction.effects(&effects);
                if recovery::capture_requested(
                    &config.paths.data_dir,
                    &config.recovery,
                    args.recoverable,
                ) {
                    let _ = history::record_recovery_event(
                        &config.paths.data_dir,
                        &config.history,
                        &run_id,
                        route,
                        mode.as_str(),
                        history::EventKind::RecoveryClassified,
                        recovery::RecoveryClass::BestEffortOnly.as_str(),
                        Some("parent-shell changes have a receipt but no controlled preimage"),
                        0,
                        related_run_id,
                    );
                }
                let command = match crate::shell_integration::fallback(&parent_action, &shell_name)
                {
                    Ok(v) => v,
                    Err(e) => {
                        return app_error(args, outcome::NOT_EXECUTED, "parent_action_error", &e)
                    }
                };
                if args.dry_run {
                    interaction.decision("dry_run");
                    return dry_run(args, &command);
                }
                if !args.json {
                    card::preview(
                        &command,
                        &metadata.summary,
                        safety::Tier::Low,
                        &effects,
                        &[],
                    );
                    if matches!(
                        parent_action.kind,
                        crate::action::ParentActionKind::SourceFile
                    ) {
                        eprintln!("Source warning: this file executes with your full shell authority and may exit or replace the shell before cleanup or acknowledgement.")
                    }
                }
                let Some(session) = integration else {
                    interaction.decision("needs_parent");
                    if !args.json {
                        eprintln!("This must run in your current shell. Install the optional wrapper with: uhm shell-init {}",std::path::Path::new(&shell_name).file_name().and_then(|v|v.to_str()).unwrap_or("bash"));
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
                        &effects,
                        &[],
                    );
                    return requires_parent(args, &command);
                };
                let expected = std::path::Path::new(&shell_name)
                    .file_name()
                    .and_then(|v| v.to_str())
                    .unwrap_or(&shell_name);
                if session.shell().as_str() != expected {
                    return app_error(
                        args,
                        outcome::NOT_EXECUTED,
                        "integration_shell_mismatch",
                        "the installed wrapper shell does not match the selected target shell",
                    );
                }
                if args.json && !args.force {
                    return not_executed(
                        args,
                        &command,
                        "parent-shell review is required; automation must use --force or --dry-run",
                    );
                }
                if !args.force {
                    if !tty_available() {
                        return not_executed(args,&command,"parent-shell confirmation is required, but no terminal is available; use --force or --dry-run");
                    }
                    if !ask("Apply this change to the current shell? [y/N] ") {
                        interaction.decision("cancelled");
                        return not_executed(args, &command, "cancelled by user");
                    }
                }
                if let Err(e) = session.write_response(&run_id, &parent_action) {
                    return app_error(
                        args,
                        crate::shell_integration::INTEGRATION_FAILURE,
                        "integration_response_error",
                        &e,
                    );
                }
                interaction.decision("needs_parent");
                interaction.parent_pending();
                receipt(
                    config,
                    &run_id,
                    route,
                    mode,
                    "require_parent_shell",
                    "parent_pending",
                    false,
                    0,
                    None,
                    started.elapsed(),
                    budget.second_used(),
                    &effects,
                    &[],
                );
                return 0;
            }
            ProposedAction::Program {
                program: mut proposal,
            } => {
                interaction.route("program");
                let detected = program::detected_effects(&proposal.source);
                let effects = merged_effects(&proposal.effects, &detected);
                interaction.effects(&effects);
                let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                let recovery_classification =
                    if proposal.result_mode == ProgramResultMode::Artifacts {
                        recovery::classify(
                            &config.paths.data_dir,
                            &cwd,
                            &proposal.outputs,
                            &config.recovery,
                            config.history.enabled,
                            args.recoverable,
                        )
                    } else {
                        recovery::Classification {
                            requested: recovery::capture_requested(
                                &config.paths.data_dir,
                                &config.recovery,
                                args.recoverable,
                            ),
                            class: recovery::RecoveryClass::Unavailable,
                            reason: "stdout-only programs have no managed artifact preimage".into(),
                            items: Vec::new(),
                        }
                    };
                if recovery_classification.requested {
                    if let Err(error) = history::record_recovery_event(
                        &config.paths.data_dir,
                        &config.history,
                        &run_id,
                        route,
                        mode.as_str(),
                        history::EventKind::RecoveryClassified,
                        recovery_classification.class.as_str(),
                        Some(&recovery_classification.reason),
                        recovery_classification.items.len(),
                        related_run_id,
                    ) {
                        eprintln!("uhm: history: {error}");
                    }
                }
                if !snapshot.program_runtime.available {
                    interaction.decision("unavailable");
                    return app_error(
                        args,
                        outcome::UNAVAILABLE,
                        "runtime_unavailable",
                        snapshot.program_runtime.version.as_deref().unwrap_or(
                            "Python 3 is unavailable; install python3 or use a shell route",
                        ),
                    );
                }
                let consequential = proposal.result_mode == ProgramResultMode::Artifacts
                    || effects.iter().any(Effect::requires_advisory_pause);
                let review = args.review || consequential;
                if args.dry_run {
                    interaction.decision("dry_run");
                    return dry_run(args, &proposal.source);
                }
                if review && !args.json {
                    program_preview(&proposal, &snapshot, config, &recovery_classification);
                }
                if args.json && review && !args.force {
                    return not_executed(
                        args,
                        &proposal.source,
                        "program review is required; automation must use --force or --dry-run",
                    );
                }
                if review && !args.force {
                    if !tty_available() {
                        return not_executed(
                            args,
                            &proposal.source,
                            "program review is required, but no terminal is available; use --force or --dry-run",
                        );
                    }
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
                                stdin.model_value_for(
                                    args.local_input,
                                    args.input_format.as_deref(),
                                ),
                                Some(
                                    json!({"kind":"revision","prior_action":{"kind":"program","program":proposal},"feedback":feedback}),
                                ),
                                &shell_name,
                            ) {
                                Ok((value, _)) => value,
                                Err(error) => {
                                    return app_error(args, outcome::MODEL, "model_error", &error)
                                }
                            };
                            continue;
                        }
                        "e" | "edit" if budget.replacement.is_none() => {
                            match edit(&proposal.source) {
                                Ok(source) => {
                                    proposal.source = source;
                                    action = match (ProposedAction::Program { program: proposal })
                                        .validate()
                                    {
                                        Ok(value) => value,
                                        Err(error) => {
                                            return app_error(
                                                args,
                                                outcome::NOT_EXECUTED,
                                                "edit_error",
                                                &error,
                                            )
                                        }
                                    };
                                    let _ = budget.replace_with_edit();
                                    continue;
                                }
                                Err(error) => {
                                    return app_error(
                                        args,
                                        outcome::NOT_EXECUTED,
                                        "edit_error",
                                        &error,
                                    )
                                }
                            }
                        }
                        "c" | "copy" => {
                            let _ = write_command(std::io::stdout(), &proposal.source);
                            return outcome::NOT_EXECUTED;
                        }
                        _ => {
                            interaction.decision("cancelled");
                            return not_executed(args, &proposal.source, "cancelled by user");
                        }
                    }
                } else if consequential && args.force && !args.json {
                    eprintln!(
                        "{}",
                        ansi::warning("Proceeding because --force was supplied.")
                    );
                }
                if recovery_classification.requested
                    && !recovery_classification.all_eligible()
                    && !args.force
                {
                    interaction.decision("unavailable");
                    return app_error(
                        args,
                        outcome::NOT_EXECUTED,
                        "verified_restore_unavailable",
                        &format!(
                            "{}; use --force to run without a verified restore",
                            recovery_classification.reason
                        ),
                    );
                }
                if !budget.execute() {
                    interaction.decision("unavailable");
                    return app_error(
                        args,
                        outcome::NOT_EXECUTED,
                        "budget_exhausted",
                        "execution budget exhausted",
                    );
                }
                let result = match program::execute(program::Request {
                    proposal: &proposal,
                    python: &snapshot.program_runtime,
                    stdin: stdin.is_piped().then(|| stdin.bytes()),
                    cwd: &cwd,
                    config: &config.program,
                    retain_workspace: args.retain_program,
                    recovery: recovery_classification.all_eligible().then_some(
                        program::RecoveryRequest {
                            data_dir: &config.paths.data_dir,
                            run_id: &run_id,
                            config: &config.recovery,
                            allow_unrecoverable: args.force,
                        },
                    ),
                }) {
                    Ok(value) => value,
                    Err(error) => {
                        interaction.execution("spawn_error");
                        return app_error(args, outcome::NOT_EXECUTED, "program_error", &error);
                    }
                };
                if result.code != 0
                    && budget.can_replace()
                    && budget.executions < 2
                    && tty_available()
                {
                    let diagnostics =
                        ansi::sanitize_untrusted(&String::from_utf8_lossy(&result.stderr_tail));
                    eprintln!("uhm: program exited {} ({})", result.code, diagnostics);
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
                                stdin.model_value_for(
                                    args.local_input,
                                    args.input_format.as_deref(),
                                ),
                                Some(
                                    json!({"kind":"repair","prior_action":{"kind":"program","program":proposal},"exit_code":result.code,"signal":result.signal,"stderr":diagnostics}),
                                ),
                                &shell_name,
                            ) {
                                Ok((value, _)) => value,
                                Err(error) => {
                                    return app_error(args, outcome::MODEL, "model_error", &error)
                                }
                            };
                            continue;
                        }
                        "e" | "edit" => match edit(&proposal.source) {
                            Ok(source) => {
                                proposal.source = source;
                                action = match (ProposedAction::Program { program: proposal })
                                    .validate()
                                {
                                    Ok(value) => value,
                                    Err(error) => {
                                        return app_error(
                                            args,
                                            outcome::NOT_EXECUTED,
                                            "edit_error",
                                            &error,
                                        )
                                    }
                                };
                                let _ = budget.replace_with_edit();
                                continue;
                            }
                            Err(error) => {
                                return app_error(args, outcome::NOT_EXECUTED, "edit_error", &error)
                            }
                        },
                        _ => {}
                    }
                }
                if result.code == 0 {
                    if proposal.result_mode == ProgramResultMode::Stdout {
                        let _ = std::io::stdout().write_all(&result.stdout);
                        let _ = std::io::stdout().flush();
                    } else if !args.json {
                        for artifact in &result.artifacts {
                            println!("{}", artifact.display());
                        }
                    }
                }
                if let Some(path) = &result.retained_workspace {
                    eprintln!(
                        "uhm: retained private program workspace at {}",
                        path.display()
                    );
                }
                if result.recovery_prepared {
                    let _ = history::record_recovery_event(
                        &config.paths.data_dir,
                        &config.history,
                        &run_id,
                        route,
                        mode.as_str(),
                        history::EventKind::RecoveryPrepared,
                        "preparing",
                        Some("eligible preimages were captured before the program started"),
                        proposal.outputs.len(),
                        related_run_id,
                    );
                }
                if let Some(state) = &result.recovery_state {
                    if !args.json {
                        eprintln!("Verified restore: {state} (run {run_id})");
                    }
                    let kind = if state == "available" {
                        history::EventKind::RecoveryCommitted
                    } else {
                        history::EventKind::RecoveryUnavailable
                    };
                    if let Err(error) = history::record_recovery_event(
                        &config.paths.data_dir,
                        &config.history,
                        &run_id,
                        route,
                        mode.as_str(),
                        kind,
                        state,
                        result.recovery_reason.as_deref(),
                        proposal.outputs.len(),
                        related_run_id,
                    ) {
                        eprintln!("uhm: history: {error}");
                    }
                } else if let Some(reason) = &result.recovery_reason {
                    if !args.json {
                        eprintln!(
                            "Verified restore unavailable: {}",
                            ansi::sanitize_untrusted(reason)
                        );
                    }
                    let _ = history::record_recovery_event(
                        &config.paths.data_dir,
                        &config.history,
                        &run_id,
                        route,
                        mode.as_str(),
                        history::EventKind::RecoveryUnavailable,
                        "unavailable",
                        Some(reason),
                        proposal.outputs.len(),
                        related_run_id,
                    );
                }
                let decision = if result.timed_out {
                    "timed_out"
                } else if result.output_overflow {
                    "output_overflow"
                } else if result.code == 0 {
                    "completed"
                } else {
                    "failed"
                };
                interaction.decision("ran");
                interaction.execution(if result.timed_out {
                    "timeout"
                } else if result.output_overflow {
                    "output_overflow"
                } else if result.signal.is_some() {
                    "signal"
                } else if result.code == 0 {
                    "exit_zero"
                } else {
                    "exit_nonzero"
                });
                if let Err(error) = history::record_output(
                    &config.paths.data_dir,
                    &config.history,
                    &run_id,
                    "run_program",
                    mode.as_str(),
                    Some(&result.stdout_tail),
                    Some(&result.stderr_tail),
                    result.code != 0,
                ) {
                    eprintln!("uhm: history: {}", error);
                }
                receipt(
                    config,
                    &run_id,
                    route,
                    mode,
                    "run_program",
                    decision,
                    true,
                    result.code,
                    result.signal,
                    started.elapsed(),
                    budget.second_used(),
                    &proposal.effects,
                    &detected,
                );
                if args.verbose {
                    eprintln!(
                        "uhm: program execution {} ms; stdout tail {} bytes; stderr tail {} bytes",
                        result.duration.as_millis(),
                        result.stdout_tail.len(),
                        result.stderr_tail.len()
                    );
                }
                if args.json {
                    let message = if result.artifacts.is_empty() {
                        None
                    } else {
                        Some(
                            result
                                .artifacts
                                .iter()
                                .map(|v| v.display().to_string())
                                .collect::<Vec<_>>()
                                .join(", "),
                        )
                    };
                    eprintln!(
                        "{}",
                        Outcome {
                            namespace: "uhm.child",
                            outcome: decision,
                            exit_code: result.code,
                            executed: true,
                            command: None,
                            message: message.as_deref()
                        }
                        .json()
                    );
                }
                return result.code;
            }
            ProposedAction::Shell {
                mut command,
                metadata,
                stdin_mode,
            } => {
                interaction.route("shell");
                if parent_shell::required(&command) {
                    interaction.route("parent_shell");
                    interaction.decision("needs_parent");
                    return not_executed(args,&command,"the model returned free-form parent-shell source; uhm will not parse or apply it—retry for a typed parent action");
                }
                if args.local_input && stdin_mode == StdinMode::Original {
                    interaction.decision("unavailable");
                    return app_error(
                        args,
                        outcome::NOT_EXECUTED,
                        "local_input_route_error",
                        "--local-input bytes may only be opened by a generated program; the model proposed passing them to a shell action",
                    );
                }
                let missing = context::missing_requirements(&metadata.requirements);
                if !missing.is_empty() {
                    interaction.effects(&metadata.effects);
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
                            stdin.model_value_for(args.local_input, args.input_format.as_deref()),
                            Some(json!({"kind":"revision","prior_action":command,"feedback":msg})),
                            &shell_name,
                        ) {
                            Ok((v, _)) => v,
                            Err(e) => return app_error(args, outcome::MODEL, "model_error", &e),
                        };
                        continue;
                    }
                    interaction.decision("unavailable");
                    return app_error(args, outcome::UNAVAILABLE, "requirement_unavailable", &msg);
                }
                let classification = safety::classify(&command);
                let effects = merged_effects(&classification.effects, &metadata.effects);
                interaction.effects(&effects);
                if recovery::capture_requested(
                    &config.paths.data_dir,
                    &config.recovery,
                    args.recoverable,
                ) {
                    let _ = history::record_recovery_event(
                        &config.paths.data_dir,
                        &config.history,
                        &run_id,
                        route,
                        mode.as_str(),
                        history::EventKind::RecoveryClassified,
                        recovery::RecoveryClass::BestEffortOnly.as_str(),
                        Some("shell execution has a receipt but no controlled preimage"),
                        0,
                        related_run_id,
                    );
                }
                let consequential = classification.tier.severity()
                    >= safety::Tier::Destructive.severity()
                    || effects.iter().any(Effect::requires_advisory_pause);
                let review = args.review || consequential;
                if args.dry_run {
                    interaction.decision("dry_run");
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
                                stdin.model_value_for(
                                    args.local_input,
                                    args.input_format.as_deref(),
                                ),
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
                        _ => {
                            interaction.decision("cancelled");
                            return not_executed(args, &command, "cancelled by user");
                        }
                    }
                } else if consequential && args.force && !args.json {
                    eprintln!(
                        "{}",
                        ansi::warning("Proceeding because --force was supplied.")
                    );
                }
                let child_stdin = (stdin_mode == StdinMode::Original).then(|| stdin.bytes());
                if !budget.execute() {
                    interaction.decision("unavailable");
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
                    Err(e) => {
                        interaction.execution("spawn_error");
                        return app_error(args, outcome::NOT_EXECUTED, "spawn_error", &e);
                    }
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
                                stdin.model_value_for(
                                    args.local_input,
                                    args.input_format.as_deref(),
                                ),
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
                interaction.decision("ran");
                interaction.execution(if result.timed_out {
                    "timeout"
                } else if result.signal.is_some() {
                    "signal"
                } else if result.code == 0 {
                    "exit_zero"
                } else {
                    "exit_nonzero"
                });
                if let Err(error) = history::record_output(
                    &config.paths.data_dir,
                    &config.history,
                    &run_id,
                    "run_shell",
                    mode.as_str(),
                    result.stdout_tail.as_deref(),
                    result.stderr_tail.as_deref(),
                    result.code != 0,
                ) {
                    eprintln!("uhm: history: {}", error);
                }
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
    let context_value = serde_json::to_value(snapshot).map_err(|e| e.to_string())?;
    let context_hash = blake3::hash(serde_json::to_string(&context_value).unwrap().as_bytes())
        .to_hex()
        .to_string();
    let input_hash = blake3::hash(serde_json::to_string(&stdin).unwrap().as_bytes())
        .to_hex()
        .to_string();
    let key = cache::key_hash(
        &api_config.model,
        shell,
        api_config.max_tokens,
        &api_config.reasoning_effort,
        &snapshot.mode,
        &context_hash,
        route,
        &input_hash,
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
    if marker == Some(crate::first_run::RENDERED_MARKER) {
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
        runtime: if route == "run_program" {
            "python3"
        } else {
            "none"
        }
        .into(),
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
        user_feedback: "unknown".into(),
    };
    if let Err(e) = history::append_receipt(&config.paths.data_dir, &config.history, &entry) {
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
fn program_preview(
    proposal: &crate::action::ProgramProposal,
    snapshot: &context::Snapshot,
    config: &Config,
    recovery: &recovery::Classification,
) {
    eprintln!("{}", ansi::primary("Proposed Python microprogram"));
    eprintln!("{}", ansi::sanitize_untrusted(&proposal.source));
    eprintln!("{}", ansi::sanitize_untrusted(&proposal.summary));
    eprintln!(
        "Runtime: {} -I -S\nWorking directory: private temporary directory\nResult: {:?}",
        snapshot
            .program_runtime
            .resolved_path
            .as_deref()
            .unwrap_or("python3"),
        proposal.result_mode
    );
    for input in &proposal.inputs {
        eprintln!(
            "Input ({:?}): {}",
            input.access,
            ansi::sanitize_untrusted_inline(&input.path)
        );
    }
    for output in &proposal.outputs {
        eprintln!(
            "Output (staged, then renamed): {}",
            ansi::sanitize_untrusted_inline(output)
        );
    }
    eprintln!(
        "Recovery: {} — {}",
        recovery.class.as_str(),
        ansi::sanitize_untrusted_inline(&recovery.reason)
    );
    for item in &recovery.items {
        eprintln!(
            "  {}: {} ({})",
            ansi::sanitize_untrusted_inline(&item.destination.display().to_string()),
            item.class.as_str(),
            ansi::sanitize_untrusted_inline(&item.reason)
        );
    }
    for assumption in &proposal.assumptions {
        eprintln!(
            "Assumption: {}",
            ansi::sanitize_untrusted_inline(assumption)
        );
    }
    eprintln!(
        "Limits: {}s wall, {}s CPU, {} MiB address space, {} MiB combined output, {} MiB workspace",
        config.program.timeout_secs,
        config.program.cpu_secs,
        config.program.address_space_bytes / (1024 * 1024),
        config.program.output_max_bytes / (1024 * 1024),
        config.program.workspace_max_bytes / (1024 * 1024),
    );
    eprintln!(
        "Host controls: CPU/open-files applied at spawn; address-space {}; child-process limit {} on {}.",
        if cfg!(target_os = "macos") {
            "unavailable"
        } else {
            "applied"
        },
        if cfg!(target_os = "linux") {
            "applied"
        } else {
            "unavailable"
        },
        std::env::consts::OS
    );
    eprintln!("Not sandboxed: the program runs with your user permissions and can access data your user can access.");
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
fn requires_parent(args: &Args, command: &str) -> i32 {
    if args.json {
        println!(
            "{}",
            serde_json::json!({"namespace":"uhm","outcome":"requires_parent_shell","exit_code":outcome::NOT_EXECUTED,"executed":false,"requires_parent_shell":true,"command":command,"message":"install the optional shell wrapper to apply this typed action"})
        );
    } else {
        eprintln!("uhm: parent shell state was not changed");
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
        assert!(ensure_disclosure(Some(crate::first_run::RENDERED_MARKER)).is_ok());
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

    #[test]
    fn typed_parent_action_requires_integration_and_then_publishes_one_response() {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::test(crate::dirs::Paths {
            config_file: root.path().join("config"),
            data_dir: root.path().join("data"),
            cache_dir: root.path().join("cache"),
        });
        config.history.enabled = false;
        let proposal = ProposedAction::ParentShell {
            action: crate::action::ParentAction {
                kind: crate::action::ParentActionKind::SetEnvironment,
                path: None,
                name: Some("UHM_PARENT_TEST".into()),
                value: Some("works".into()),
            },
            metadata: ProposalMetadata {
                summary: "Set a test value.".into(),
                effects: vec![Effect::ShellState],
                ..ProposalMetadata::default()
            },
        };
        let args = Args {
            force: true,
            shell: Some("bash".into()),
            ..Args::default()
        };
        let api = api::ApiConfig {
            model: "unused".into(),
            key: String::new(),
            max_tokens: 1,
            reasoning_effort: "low".into(),
            request_max_bytes: 1024,
            response_max_bytes: 1024,
        };
        let mut without = telemetry::Interaction::new("run", false, false);
        assert_eq!(
            handle(
                &args,
                &config,
                &api,
                "set a value",
                "run",
                &crate::input::Spool::default(),
                crate::first_run::RENDERED_MARKER,
                &mut without,
                Some(proposal.clone()),
                None,
                None,
                None,
            ),
            outcome::NOT_EXECUTED
        );
        let (dir, nonce) = crate::shell_integration::open(
            &config,
            crate::shell_integration::ShellFamily::Bash,
            "/tmp",
            0,
        )
        .unwrap();
        let session = crate::shell_integration::load(&config, &dir, &nonce).unwrap();
        let mut interaction = telemetry::Interaction::new("run", false, false);
        let run_id = interaction.run_id.clone();
        let code = handle(
            &args,
            &config,
            &api,
            "set a value",
            "run",
            &crate::input::Spool::default(),
            crate::first_run::RENDERED_MARKER,
            &mut interaction,
            Some(proposal),
            None,
            Some(&session),
            None,
        );
        assert_eq!(code, 0);
        let response = crate::shell_integration::validate_response(
            &config,
            &dir,
            &nonce,
            crate::shell_integration::ShellFamily::Bash,
        )
        .unwrap();
        assert_eq!(response.run_id, run_id);
        assert_eq!(response.action.name.as_deref(), Some("UHM_PARENT_TEST"));
    }
}
