//! Bounded result-first job: at most two proposals and, for explicit repair, two executions.

use crate::action::{Effect, ProposalMetadata, ProposedAction, StdinMode};
use crate::args::Args;
use crate::config::Config;
use crate::outcome::Outcome;
use crate::render::{ansi, card, spinner};
use crate::{
    api, cache, context, history, model_selection, outcome, parent_shell, program, prompt,
    recovery, safety, shell, telemetry, tool_surface, tty,
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
    replacement_after_executions: Option<u8>,
}

impl Budget {
    fn initial_model(&mut self, attempts: u8) {
        self.model_calls = attempts.min(2);
    }
    fn can_replace(&self) -> bool {
        self.replacement.is_none() && self.model_calls < 2
    }
    fn replace_with_model(&mut self, kind: Replacement) -> bool {
        if !self.can_replace() {
            return false;
        }
        self.replacement = Some(kind);
        self.replacement_after_executions = Some(self.executions);
        self.model_calls += 1;
        true
    }
    fn replace_with_edit(&mut self) -> bool {
        if self.replacement.is_some() {
            return false;
        }
        self.replacement = Some(Replacement::Edit);
        self.replacement_after_executions = Some(self.executions);
        true
    }
    fn execute(&mut self) -> bool {
        let allowed = self.executions == 0
            || (self.executions == 1
                && matches!(
                    self.replacement,
                    Some(Replacement::Repair | Replacement::Edit)
                )
                && self.replacement_after_executions == Some(1));
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
    let local_alias = alias.is_some();
    let mut snapshot = if local_alias {
        interaction.suppress();
        context::gather(
            context::Mode::Minimal,
            &shell_name,
            config.context_timeout_ms,
        )
    } else {
        context::gather(mode, &shell_name, config.context_timeout_ms)
    };
    if !local_alias && mode != context::Mode::Minimal {
        // Probing runs a local program, so consent is required before the first
        // probe of a binary and is then remembered. Without a terminal there is
        // nobody to ask, so only tools already allowed contribute.
        let interactive = tty_available() && !args.json;
        let observed = tool_surface::surface(
            request,
            &config.paths.data_dir,
            &context::path_entries(),
            Instant::now() + Duration::from_millis(config.context_timeout_ms),
            &mut |identity| {
                interactive
                    && ask(&format!(
                        "Run `{} --help` to learn its interface? [y/N] ",
                        ansi::sanitize_untrusted_inline(&identity.name)
                    ))
            },
        );
        context::add_tool_surface(&mut snapshot, &observed);
    }
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
        history::warn(&e);
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
        history::warn(&e);
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
    let (mut action, mut profile_allowed) = match preset_action {
        Some(v) => (v, true),
        None => match alias {
            Some(v) => (v, true),
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
                &run_id,
                mode,
                related_run_id,
            ) {
                Ok((v, cache_hit, attempts, allowed)) => {
                    budget.initial_model(attempts);
                    interaction.proposal(true, cache_hit);
                    (v, allowed)
                }
                Err(e) => {
                    interaction.proposal(false, false);
                    return model_error(args, &e);
                }
            },
        },
    };
    let mut recovery_label_shown = false;
    // Labeled so replacement paths nested in inner blocks can restart the job
    // explicitly rather than binding `continue` to the wrong scope.
    'job: loop {
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
                "ask and explain cannot execute local actions; retry the prose request, or use `uhm run <intent>` to authorize execution",
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
            history::warn(&e);
        }
        if !profile_allowed {
            interaction.decision("invalid");
            if budget.can_replace() && tty_available() && !args.json {
                eprintln!("The proposed action is outside the selected evidence profile.");
                eprint!("Request a complete replacement or stop? [r/N] ");
                let _ = std::io::stderr().flush();
                if matches!(
                    tty::read_line_cooked()
                        .unwrap_or_default()
                        .to_lowercase()
                        .as_str(),
                    "r" | "repair" | "y" | "yes"
                ) {
                    let _ = budget.replace_with_model(Replacement::Repair);
                    let result = match propose(
                        args,
                        config,
                        api_config,
                        route,
                        request,
                        &snapshot,
                        stdin.model_value_for(args.local_input, args.input_format.as_deref()),
                        Some(json!({
                            "kind":"evidence_profile_replacement",
                            "prior_action":action,
                            "permitted_action_types":api_config.permitted_action_types,
                            "instruction":"Return one complete replacement action, never a patch."
                        })),
                        &shell_name,
                        &run_id,
                        mode,
                        related_run_id,
                    ) {
                        Ok(value) => value,
                        Err(error) => return model_error(args, &error),
                    };
                    action = result.0;
                    profile_allowed = result.3;
                    continue;
                }
            }
            return app_error(
                args,
                outcome::NOT_EXECUTED,
                "action_outside_evidence_profile",
                "the proposed action is outside the selected evidence profile",
            );
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
                } else if std::io::stdout().is_terminal() && !ansi::plain_enabled() {
                    print!(
                        "{}",
                        crate::render::markdown::render(&ansi::sanitize_untrusted(&text))
                    );
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
                    // The job is already over, so the outstanding detail is
                    // information rather than an invitation. The question keeps
                    // its place on stdout; only the framing around it changes.
                    if budget.model_calls >= 2 && !args.json {
                        eprintln!("{}", CLARIFICATION_ENDED);
                        let code = clarification(args, &question);
                        eprintln!("{}", CLARIFICATION_RETRY);
                        return code;
                    }
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
                    &run_id,
                    mode,
                    related_run_id,
                ) {
                    Ok((v, _, _, allowed)) => {
                        profile_allowed = allowed;
                        v
                    }
                    Err(e) => return model_error(args, &e),
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
                let diagnostics =
                    program::preflight(&proposal, &snapshot.program_runtime, stdin.is_piped());
                if let Err(error) = history::record_program_preflight(
                    &config.paths.data_dir,
                    &config.history,
                    &run_id,
                    route,
                    route,
                    mode.as_str(),
                    &diagnostics,
                    related_run_id,
                ) {
                    history::warn(&error);
                }
                for diagnostic in diagnostics
                    .iter()
                    .filter(|value| value.severity == program::DiagnosticSeverity::Warning)
                {
                    eprintln!(
                        "uhm: program contract warning [{}]: {}",
                        diagnostic.code,
                        ansi::sanitize_untrusted_inline(&diagnostic.message)
                    );
                }
                if let Some(diagnostic) = diagnostics.iter().find(|value| {
                    matches!(
                        value.severity,
                        program::DiagnosticSeverity::HardError
                            | program::DiagnosticSeverity::Availability
                    )
                }) {
                    interaction.decision(
                        if diagnostic.severity == program::DiagnosticSeverity::Availability {
                            "unavailable"
                        } else {
                            "invalid"
                        },
                    );
                    if budget.can_replace() && tty_available() && !args.json {
                        eprintln!(
                            "Program contract error: {}",
                            ansi::sanitize_untrusted_inline(&diagnostic.message)
                        );
                        eprint!("Repair or stop? [r/N] ");
                        let _ = std::io::stderr().flush();
                        if matches!(
                            tty::read_line_cooked()
                                .unwrap_or_default()
                                .to_lowercase()
                                .as_str(),
                            "r" | "repair" | "y" | "yes"
                        ) {
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
                                Some(program_contract_repair_payload(&proposal, diagnostic)),
                                &shell_name,
                                &run_id,
                                mode,
                                related_run_id,
                            ) {
                                Ok((value, _, _, allowed)) => {
                                    profile_allowed = allowed;
                                    value
                                }
                                Err(error) => return model_error(args, &error),
                            };
                            continue;
                        }
                    }
                    return app_error(
                        args,
                        if diagnostic.severity == program::DiagnosticSeverity::Availability {
                            outcome::UNAVAILABLE
                        } else {
                            outcome::NOT_EXECUTED
                        },
                        &diagnostic.code,
                        &diagnostic.message,
                    );
                }
                let recovery_classification = if program::has_writable_files(&proposal) {
                    recovery::classify(
                        &config.paths.data_dir,
                        &cwd,
                        &program::writable_paths(&proposal),
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
                        history::warn(&error);
                    }
                }
                let consequential = program::has_writable_files(&proposal)
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
                    let options = review_options(&budget);
                    let decision = loop {
                        eprint!("{}", review_prompt(options));
                        let _ = std::io::stderr().flush();
                        let Some(review_choice) = tty::read_line_cooked() else {
                            interaction.decision("cancelled");
                            return not_executed(
                                args,
                                &proposal.source,
                                "review input closed; cancelled without execution",
                            );
                        };
                        match review_decision(&review_choice, options) {
                            ReviewDecision::Unavailable(reason) => {
                                eprintln!("{}", ansi::warning(reason))
                            }
                            decision => break decision,
                        }
                    };
                    match decision {
                        ReviewDecision::Run => {}
                        ReviewDecision::Revise => {
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
                                &run_id,
                                mode,
                                related_run_id,
                            ) {
                                Ok((value, _, _, allowed)) => {
                                    profile_allowed = allowed;
                                    value
                                }
                                Err(error) => return model_error(args, &error),
                            };
                            continue;
                        }
                        ReviewDecision::Edit => match edit(&proposal.source) {
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
                        ReviewDecision::Copy => {
                            let _ = write_command(std::io::stdout(), &proposal.source);
                            return outcome::NOT_EXECUTED;
                        }
                        ReviewDecision::Cancel => {
                            interaction.decision("cancelled");
                            return not_executed(args, &proposal.source, "cancelled by user");
                        }
                        ReviewDecision::Unavailable(_) => {
                            unreachable!("the prompt loop only breaks on an available option")
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
                    containment: config.execution.containment,
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
                    let record_failed_attempt = || {
                        let _ = history::record_output(
                            &config.paths.data_dir,
                            &config.history,
                            &run_id,
                            "run_program",
                            mode.as_str(),
                            Some(&result.stdout_tail),
                            Some(&result.stderr_tail),
                            true,
                        );
                        receipt(
                            config,
                            &run_id,
                            route,
                            mode,
                            "run_program",
                            if result.timed_out {
                                "timed_out"
                            } else {
                                "failed"
                            },
                            true,
                            result.code,
                            result.signal,
                            started.elapsed(),
                            budget.second_used(),
                            &proposal.effects,
                            &detected,
                        );
                    };
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
                            record_failed_attempt();
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
                                Some(program_repair_payload(
                                    &proposal,
                                    &result,
                                    (!args.local_input).then_some(diagnostics.as_str()),
                                )),
                                &shell_name,
                                &run_id,
                                mode,
                                related_run_id,
                            ) {
                                Ok((value, _, _, allowed)) => {
                                    profile_allowed = allowed;
                                    value
                                }
                                Err(error) => return model_error(args, &error),
                            };
                            continue;
                        }
                        "e" | "edit" => match edit(&proposal.source) {
                            Ok(source) => {
                                record_failed_attempt();
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
                    if !program::has_writable_files(&proposal) {
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
                        program::writable_paths(&proposal).len(),
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
                        program::writable_paths(&proposal).len(),
                        related_run_id,
                    ) {
                        history::warn(&error);
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
                        program::writable_paths(&proposal).len(),
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
                    history::warn(&error);
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
                command,
                metadata,
                stdin_mode,
            } => {
                interaction.route("shell");
                if let Some(variable) = referenced_provider_credential(&command) {
                    interaction.decision("not_run");
                    return app_error(
                        args,
                        outcome::NOT_EXECUTED,
                        "credential_isolation",
                        &format!(
                            "{variable} is intentionally unavailable to generated commands. Use `uhm doctor` to inspect credential status and the private secrets path; uhm never prints provider keys."
                        ),
                    );
                }
                if parent_shell::required(&command) {
                    interaction.route("parent_shell");
                    interaction.decision("needs_parent");
                    let message = if local_alias {
                        "the local alias contains parent-shell source; local aliases cannot directly change the current shell—use a typed parent-shell action through the shell integration"
                    } else {
                        "the model returned free-form parent-shell source; uhm will not parse or apply it—retry for a typed parent action"
                    };
                    return not_executed(args, &command, message);
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
                            &run_id,
                            mode,
                            related_run_id,
                        ) {
                            Ok((v, _, _, allowed)) => {
                                profile_allowed = allowed;
                                v
                            }
                            Err(e) => return model_error(args, &e),
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
                    let options = review_options(&budget);
                    let decision = loop {
                        eprint!("{}", review_prompt(options));
                        let _ = std::io::stderr().flush();
                        let Some(review_choice) = tty::read_line_cooked() else {
                            interaction.decision("cancelled");
                            return not_executed(
                                args,
                                &command,
                                "review input closed; cancelled without execution",
                            );
                        };
                        match review_decision(&review_choice, options) {
                            ReviewDecision::Unavailable(reason) => {
                                eprintln!("{}", ansi::warning(reason))
                            }
                            decision => break decision,
                        }
                    };
                    match decision {
                        ReviewDecision::Run => {}
                        ReviewDecision::Revise => {
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
                                &run_id,
                                mode,
                                related_run_id,
                            ) {
                                Ok((v, _, _, allowed)) => {
                                    profile_allowed = allowed;
                                    v
                                }
                                Err(e) => return model_error(args, &e),
                            };
                            continue;
                        }
                        ReviewDecision::Edit => match edit(&command) {
                            Ok(v) => {
                                let _ = budget.replace_with_edit();
                                action = match (ProposedAction::Shell {
                                    command: v,
                                    // Provider claims no longer describe edited
                                    // bytes; the next loop re-detects effects,
                                    // parent-shell requirements, and warnings.
                                    metadata: crate::action::ProposalMetadata {
                                        summary: "user-edited command".into(),
                                        assumptions: Vec::new(),
                                        effects: Vec::new(),
                                        requirements: Vec::new(),
                                    },
                                    stdin_mode,
                                })
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
                                continue;
                            }
                            Err(e) => {
                                return app_error(args, outcome::NOT_EXECUTED, "edit_error", &e)
                            }
                        },
                        ReviewDecision::Copy => {
                            let _ = write_command(std::io::stdout(), &command);
                            return outcome::NOT_EXECUTED;
                        }
                        ReviewDecision::Cancel => {
                            interaction.decision("cancelled");
                            return not_executed(args, &command, "cancelled by user");
                        }
                        ReviewDecision::Unavailable(_) => {
                            unreachable!("the prompt loop only breaks on an available option")
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
                    deny_common_env: config.execution.deny_common_env,
                    deny_env: &config.execution.deny_env,
                    containment: config.execution.containment,
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
                    let record_failed_attempt = || {
                        let _ = history::record_output(
                            &config.paths.data_dir,
                            &config.history,
                            &run_id,
                            "run_shell",
                            mode.as_str(),
                            result.stdout_tail.as_deref(),
                            result.stderr_tail.as_deref(),
                            true,
                        );
                        receipt(
                            config,
                            &run_id,
                            route,
                            mode,
                            "run_shell",
                            if result.timed_out {
                                "timed_out"
                            } else {
                                "failed"
                            },
                            true,
                            result.code,
                            result.signal,
                            started.elapsed(),
                            budget.second_used(),
                            &effects,
                            &detected,
                        );
                    };
                    let diagnostics = result
                        .stderr_tail
                        .as_ref()
                        .map(|v| ansi::sanitize_untrusted(&String::from_utf8_lossy(v)));
                    eprintln!(
                        "uhm: command exited {} ({})",
                        result.code,
                        diagnostics.as_deref().unwrap_or(NO_RETAINED_DIAGNOSTICS)
                    );
                    // Without retained diagnostics the repair seed holds only the
                    // inputs that already produced this failure, so repair needs
                    // the user to supply what the child printed.
                    if diagnostics.is_none() {
                        eprint!("Repair with feedback, edit, or stop? [r/e/N] ");
                    } else {
                        eprint!("Repair, edit, or stop? [r/e/N] ");
                    }
                    let _ = std::io::stderr().flush();
                    // Breaking out of this block declines replacement and reports
                    // the failure through the ordinary path below.
                    'replacement: {
                        match tty::read_line_cooked()
                            .unwrap_or_default()
                            .to_lowercase()
                            .as_str()
                        {
                            "r" | "repair" | "y" | "yes" => {
                                let feedback = if diagnostics.is_none() {
                                    eprint!("Feedback: ");
                                    let _ = std::io::stderr().flush();
                                    let value = tty::read_line_cooked().unwrap_or_default();
                                    if value.trim().is_empty() {
                                        eprintln!(
                                        "{}",
                                        ansi::warning(
                                            "no diagnostics and no feedback: repair would resend the same command"
                                        )
                                    );
                                        break 'replacement;
                                    }
                                    Some(value.trim().to_owned())
                                } else {
                                    None
                                };
                                record_failed_attempt();
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
                                    Some(shell_repair_payload(
                                        &command,
                                        &result,
                                        diagnostics.as_deref(),
                                        feedback.as_deref(),
                                    )),
                                    &shell_name,
                                    &run_id,
                                    mode,
                                    related_run_id,
                                ) {
                                    Ok((v, _, _, allowed)) => {
                                        profile_allowed = allowed;
                                        v
                                    }
                                    Err(e) => return model_error(args, &e),
                                };
                                continue 'job;
                            }
                            "e" | "edit" => match edit(&command) {
                                Ok(replacement) => {
                                    record_failed_attempt();
                                    let _ = budget.replace_with_edit();
                                    action = match (ProposedAction::Shell {
                                        command: replacement,
                                        metadata: crate::action::ProposalMetadata {
                                            summary: "user-edited command".into(),
                                            assumptions: Vec::new(),
                                            effects: Vec::new(),
                                            requirements: Vec::new(),
                                        },
                                        stdin_mode,
                                    })
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
                                    continue 'job;
                                }
                                Err(e) => {
                                    return app_error(args, outcome::NOT_EXECUTED, "edit_error", &e)
                                }
                            },
                            _ => {}
                        }
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
                    history::warn(&error);
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
                if result.code == 0 && std::io::stderr().is_terminal() && !args.json {
                    let finished = if ansi::plain_enabled() {
                        "Finished"
                    } else {
                        "✓ Finished"
                    };
                    eprintln!("\n{}", ansi::success(finished));
                }
                return result.code;
            }
        }
    }
}

fn referenced_provider_credential(command: &str) -> Option<&'static str> {
    ["OPENAI_API_KEY", "CEREBRAS_API_KEY"]
        .into_iter()
        .find(|variable| command.contains(variable))
}

/// Build the only model-visible program failure payload. Child-derived text is
/// a separate optional input so local-input callers cannot accidentally include
/// it by interpolating a diagnostic string into the coarse outcome.
fn program_repair_payload(
    proposal: &crate::action::ProgramProposal,
    result: &program::ExecutionResult,
    diagnostic: Option<&str>,
) -> Value {
    let failure_class = if result.timed_out {
        "timeout"
    } else if result.output_overflow {
        "output_overflow"
    } else if result.signal.is_some() {
        "signal"
    } else {
        "exit_nonzero"
    };
    let mut value = json!({
        "kind":"repair",
        "prior_action":{"kind":"program","program":proposal},
        "failure":{
            "class":failure_class,
            "exit_code":result.code,
            "signal":result.signal,
            "timed_out":result.timed_out,
            "output_overflow":result.output_overflow
        }
    });
    if let Some(diagnostic) = diagnostic {
        value["diagnostic"] = Value::String(diagnostic.to_owned());
    }
    value
}

/// Framing for a clarification that can no longer be answered. Neither line may
/// be phrased as a question: the job has ended, and asking again would offer an
/// interaction that cannot be honored.
const CLARIFICATION_ENDED: &str =
    "uhm: this job ended without an action; another detail was needed and its two model calls are spent.";
const CLARIFICATION_RETRY: &str = "uhm: re-run the request with that detail included.";

/// Shown to the user when a terminal-attached child left no retained stderr.
/// It describes `uhm`'s own stream wiring, so it is never sent to the model as
/// if it were child output.
const NO_RETAINED_DIAGNOSTICS: &str =
    "diagnostics unavailable because stderr was attached to the terminal";

fn shell_repair_payload(
    command: &str,
    result: &shell::Result,
    diagnostics: Option<&str>,
    feedback: Option<&str>,
) -> Value {
    json!({
        "kind":"repair",
        "prior_action":command,
        "exit_code":result.code,
        "signal":result.signal,
        "stderr":diagnostics,
        "feedback":feedback
    })
}

/// Which review options the current budget can actually honor. Revision spends
/// the global second model call; a local edit only needs the replacement slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReviewOptions {
    revise: bool,
    edit: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum ReviewDecision {
    Run,
    Revise,
    Edit,
    Copy,
    Cancel,
    Unavailable(&'static str),
}

fn review_options(budget: &Budget) -> ReviewOptions {
    ReviewOptions {
        revise: budget.can_replace(),
        edit: budget.replacement.is_none(),
    }
}

fn review_prompt(options: ReviewOptions) -> String {
    let mut words = vec!["Run"];
    let mut keys = vec!["R"];
    if options.revise {
        words.push("revise");
        keys.push("v");
    }
    if options.edit {
        words.push("edit");
        keys.push("e");
    }
    words.push("copy");
    keys.push("c");
    words.push("cancel");
    keys.push("q");
    format!("{}? [{}] ", words.join(", "), keys.join("/"))
}

/// Resolve one review keystroke. An option that exists but is not currently
/// offered explains itself so the caller can re-prompt; it never silently
/// becomes a cancellation.
fn review_decision(input: &str, options: ReviewOptions) -> ReviewDecision {
    match input.trim().to_lowercase().as_str() {
        "" | "r" | "run" => ReviewDecision::Run,
        "v" | "revise" => {
            if options.revise {
                ReviewDecision::Revise
            } else {
                ReviewDecision::Unavailable(
                    "revise is unavailable: this job has already spent its one replacement turn",
                )
            }
        }
        "e" | "edit" => {
            if options.edit {
                ReviewDecision::Edit
            } else {
                ReviewDecision::Unavailable(
                    "edit is unavailable: this job has already spent its one replacement turn",
                )
            }
        }
        "c" | "copy" => ReviewDecision::Copy,
        _ => ReviewDecision::Cancel,
    }
}

fn program_contract_repair_payload(
    proposal: &crate::action::ProgramProposal,
    diagnostic: &program::ProgramContractDiagnostic,
) -> Value {
    json!({
        "kind":"program_contract_repair",
        "prior_action":{"kind":"program","program":proposal},
        "diagnostic":{
            "code":diagnostic.code,
            "severity":diagnostic.severity,
            "explanation":diagnostic.message,
        },
        "instruction":"Return one complete replacement action, never a patch."
    })
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
    run_id: &str,
    mode: context::Mode,
    related_run_id: Option<&str>,
) -> Result<(ProposedAction, bool, u8, bool), ProposalError> {
    let context_value = serde_json::to_value(snapshot).map_err(|e| e.to_string())?;
    let context_hash = blake3::hash(serde_json::to_string(&context_value).unwrap().as_bytes())
        .to_hex()
        .to_string();
    let input_hash = blake3::hash(serde_json::to_string(&stdin).unwrap().as_bytes())
        .to_hex()
        .to_string();
    let key = cache::key_hash(
        api_config.provider,
        api_config.selection_mode,
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
            api_config.provider,
        ) {
            if let Ok(action) = api::parse_response(api_config, &raw) {
                let profile_allowed =
                    api_config
                        .permitted_action_types
                        .as_ref()
                        .is_none_or(|allowed| {
                            allowed
                                .iter()
                                .any(|value| value == model_selection::action_type(&action))
                        });
                if args.verbose {
                    eprintln!("uhm: cache hit {}", &key[..8]);
                }
                return Ok((action, true, 0, profile_allowed));
            }
        }
    }
    let cacheable = follow_up.is_none();
    let allow_fallback = follow_up.is_none();
    let input = prompt::proposal_input(route, request, context_value, stdin, follow_up);
    let mut progress = spinner::Spinner::start("thinking");
    let response = api::request_action(
        api_config,
        &input,
        config.stream && !args.no_stream && !args.json,
        allow_fallback,
    );
    progress.stop();
    let response = match response {
        Ok(response) => {
            if let Err(error) = history::record_provider_attempts(
                &config.paths.data_dir,
                &config.history,
                run_id,
                route,
                route,
                mode.as_str(),
                &response.attempts,
                api_config.selection_mode,
                related_run_id,
            ) {
                history::warn(&error);
            }
            response
        }
        Err(error) => {
            if args.verbose {
                eprintln!(
                    "uhm: provider attempts consumed={} outcome={:?}",
                    error.attempts_consumed, error.kind
                );
            }
            if let Err(history_error) = history::record_provider_attempts(
                &config.paths.data_dir,
                &config.history,
                run_id,
                route,
                route,
                mode.as_str(),
                &error.attempts,
                api_config.selection_mode,
                related_run_id,
            ) {
                history::warn(&history_error);
            }
            return Err(ProposalError {
                message: error.message,
                kind: error.kind,
            });
        }
    };
    let action = response.action;
    let profile_allowed = response.profile_allowed;
    let raw = response.raw;
    // A fallback response belongs to a different provider provenance and must
    // never be published beneath the initial provider's cache key.
    if cacheable && response.attempts_consumed == 1 {
        if let Err(e) = cache::put(
            &config.paths.cache_dir,
            config.cache_enabled,
            &key,
            &raw,
            api_config.provider,
        ) {
            eprintln!("uhm: {}", e)
        }
    }
    Ok((action, false, response.attempts_consumed, profile_allowed))
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
        app_version: env!("CARGO_PKG_VERSION").into(),
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
        declared_effects: declared.iter().map(|e| e.wire_name().into()).collect(),
        detected_effects: detected.iter().map(|e| e.wire_name().into()).collect(),
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
        history::warn(&e)
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
        if program::has_writable_files(proposal) {
            "artifacts"
        } else {
            "stdout"
        }
    );
    eprintln!("Contract: {}\nProcess stdin: closed", proposal.contract);
    for file in &proposal.files {
        eprintln!(
            "Resource {} ({:?}): {}",
            ansi::sanitize_untrusted_inline(&file.id),
            file.access,
            ansi::sanitize_untrusted_inline(&file.path)
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

#[derive(Debug)]
struct ProposalError {
    message: String,
    kind: Option<crate::provider::ProviderErrorKind>,
}

impl From<String> for ProposalError {
    fn from(message: String) -> Self {
        Self {
            message,
            kind: None,
        }
    }
}

fn model_error(args: &Args, error: &ProposalError) -> i32 {
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "namespace": "uhm",
                "outcome": "model_error",
                "exit_code": outcome::MODEL,
                "executed": false,
                "error_kind": error.kind,
                "message": error.message,
            })
        )
    } else {
        eprintln!("uhm: {}", error.message)
    }
    outcome::MODEL
}
pub(crate) fn target_shell(config: &Config, over: Option<&str>) -> Result<String, String> {
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
    fn local_input_repair_payload_cannot_contain_child_diagnostics() {
        let proposal = crate::action::ProgramProposal {
            runtime: crate::action::ProgramRuntime::Python3,
            contract: "uhm_helper_v1".into(),
            source: "raise SystemExit(1)".into(),
            summary: "test".into(),
            assumptions: vec![],
            stdin_mode: crate::action::ProgramStdinMode::None,
            files: vec![],
            effects: vec![],
        };
        let result = program::ExecutionResult {
            code: 1,
            signal: None,
            stdout: vec![],
            stdout_tail: vec![],
            stderr_tail: b"LOCAL-INPUT-SENTINEL".to_vec(),
            timed_out: false,
            output_overflow: false,
            duration: std::time::Duration::ZERO,
            helper_setup_duration: std::time::Duration::ZERO,
            artifacts: vec![],
            retained_workspace: None,
            recovery_prepared: false,
            recovery_state: None,
            recovery_reason: None,
            artifact_commit_success: false,
        };
        let local = program_repair_payload(&proposal, &result, None).to_string();
        assert!(!local.contains("LOCAL-INPUT-SENTINEL"));
        let ordinary =
            program_repair_payload(&proposal, &result, Some("LOCAL-INPUT-SENTINEL")).to_string();
        assert!(ordinary.contains("LOCAL-INPUT-SENTINEL"));
    }

    #[test]
    fn contract_repair_payload_has_only_model_authored_and_content_free_fields() {
        let proposal = crate::action::ProgramProposal {
            runtime: crate::action::ProgramRuntime::Python3,
            contract: "uhm_helper_v1".into(),
            source: "import sys\nprint(sys.stdin.read())".into(),
            summary: "Read input".into(),
            assumptions: vec![],
            stdin_mode: crate::action::ProgramStdinMode::LocalPath,
            files: vec![],
            effects: vec![crate::action::Effect::ReadLocal],
        };
        let diagnostic = program::ProgramContractDiagnostic {
            code: "process_stdin_is_closed".into(),
            severity: program::DiagnosticSeverity::HardError,
            message: "Process stdin is closed; use uhm_runtime.stdin_path.".into(),
        };
        let payload = program_contract_repair_payload(&proposal, &diagnostic).to_string();
        assert!(payload.contains("process_stdin_is_closed"));
        for secret in [
            "LOCAL-INPUT-SENTINEL",
            ".uhm-stage-secret",
            "OPENAI-KEY-SENTINEL",
            "launcher-contract.json",
        ] {
            assert!(!payload.contains(secret));
        }
    }
    #[test]
    fn gate_blocks_missing_marker() {
        assert!(ensure_disclosure(None).is_err());
        assert!(ensure_disclosure(Some(crate::first_run::RENDERED_MARKER)).is_ok());
    }

    #[test]
    fn provider_credentials_are_caught_before_child_execution() {
        assert_eq!(
            referenced_provider_credential("print -r -- \"$OPENAI_API_KEY\""),
            Some("OPENAI_API_KEY")
        );
        assert_eq!(referenced_provider_credential("printf ordinary"), None);
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
            budget.initial_model(1);
            assert!(budget.replace_with_model(first));
            assert!(!budget.replace_with_model(Replacement::Revision));
            assert!(!budget.replace_with_edit());
            assert_eq!(budget.model_calls, 2);
        }
    }

    #[test]
    fn only_post_failure_replacement_allows_a_second_execution() {
        let mut normal = Budget::default();
        normal.initial_model(1);
        assert!(normal.execute());
        assert!(!normal.execute());

        let mut repaired = Budget::default();
        repaired.initial_model(1);
        assert!(repaired.execute());
        assert!(repaired.replace_with_model(Replacement::Repair));
        assert!(repaired.execute());
        assert!(!repaired.execute());

        let mut clarified = Budget::default();
        clarified.initial_model(1);
        assert!(clarified.replace_with_model(Replacement::Clarification));
        assert!(clarified.execute());
        assert!(!clarified.execute());
    }

    #[test]
    fn preexecution_repair_uses_two_calls_but_only_one_execution() {
        let mut budget = Budget::default();
        budget.initial_model(1);
        assert!(budget.replace_with_model(Replacement::Repair));
        assert!(budget.execute());
        assert!(!budget.execute());
        assert_eq!(budget.model_calls, 2);
        assert_eq!(budget.executions, 1);
    }

    #[test]
    fn invalid_replacement_has_no_third_turn() {
        let mut budget = Budget::default();
        budget.initial_model(1);
        assert!(budget.replace_with_model(Replacement::Repair));
        assert!(!budget.replace_with_model(Replacement::Repair));
        assert!(!budget.replace_with_edit());
        assert_eq!(budget.model_calls, 2);
    }

    #[test]
    fn transport_fallback_consumes_the_global_second_call_slot() {
        let mut budget = Budget::default();
        budget.initial_model(2);
        assert!(!budget.can_replace());
        assert!(!budget.replace_with_model(Replacement::Clarification));
        assert!(budget.execute());
        assert!(!budget.execute());
        assert_eq!(budget.model_calls, 2);
        assert_eq!(budget.executions, 1);
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
            provider: crate::provider::ProviderId::Openai,
            model: "unused".into(),
            key: String::new(),
            max_tokens: 1,
            reasoning_effort: "low".into(),
            request_max_bytes: 1024,
            response_max_bytes: 1024,
            alternate: None,
            fallback_on: Vec::new(),
            selection_mode: crate::config::SelectionMode::Fixed,
            permitted_action_types: None,
            resolved_fingerprint: None,
            resolved_model: None,
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

    fn failed_shell_result(stderr_tail: Option<&[u8]>) -> shell::Result {
        shell::Result {
            code: 2,
            signal: None,
            stdout_tail: None,
            stderr_tail: stderr_tail.map(<[u8]>::to_vec),
            timed_out: false,
            duration: std::time::Duration::ZERO,
        }
    }

    #[test]
    fn an_unanswerable_clarification_is_framed_as_a_statement() {
        for line in [CLARIFICATION_ENDED, CLARIFICATION_RETRY] {
            assert!(
                !line.trim_end().ends_with('?'),
                "an ended job must not ask: {line}"
            );
            assert!(line.starts_with("uhm: "), "{line}");
        }
        assert!(CLARIFICATION_RETRY.contains("re-run"));
    }

    #[test]
    fn review_prompt_offers_only_the_options_the_budget_can_honor() {
        assert_eq!(
            review_prompt(ReviewOptions {
                revise: true,
                edit: true
            }),
            "Run, revise, edit, copy, cancel? [R/v/e/c/q] "
        );
        assert_eq!(
            review_prompt(ReviewOptions {
                revise: false,
                edit: true
            }),
            "Run, edit, copy, cancel? [R/e/c/q] "
        );
        assert_eq!(
            review_prompt(ReviewOptions {
                revise: true,
                edit: false
            }),
            "Run, revise, copy, cancel? [R/v/c/q] "
        );
        assert_eq!(
            review_prompt(ReviewOptions {
                revise: false,
                edit: false
            }),
            "Run, copy, cancel? [R/c/q] "
        );
    }

    #[test]
    fn a_spent_replacement_slot_hides_revise_and_edit_from_the_review_prompt() {
        let mut budget = Budget::default();
        budget.initial_model(1);
        assert_eq!(
            review_options(&budget),
            ReviewOptions {
                revise: true,
                edit: true
            }
        );
        assert!(budget.replace_with_model(Replacement::Repair));
        assert_eq!(
            review_options(&budget),
            ReviewOptions {
                revise: false,
                edit: false
            }
        );
    }

    #[test]
    fn an_unavailable_option_explains_itself_instead_of_cancelling_the_job() {
        let spent = ReviewOptions {
            revise: false,
            edit: false,
        };
        assert!(matches!(
            review_decision("v", spent),
            ReviewDecision::Unavailable(_)
        ));
        assert!(matches!(
            review_decision("revise", spent),
            ReviewDecision::Unavailable(_)
        ));
        assert!(matches!(
            review_decision("e", spent),
            ReviewDecision::Unavailable(_)
        ));
        assert!(matches!(
            review_decision("edit", spent),
            ReviewDecision::Unavailable(_)
        ));
    }

    #[test]
    fn review_decision_maps_every_advertised_key() {
        let live = ReviewOptions {
            revise: true,
            edit: true,
        };
        for input in ["", "r", "run", "R"] {
            assert_eq!(review_decision(input, live), ReviewDecision::Run, "{input}");
        }
        assert_eq!(review_decision("v", live), ReviewDecision::Revise);
        assert_eq!(review_decision("e", live), ReviewDecision::Edit);
        assert_eq!(review_decision("c", live), ReviewDecision::Copy);
        assert_eq!(review_decision("copy", live), ReviewDecision::Copy);
        for input in ["q", "n", "no", "cancel", "quit"] {
            assert_eq!(
                review_decision(input, live),
                ReviewDecision::Cancel,
                "{input}"
            );
        }
        assert_eq!(review_decision("zzz", live), ReviewDecision::Cancel);
    }

    #[test]
    fn shell_repair_payload_sends_null_rather_than_the_host_placeholder() {
        let result = failed_shell_result(None);
        let blind = shell_repair_payload("steel session start", &result, None, None);
        assert_eq!(blind["stderr"], Value::Null);
        assert!(!blind.to_string().contains(NO_RETAINED_DIAGNOSTICS));
        assert!(!blind.to_string().contains("stderr was attached"));
    }

    #[test]
    fn shell_repair_payload_carries_retained_diagnostics_and_user_feedback() {
        let result = failed_shell_result(Some(b"unrecognized subcommand 'session'"));
        let observed = shell_repair_payload(
            "steel session start",
            &result,
            Some("unrecognized subcommand 'session'"),
            None,
        );
        assert_eq!(
            observed["stderr"],
            Value::String("unrecognized subcommand 'session'".into())
        );
        assert_eq!(observed["feedback"], Value::Null);
        let guided = shell_repair_payload(
            "steel session start",
            &result,
            None,
            Some("the subcommand is `sessions`, not `session`"),
        );
        assert_eq!(
            guided["feedback"],
            Value::String("the subcommand is `sessions`, not `session`".into())
        );
        assert_eq!(guided["exit_code"], Value::from(2));
    }
}
