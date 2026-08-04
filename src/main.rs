//! uhm — say what you need; get the result.

mod action;
mod api;
mod args;
mod cache;
mod capabilities;
mod clock;
mod command;
mod config;
mod context;
#[allow(dead_code)]
mod contract;
mod dirs;
mod doctor;
mod first_run;
mod history;
mod http;
mod input;
mod model_selection;
mod outcome;
mod parent_shell;
#[allow(dead_code)]
mod program;
mod prompt;
mod provider;
mod recovery;
mod render;
mod runtime;
mod safety;
mod secret;
mod shell;
mod shell_integration;
mod sse;
mod telemetry;
mod tty;

use render::ansi;
use std::io::{IsTerminal, Write};
const VERSION: &str = env!("CARGO_PKG_VERSION");
fn main() {
    std::process::exit(run(std::env::args().collect()))
}

fn run(argv: Vec<String>) -> i32 {
    let mut args = match args::parse_from(argv) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("uhm: {}", e);
            return outcome::USAGE;
        }
    };
    ansi::set_plain(args.plain);
    ansi::set_no_motion(args.no_motion);
    if args.help || args.subcommand.as_deref() == Some("help") {
        print_help();
        return 0;
    }
    if args.version || args.subcommand.as_deref() == Some("version") {
        println!("uhm {}", VERSION);
        return 0;
    }
    if args.subcommand.as_deref() == Some("shell-init") {
        let words = args.operands.iter().map(String::as_str).collect::<Vec<_>>();
        return match words.as_slice() {
            [shell] => match shell_integration::ShellFamily::parse(shell) {
                Ok(shell) => {
                    print!("{}", shell_integration::template(shell));
                    0
                }
                Err(e) => app_error(
                    &args,
                    outcome::USAGE,
                    "usage_error",
                    &format!("usage: uhm shell-init bash|zsh|fish ({})", e),
                ),
            },
            _ => app_error(
                &args,
                outcome::USAGE,
                "usage_error",
                "usage: uhm shell-init bash|zsh|fish",
            ),
        };
    }
    let config = match config::load(args.provider.as_deref(), args.model.as_deref()) {
        Ok(v) => v,
        Err(e) => return app_error(&args, outcome::CONFIG, "configuration_error", &e),
    };
    let telemetry_policy = telemetry::policy(&config, args.no_telemetry);
    let _interrupted_recovery_count =
        recovery::startup_check(&config.paths.data_dir, &config.recovery);
    if let Some(code) = integration_management(&args, &config, &telemetry_policy) {
        return code;
    }
    let integration = match (&args.control_dir, &args.control_nonce) {
        (None, None) => None,
        (Some(dir), Some(nonce)) => {
            match shell_integration::load(&config, std::path::Path::new(dir), nonce) {
                Ok(value) => Some(value),
                Err(e) => return app_error(&args, outcome::CONFIG, "integration_error", &e),
            }
        }
        _ => {
            return app_error(
                &args,
                outcome::USAGE,
                "usage_error",
                "the reserved integration directory and nonce must be supplied together",
            )
        }
    };
    let approved_history = if args.is_local_only() {
        None
    } else if let Some(entry) = args.last_history_entry.as_deref() {
        if !config.shell_context.last_history_entry {
            return app_error(
                &args,
                outcome::USAGE,
                "history_context_disabled",
                "shell_context.last_history_entry is off",
            );
        }
        if entry.is_empty() || entry.len() > 16 * 1024 || entry.contains('\0') {
            return app_error(
                &args,
                outcome::USAGE,
                "history_context_invalid",
                "the one-entry shell history sample is empty, oversized, or contains NUL",
            );
        }
        eprintln!("uhm: the following one shell-history entry will be sent to OpenAI if you continue:\n{}",ansi::sanitize_untrusted(entry));
        if !std::io::stderr().is_terminal() {
            return app_error(
                &args,
                outcome::NOT_EXECUTED,
                "history_context_cancelled",
                "a terminal is required to approve sending shell history",
            );
        }
        eprint!("Continue? [y/N] ");
        let _ = std::io::stderr().flush();
        if !tty::read_line_cooked().is_some_and(|v| matches!(v.as_str(), "y" | "yes")) {
            return app_error(
                &args,
                outcome::NOT_EXECUTED,
                "history_context_cancelled",
                "shell history was not sent",
            );
        }
        Some(entry)
    } else {
        None
    };
    // The first-use disclosure gates outbound work, not every invocation, so
    // skip it (and the notice marker) for purely-local commands; it is rendered
    // on the first actual outbound request instead.
    let disclosure_marker = if args.is_local_only() {
        first_run::RENDERED_MARKER
    } else {
        match first_run::ensure(&config, telemetry_policy.enabled) {
            Ok(marker) => marker,
            Err(e) => return app_error(&args, outcome::CONFIG, "notice_error", &e),
        }
    };
    let mut preset_action = None;
    let mut related_run_id = None;
    if args.subcommand.as_deref() == Some("history") {
        let words = args.operands.iter().map(String::as_str).collect::<Vec<_>>();
        if words.first() == Some(&"replay") {
            if words.len() != 3 || words[2] != "--review" {
                return app_error(
                    &args,
                    outcome::USAGE,
                    "usage_error",
                    "usage: uhm history replay <run-id|last> --review",
                );
            }
            match history::load_proposal(&config.paths.data_dir, words[1]) {
                Ok((id, action)) => {
                    related_run_id = Some(id);
                    preset_action = Some(action);
                    args.subcommand = Some("run".into());
                    args.prompt =
                        "Replay retained proposal under current context and policy".into();
                    args.review = true;
                }
                Err(e) => return app_error(&args, outcome::NOT_EXECUTED, "replay_unavailable", &e),
            }
        }
    }
    if args.subcommand.as_deref() == Some("repair") {
        let selected = args.operands.first().map(String::as_str).unwrap_or("last");
        let feedback_text = args
            .operands
            .get(1..)
            .unwrap_or_default()
            .iter()
            .map(String::as_str)
            .filter(|value| *value != "--")
            .collect::<Vec<_>>()
            .join(" ");
        let feedback = (!feedback_text.is_empty()).then_some(feedback_text.as_str());
        match history::repair_seed(
            &config.paths.data_dir,
            if selected.is_empty() {
                "last"
            } else {
                selected
            },
            feedback,
        ) {
            Ok((id, seed)) => {
                let request = format!(
                    "Repair the retained action using only this bounded receipt subset.\n{seed}"
                );
                eprintln!("uhm: exact retained subset and repair instruction that will be sent to OpenAI:\n{}", ansi::sanitize_untrusted(&request));
                eprintln!("uhm: the selected current machine context will also be sent under the normal context policy. Snapshots and the full journal will not be sent.");
                if !std::io::stderr().is_terminal() {
                    return app_error(
                        &args,
                        outcome::NOT_EXECUTED,
                        "repair_cancelled",
                        "a terminal is required to approve the bounded repair request",
                    );
                }
                eprint!("Send this repair request? [y/N] ");
                let _ = std::io::stderr().flush();
                if !tty::read_line_cooked()
                    .is_some_and(|value| matches!(value.as_str(), "y" | "yes"))
                {
                    return app_error(
                        &args,
                        outcome::NOT_EXECUTED,
                        "repair_cancelled",
                        "repair request was not sent",
                    );
                }
                related_run_id = Some(id);
                args.prompt = request;
                args.review = true;
                args.fresh = true;
            }
            Err(e) => return app_error(&args, outcome::NOT_EXECUTED, "repair_unavailable", &e),
        }
    }
    if args.subcommand.as_deref() == Some("recover") {
        if args.force {
            return app_error(
                &args,
                outcome::USAGE,
                "usage_error",
                "recover always requires review; --force is not accepted",
            );
        }
        let selected = args.operands.first().map(String::as_str).unwrap_or("last");
        let guidance_text = args
            .operands
            .get(1..)
            .unwrap_or_default()
            .iter()
            .map(String::as_str)
            .filter(|value| *value != "--")
            .collect::<Vec<_>>()
            .join(" ");
        let guidance = (!guidance_text.is_empty()).then_some(guidance_text.as_str());
        match history::recovery_seed(
            &config.paths.data_dir,
            if selected.is_empty() {
                "last"
            } else {
                selected
            },
            guidance,
        ) {
            Ok((id, subset)) => {
                let request = format!("Propose one best-effort inverse for this retained receipt subset. Execution success must not be described as verified restoration.\n{subset}");
                eprintln!("uhm: exact retained subset and recovery instruction that will be sent to OpenAI:\n{}", ansi::sanitize_untrusted(&request));
                eprintln!("uhm: the selected current machine context will also be sent under the normal context policy. Snapshots and the full journal will not be sent.");
                if !std::io::stderr().is_terminal() {
                    return app_error(
                        &args,
                        outcome::NOT_EXECUTED,
                        "recovery_cancelled",
                        "a terminal is required to approve the bounded recovery request",
                    );
                }
                eprint!("Send this recovery request? [y/N] ");
                let _ = std::io::stderr().flush();
                if !tty::read_line_cooked()
                    .is_some_and(|value| matches!(value.as_str(), "y" | "yes"))
                {
                    return app_error(
                        &args,
                        outcome::NOT_EXECUTED,
                        "recovery_cancelled",
                        "best-effort recovery request was not sent",
                    );
                }
                related_run_id = Some(id);
                args.prompt = request;
                args.review = true;
                args.fresh = true;
            }
            Err(error) => {
                return app_error(&args, outcome::NOT_EXECUTED, "recovery_unavailable", &error)
            }
        }
    }
    if matches!(
        args.subcommand.as_deref(),
        Some(
            "history"
                | "config"
                | "context"
                | "telemetry"
                | "feedback"
                | "doctor"
                | "undo"
                | "restore"
                | "recovery"
        )
    ) {
        return management(
            &args,
            &config,
            &telemetry_policy,
            integration.as_ref(),
            approved_history,
        );
    }
    let stdin = match input::Spool::read(config.stdin_max_bytes) {
        Ok(v) => v,
        Err(e) => return app_error(&args, outcome::USAGE, "input_error", &e),
    };
    let request = if !args.prompt.trim().is_empty() {
        args.prompt.clone()
    } else {
        eprint!("uhm› What result do you need? ");
        let _ = std::io::stderr().flush();
        match tty::read_line_cooked() {
            Some(v) if !v.trim().is_empty() => v,
            _ => {
                return app_error(
                    &args,
                    outcome::USAGE,
                    "input_error",
                    "an intent is required (piped bytes are input, not the intent)",
                )
            }
        }
    };
    let alias = config
        .aliases
        .iter()
        .any(|(name, _)| name.trim() == request.trim());
    let route = match args.subcommand.as_deref() {
        Some("run") => "run",
        Some("ask") => "ask",
        Some("explain") => "explain",
        Some("repair") => "repair",
        Some("recover") => "recover",
        _ => "auto",
    };
    let request_class = capabilities::RequestClass {
        route: route.into(),
        stdin_present: stdin.is_piped(),
        local_input: args.local_input,
        input_format: args.input_format.clone(),
        follow_up: if matches!(route, "repair" | "recover") {
            route.into()
        } else {
            "none".into()
        },
        runtime_available: runtime::inventory().available,
    };
    let selection = match model_selection::resolve(&config, &request_class) {
        Ok(value) => value,
        Err(error) => {
            return app_error(&args, outcome::UNAVAILABLE, "selection_unavailable", &error)
        }
    };
    let key = match secret::resolve_key(selection.initial.provider) {
        Ok(v) => v,
        Err(_) if alias || preset_action.is_some() => String::new(),
        Err(e) => return app_error(&args, outcome::CONFIG, "credential_error", &e),
    };
    if args.verbose && !key.is_empty() {
        eprintln!("uhm: using key {}", secret::mask(&key));
    }
    let api = api::ApiConfig {
        provider: selection.initial.provider,
        model: selection.initial.model.clone(),
        key,
        max_tokens: config.max_completion_tokens,
        reasoning_effort: config.reasoning_effort.clone(),
        request_max_bytes: config.request_max_bytes,
        response_max_bytes: config.response_max_bytes,
        alternate: selection
            .alternate
            .as_ref()
            .map(|alternate| api::ApiCandidate {
                provider: alternate.provider,
                model: alternate.model.clone(),
                key: secret::resolve_key(alternate.provider).ok(),
                resolved_fingerprint: selection.alternate_fingerprint.clone(),
                resolved_model: selection.alternate_resolved_model.clone(),
            }),
        fallback_on: selection.fallback_on.clone(),
        selection_mode: selection.mode,
        permitted_action_types: selection.permitted_action_types.clone(),
        resolved_fingerprint: selection.resolved_fingerprint.clone(),
        resolved_model: selection.resolved_model.clone(),
    };
    let mut interaction = telemetry::Interaction::new(
        if route == "recover" { "run" } else { route },
        std::io::stderr().is_terminal(),
        telemetry_policy.enabled,
    );
    let code = command::handle(
        &args,
        &config,
        &api,
        &request,
        route,
        &stdin,
        disclosure_marker,
        &mut interaction,
        preset_action,
        related_run_id.as_deref(),
        integration.as_ref(),
        approved_history,
    );
    let _ = std::io::stdout().flush();
    telemetry::complete(&config, &telemetry_policy, interaction);
    code
}

fn integration_management(
    args: &args::Args,
    config: &config::Config,
    policy: &telemetry::Policy,
) -> Option<i32> {
    let verb = args.subcommand.as_deref()?;
    let words = args.operands.iter().map(String::as_str).collect::<Vec<_>>();
    let fail = |name: &str, message: &str| app_error(args, outcome::CONFIG, name, message);
    match verb {
        "shell-control-open" => Some(match words.as_slice() {
            [] => match (
                args.integration_shell
                    .as_deref()
                    .map(shell_integration::ShellFamily::parse),
                args.parent_cwd.as_deref(),
                args.parent_status,
            ) {
                (Some(Ok(shell)), Some(cwd), Some(status)) => {
                    match shell_integration::open(config, shell, cwd, status) {
                        Ok((dir, nonce)) => {
                            println!("{}\t{}", dir.display(), nonce);
                            0
                        }
                        Err(e) => fail("integration_error", &e),
                    }
                }
                _ => app_error(
                    args,
                    outcome::USAGE,
                    "usage_error",
                    "invalid internal shell-control open invocation",
                ),
            },
            _ => app_error(
                args,
                outcome::USAGE,
                "usage_error",
                "invalid internal shell-control open invocation",
            ),
        }),
        "shell-validate" => Some(match words.as_slice() {
            [] => match (
                args.integration_shell.as_deref(),
                args.control_dir.as_deref(),
                args.control_nonce.as_deref(),
            ) {
                (Some(shell), Some(dir), Some(nonce)) => {
                    match shell_integration::ShellFamily::parse(shell).and_then(|shell| {
                        shell_integration::validate_response(
                            config,
                            std::path::Path::new(dir),
                            nonce,
                            shell,
                        )
                        .and_then(|response| shell_integration::render(&response.action, shell))
                    }) {
                        Ok(code) => {
                            println!("{}", code);
                            0
                        }
                        Err(e) => fail("integration_validation_error", &e),
                    }
                }
                _ => app_error(
                    args,
                    outcome::USAGE,
                    "usage_error",
                    "invalid internal shell-validation invocation",
                ),
            },
            _ => app_error(
                args,
                outcome::USAGE,
                "usage_error",
                "invalid internal shell-validation invocation",
            ),
        }),
        "shell-clean" => Some(
            match (
                words.as_slice(),
                args.control_dir.as_deref(),
                args.control_nonce.as_deref(),
            ) {
                ([], Some(dir), Some(nonce)) => {
                    match shell_integration::clean(config, std::path::Path::new(dir), nonce) {
                        Ok(()) => 0,
                        Err(e) => fail("integration_cleanup_error", &e),
                    }
                }
                _ => app_error(
                    args,
                    outcome::USAGE,
                    "usage_error",
                    "invalid internal shell-clean invocation",
                ),
            },
        ),
        "shell-history-enabled" => Some(if config.shell_context.last_history_entry {
            0
        } else {
            1
        }),
        "shell-ack" => Some(match words.as_slice() {
            [status] if matches!(*status, "applied" | "failed") => {
                match (args.control_dir.as_deref(), args.control_nonce.as_deref()) {
                    (Some(dir), Some(nonce)) => {
                        match shell_integration::load(config, std::path::Path::new(dir), nonce)
                            .and_then(|session| {
                                shell_integration::validate_response(
                                    config,
                                    std::path::Path::new(dir),
                                    nonce,
                                    session.shell(),
                                )
                            }) {
                            Ok(response) => {
                                telemetry::ack_parent(config, policy, &response.run_id, status);
                                let _ = history::record_parent_ack(
                                    &config.paths.data_dir,
                                    &config.history,
                                    &response.run_id,
                                    status,
                                );
                                0
                            }
                            Err(e) => fail("integration_ack_error", &e),
                        }
                    }
                    _ => app_error(
                        args,
                        outcome::USAGE,
                        "usage_error",
                        "invalid internal shell acknowledgement",
                    ),
                }
            }
            _ => app_error(
                args,
                outcome::USAGE,
                "usage_error",
                "invalid internal shell acknowledgement",
            ),
        }),
        _ => None,
    }
}

fn management(
    args: &args::Args,
    config: &config::Config,
    telemetry_policy: &telemetry::Policy,
    integration: Option<&shell_integration::Session>,
    approved_history: Option<&str>,
) -> i32 {
    match args.subcommand.as_deref().unwrap_or("") {
        "undo" | "restore" => {
            let verb = args.subcommand.as_deref().unwrap_or("undo");
            let words = args.operands.iter().map(String::as_str).collect::<Vec<_>>();
            let Some(selected) = words.first().copied() else {
                return app_error(
                    args,
                    outcome::USAGE,
                    "usage_error",
                    if verb == "undo" {
                        "usage: uhm undo <run-id|last> [--review]"
                    } else {
                        "usage: uhm restore <run-id|last> --force"
                    },
                );
            };
            let forced = verb == "restore";
            let literal_force = args.force || words.iter().skip(1).any(|word| *word == "--force");
            if forced && !literal_force {
                return app_error(
                    args,
                    outcome::USAGE,
                    "usage_error",
                    "forced restore requires the literal --force flag",
                );
            }
            if !forced && (args.force || words.iter().skip(1).any(|word| *word == "--force")) {
                return app_error(args, outcome::USAGE, "usage_error", "--force cannot convert a conflicted operation into verified undo; use `uhm restore <run-id> --force`");
            }
            let preview = match recovery::preview_restore(
                &config.paths.data_dir,
                selected,
                &config.recovery,
                forced,
            ) {
                Ok(value) => value,
                Err(error) => {
                    return app_error(args, outcome::NOT_EXECUTED, "recovery_unavailable", &error)
                }
            };
            if args.json {
                if !forced {
                    println!(
                        "{}",
                        serde_json::json!({"namespace":"uhm","outcome":"restore_preview","exit_code":outcome::NOT_EXECUTED,"data":preview})
                    );
                }
            } else {
                eprintln!(
                    "{} {} from run {}",
                    if forced {
                        "Forced restore"
                    } else {
                        "Verified undo"
                    },
                    if forced {
                        "will overwrite supported destinations with retained evidence"
                    } else {
                        "requires every current postimage to match"
                    },
                    preview.run_id
                );
                for item in &preview.items {
                    eprintln!(
                        "  {} · {} · {} snapshot bytes{}",
                        item.destination.display(),
                        item.operation,
                        item.snapshot_bytes,
                        item.conflict
                            .as_ref()
                            .map(|value| format!(" · CONFLICT: {value}"))
                            .unwrap_or_default()
                    );
                }
                eprintln!("{}", preview.concurrent_writer_warning);
                if forced {
                    eprintln!(
                        "This outcome will be recorded as forced_restore, never verified undo."
                    );
                }
            }
            if !forced {
                if args.json || !std::io::stderr().is_terminal() {
                    return outcome::NOT_EXECUTED;
                }
                eprint!("Restore every listed item? [y/N] ");
                let _ = std::io::stderr().flush();
                if !tty::read_line_cooked()
                    .is_some_and(|value| matches!(value.as_str(), "y" | "yes"))
                {
                    return app_error(
                        args,
                        outcome::NOT_EXECUTED,
                        "undo_cancelled",
                        "verified undo was not run",
                    );
                }
            }
            let operation_run = history::run_id();
            let route = if forced { "restore" } else { "undo" };
            let _ = history::record_request(
                &config.paths.data_dir,
                &config.history,
                &operation_run,
                route,
                route,
                "minimal",
                "local recovery operation",
                Some(&preview.run_id),
            );
            let _ = history::record_recovery_event(
                &config.paths.data_dir,
                &config.history,
                &operation_run,
                route,
                "minimal",
                history::EventKind::UndoStarted,
                "undo_in_progress",
                None,
                preview.items.len(),
                Some(&preview.run_id),
            );
            match recovery::restore(
                &config.paths.data_dir,
                &preview.run_id,
                &operation_run,
                &config.recovery,
                forced,
            ) {
                Ok(report) => {
                    for _ in 0..report.restored.saturating_add(report.removed) {
                        let _ = history::record_recovery_event(
                            &config.paths.data_dir,
                            &config.history,
                            &operation_run,
                            route,
                            "minimal",
                            history::EventKind::UndoItemFinished,
                            if forced { "forced_restore" } else { "restored" },
                            None,
                            1,
                            Some(&preview.run_id),
                        );
                    }
                    let kind = if forced {
                        history::EventKind::ForcedRestoreFinished
                    } else {
                        history::EventKind::UndoFinished
                    };
                    let _ = history::record_recovery_event(
                        &config.paths.data_dir,
                        &config.history,
                        &operation_run,
                        route,
                        "minimal",
                        kind,
                        &report.outcome,
                        None,
                        report.restored + report.removed,
                        Some(&preview.run_id),
                    );
                    if args.json {
                        println!(
                            "{}",
                            serde_json::json!({"namespace":"uhm","outcome":report.outcome,"exit_code":0,"data":report})
                        );
                    } else {
                        println!(
                            "{}: {} file(s) restored, {} created file(s) removed",
                            report.outcome, report.restored, report.removed
                        );
                    }
                    0
                }
                Err(error) => app_error(
                    args,
                    outcome::NOT_EXECUTED,
                    if forced {
                        "forced_restore_failed"
                    } else {
                        "undo_conflicted"
                    },
                    &error,
                ),
            }
        }
        "recovery" => {
            let words = args.operands.iter().map(String::as_str).collect::<Vec<_>>();
            let op = words.first().copied().unwrap_or("status");
            match op {
                "on" if words.len() == 1 => {
                    if !config.history.enabled {
                        return app_error(args, outcome::CONFIG, "recovery_history_disabled", "recovery needs the metadata journal for durable linkage; enable history first");
                    }
                    eprintln!("Recovery duplicates eligible managed file preimages under {}. Supported classes: owned single-link regular files without ACLs/xattrs, up to {} bytes each. Retention: {} days and {} total bytes. Snapshots never enter telemetry or OpenAI requests. Disable with `uhm recovery off`; remove retained snapshots with `uhm recovery prune`.", config.paths.data_dir.join("runs").display(), config.recovery.max_file_bytes, config.recovery.max_age_days, config.recovery.max_total_bytes);
                    match recovery::enable(&config.paths.data_dir) {
                        Ok(()) => { println!("recovery snapshot capture on"); 0 }
                        Err(error) => app_error(args, outcome::CONFIG, "recovery_error", &error),
                    }
                }
                "off" => match recovery::disable(&config.paths.data_dir) {
                    Ok(()) => {
                        if words.contains(&"--prune") {
                            match recovery::prune(&config.paths.data_dir, &config.recovery, false, true) {
                                Ok(report) => {
                                    for run in &report.expired_runs {
                                        let _ = history::record_recovery_event(&config.paths.data_dir, &config.history, run, "recovery", "minimal", history::EventKind::RecoveryExpired, "expired", Some("retained snapshots were explicitly pruned"), 0, None);
                                    }
                                    println!("recovery off; pruned {} snapshots ({} bytes)", report.snapshots_removed, report.bytes_removed)
                                },
                                Err(error) => return app_error(args, outcome::CONFIG, "recovery_prune_error", &error),
                            }
                        } else { println!("recovery off; retained snapshots remain until expiry (use `uhm recovery prune` to remove them now)"); }
                        0
                    }
                    Err(error) => app_error(args, outcome::CONFIG, "recovery_error", &error),
                },
                "status" | "" => {
                    match recovery::status(&config.paths.data_dir, words.get(1).copied(), &config.recovery) {
                        Ok(report) => { if args.json { println!("{}", serde_json::to_string(&report).unwrap()); } else { println!("recovery {} · state {}\n{} manifests · {} snapshots · {} bytes · {} pinned\nlimits: {} days / {} total bytes / {} bytes per file\n{}{}", if report.enabled { "enabled" } else { "disabled" }, report.state, report.manifests, report.snapshots, report.snapshot_bytes, report.pinned, report.max_age_days, report.max_total_bytes, report.max_file_bytes, report.reason, report.run_id.map(|id| format!("\nrun: {id}")).unwrap_or_default()); } 0 }
                        Err(error) => app_error(args, outcome::CONFIG, "recovery_error", &error),
                    }
                }
                "prune" => {
                    let dry = args.dry_run || words.contains(&"--dry-run");
                    match recovery::prune(&config.paths.data_dir, &config.recovery, dry, false) {
                        Ok(report) => {
                            if !dry {
                                for run in &report.expired_runs {
                                    let _ = history::record_recovery_event(&config.paths.data_dir, &config.history, run, "recovery", "minimal", history::EventKind::RecoveryExpired, "expired", Some("retained snapshots were explicitly pruned"), 0, None);
                                }
                            }
                            if args.json { println!("{}", serde_json::to_string(&report).unwrap()); } else { println!("{} {} snapshots ({} bytes); {} pinned retained", if dry { "would prune" } else { "pruned" }, report.snapshots_removed, report.bytes_removed, report.retained_pinned); }
                            0
                        }
                        Err(error) => app_error(args, outcome::CONFIG, "recovery_prune_error", &error),
                    }
                }
                "pin" | "unpin" if words.len() == 2 => match recovery::pin(&config.paths.data_dir, words[1], &config.recovery, op == "pin") {
                    Ok(run) => { println!("recovery snapshot {}: {}", if op == "pin" { "pinned" } else { "unpinned" }, run); 0 }
                    Err(error) => app_error(args, outcome::CONFIG, "recovery_error", &error),
                },
                "resume" if words.len() == 2 => {
                    if !std::io::stderr().is_terminal() {
                        return app_error(args, outcome::NOT_EXECUTED, "recovery_resume_cancelled", "a terminal is required to review a partial managed commit resume");
                    }
                    eprintln!("Resume will commit only staging outputs whose destination preimage and staged hashes still match the interrupted manifest. The multi-file set is not transactional.");
                    eprint!("Resume partial commit {}? [y/N] ", words[1]);
                    let _ = std::io::stderr().flush();
                    if !tty::read_line_cooked().is_some_and(|value| matches!(value.as_str(), "y" | "yes")) { return outcome::NOT_EXECUTED; }
                    match recovery::resume_commit(&config.paths.data_dir, words[1], &config.recovery) {
                        Ok(run) => { println!("managed commit resumed and verified: {run}"); 0 }
                        Err(error) => app_error(args, outcome::NOT_EXECUTED, "recovery_resume_failed", &error),
                    }
                }
                _ => app_error(args, outcome::USAGE, "usage_error", "usage: uhm recovery on|off [--prune]|status [<run-id|last>]|prune [--dry-run]|pin|unpin <run-id|last>|resume <run-id>"),
            }
        }
        "config" => {
            let op = args.operands.first().map(String::as_str).unwrap_or("show");
            if op == "check" {
                println!("config OK: {}", config.paths.config_file.display());
                return 0;
            }
            if op != "show" && !op.is_empty() {
                return app_error(
                    args,
                    outcome::USAGE,
                    "usage_error",
                    "usage: uhm config [show|check]",
                );
            }
            if args.json {
                println!(
                    "{}",
                    serde_json::json!({"namespace":"uhm","outcome":"config","exit_code":0,"data":{"values":config}})
                )
            } else {
                println!("config: {}", config.paths.config_file.display());
                for (k, v, s) in config.show_lines() {
                    println!("{:<28} {:<24} ({})", k, v, s)
                }
            }
            0
        }
        "history" => {
            let words = args.operands.iter().map(String::as_str).collect::<Vec<_>>();
            let op = words.first().copied().unwrap_or("status");
            match op {
                "status" | "" => match history::status(&config.paths.data_dir, &config.history) {
                    Ok(value) => {
                        let recovery_status =
                            recovery::status(&config.paths.data_dir, None, &config.recovery).ok();
                        if args.json {
                            println!(
                                "{}",
                                serde_json::json!({"history":value,"recovery":recovery_status})
                            )
                        } else {
                            println!("history {} · {} detail\n{} events across {} runs · {} bytes\njournal: {}\nretention: {} events / {} days / {} bytes\noutput capture: {} · path redaction: {}{}",if value.enabled{"enabled"}else{"disabled"},value.detail,value.events,value.runs,value.bytes,value.journal.display(),value.max_records,value.max_age_days,value.max_bytes,if value.capture_output{"on"}else{"off"},if value.redact_paths{"on"}else{"off"},if value.truncated_final_line{"\nwarning: truncated final line detected"}else{""});
                            if let Some(recovery) = recovery_status {
                                println!("recovery snapshots: {} · {} bytes · limits {} days / {} total bytes / {} per file", if recovery.enabled { "on" } else { "off" }, recovery.snapshot_bytes, recovery.max_age_days, recovery.max_total_bytes, recovery.max_file_bytes);
                            }
                        };
                        0
                    }
                    Err(e) => app_error(args, outcome::CONFIG, "history_error", &e),
                },
                "list" => {
                    let mut limit = 20usize;
                    let mut failed = false;
                    let mut route = None;
                    let mut i = 1;
                    while i < words.len() {
                        match words[i] {
                            "--limit" => {
                                i += 1;
                                limit = words.get(i).and_then(|v| v.parse().ok()).unwrap_or(0)
                            }
                            "--failed" => failed = true,
                            "--route" => {
                                i += 1;
                                route = words.get(i).copied()
                            }
                            _ => return app_error(
                                args,
                                outcome::USAGE,
                                "usage_error",
                                "usage: uhm history list [--limit N] [--failed] [--route ROUTE]",
                            ),
                        };
                        i += 1
                    }
                    if limit == 0 || limit > 1000 {
                        return app_error(
                            args,
                            outcome::USAGE,
                            "usage_error",
                            "history list limit must be 1..1000",
                        );
                    }
                    match history::list(&config.paths.data_dir, limit, failed, route) {
                        Ok(rows) => {
                            if args.json {
                                println!("{}", serde_json::to_string(&rows).unwrap())
                            } else {
                                for row in rows {
                                    println!(
                                        "{}  {}  {:<14} {} events{}",
                                        row["run_id"].as_str().unwrap_or("?"),
                                        row["timestamp"].as_u64().unwrap_or(0),
                                        row["route"].as_str().unwrap_or("unknown"),
                                        row["events"].as_u64().unwrap_or(0),
                                        if row["failed"].as_bool() == Some(true) {
                                            "  failed"
                                        } else {
                                            ""
                                        }
                                    )
                                }
                            };
                            0
                        }
                        Err(e) => app_error(args, outcome::CONFIG, "history_error", &e),
                    }
                }
                "show" => {
                    let Some(id) = words.get(1) else {
                        return app_error(
                            args,
                            outcome::USAGE,
                            "usage_error",
                            "usage: uhm history show <run-id|last>",
                        );
                    };
                    match history::events_for(&config.paths.data_dir, id) {
                        Ok(events) => {
                            if args.json {
                                println!("{}", serde_json::to_string_pretty(&events).unwrap())
                            } else {
                                for event in events {
                                    println!(
                                        "#{:<3} {}  {}",
                                        event.sequence,
                                        event.timestamp,
                                        serde_json::to_string(&event).unwrap_or_default()
                                    )
                                }
                            };
                            0
                        }
                        Err(e) => app_error(args, outcome::NOT_EXECUTED, "history_unavailable", &e),
                    }
                }
                "search" => {
                    let needle_text = args
                        .operands
                        .get(1..)
                        .unwrap_or_default()
                        .iter()
                        .map(String::as_str)
                        .filter(|value| *value != "--")
                        .collect::<Vec<_>>()
                        .join(" ");
                    let needle = needle_text.as_str();
                    if needle.is_empty() {
                        return app_error(
                            args,
                            outcome::USAGE,
                            "usage_error",
                            "usage: uhm history search -- <substring>",
                        );
                    }
                    match history::search(&config.paths.data_dir, needle) {
                        Ok(events) => {
                            for event in events {
                                println!(
                                    "{} #{} {}",
                                    event.run_id,
                                    event.sequence,
                                    serde_json::to_string(&event).unwrap_or_default()
                                )
                            }
                            0
                        }
                        Err(e) => app_error(args, outcome::CONFIG, "history_error", &e),
                    }
                }
                "export" => {
                    let include = words.contains(&"--include-content");
                    let output = words
                        .iter()
                        .position(|v| *v == "--output")
                        .and_then(|i| words.get(i + 1))
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|| {
                            std::env::current_dir()
                                .unwrap_or_default()
                                .join("uhm-history.redacted.jsonl")
                        });
                    match history::export(&config.paths.data_dir, &output, include) {
                        Ok(count) => {
                            println!(
                                "exported {} events to {} ({})",
                                count,
                                output.display(),
                                if include {
                                    "content included"
                                } else {
                                    "redacted metadata"
                                }
                            );
                            0
                        }
                        Err(e) => app_error(args, outcome::CONFIG, "history_error", &e),
                    }
                }
                "prune" => {
                    let dry = words.contains(&"--dry-run");
                    let _recovery_guard = match recovery::exclusive_guard(&config.paths.data_dir) {
                        Ok(guard) => guard,
                        Err(error) => {
                            return app_error(args, outcome::CONFIG, "history_error", &error)
                        }
                    };
                    match history::prune(&config.paths.data_dir, &config.history, dry) {
                        Ok((count, bytes)) => {
                            println!(
                                "{} {} events and {} bytes",
                                if dry { "would prune" } else { "pruned" },
                                count,
                                bytes
                            );
                            0
                        }
                        Err(e) => app_error(args, outcome::CONFIG, "history_error", &e),
                    }
                }
                "clear" if words.contains(&"--all") => {
                    let _recovery_guard = match recovery::exclusive_guard(&config.paths.data_dir) {
                        Ok(guard) => guard,
                        Err(error) => {
                            return app_error(args, outcome::CONFIG, "history_error", &error)
                        }
                    };
                    match history::clear(&config.paths.data_dir) {
                        Ok(preserved) => {
                            if args.json {
                                println!(
                                    "{}",
                                    serde_json::json!({"namespace":"uhm","outcome":"history_cleared","recovery_runs_preserved":preserved,"exit_code":0})
                                )
                            } else {
                                println!("history cleared; {preserved} recovery runs preserved")
                            };
                            0
                        }
                        Err(e) => app_error(args, outcome::CONFIG, "history_error", &e),
                    }
                }
                "clear" if words.get(1) == Some(&"--before") && words.len() == 3 => {
                    let cutoff = match parse_utc_date(words[2]) {
                        Ok(value) => value,
                        Err(e) => return app_error(args, outcome::USAGE, "usage_error", &e),
                    };
                    let _recovery_guard = match recovery::exclusive_guard(&config.paths.data_dir) {
                        Ok(guard) => guard,
                        Err(error) => {
                            return app_error(args, outcome::CONFIG, "history_error", &error)
                        }
                    };
                    match history::clear_before(&config.paths.data_dir, cutoff) {
                        Ok(count) => {
                            println!("cleared {} events before {}", count, words[2]);
                            0
                        }
                        Err(e) => app_error(args, outcome::CONFIG, "history_error", &e),
                    }
                }
                "clear" => app_error(
                    args,
                    outcome::USAGE,
                    "usage_error",
                    "history clear is destructive; use: uhm history clear --all",
                ),
                _ => app_error(
                    args,
                    outcome::USAGE,
                    "usage_error",
                    "usage: uhm history [list|show|search|replay|export|prune|clear|status]",
                ),
            }
        }
        "context" => {
            let words = args.operands.iter().map(String::as_str).collect::<Vec<_>>();
            let mode_text = match words.as_slice() {
                [] | ["show"] => args.context.as_deref().unwrap_or(&config.context_mode),
                ["show", mode] => mode,
                _ => {
                    return app_error(
                        args,
                        outcome::USAGE,
                        "usage_error",
                        "usage: uhm context show [minimal|standard|full]",
                    )
                }
            };
            let mode = match context::Mode::parse(mode_text) {
                Ok(v) => v,
                Err(e) => return app_error(args, outcome::USAGE, "usage_error", &e),
            };
            let shell = match command::target_shell(config, args.shell.as_deref()) {
                Ok(value) => value,
                Err(error) => return app_error(args, outcome::USAGE, "usage_error", &error),
            };
            let mut snapshot = context::gather(mode, &shell, config.context_timeout_ms);
            if let Some(session) = integration {
                context::add_shell_invocation(&mut snapshot, session, approved_history);
            }
            let value = serde_json::json!({"prompt":"<user intent>","stdin":{"present":"<depends on invocation>"},"context":snapshot,"disclosure":context::disclosure_payload()});
            println!("{}", serde_json::to_string_pretty(&value).unwrap());
            0
        }
        "telemetry" => {
            let op = args
                .operands
                .first()
                .map(String::as_str)
                .unwrap_or("status");
            match op {
                "status" | "" => {
                    let value = serde_json::json!({
                        "enabled": telemetry_policy.enabled,
                        "reason": telemetry_policy.reason,
                        "endpoint": telemetry::ENDPOINT,
                        "queued": telemetry::queue_count(config),
                        "schema_version": telemetry::SCHEMA_VERSION,
                        "lossy": true
                    });
                    if args.json {
                        println!("{}", value);
                    } else {
                        println!(
                            "telemetry {} ({})\nendpoint: {}\nqueued: {} (best-effort and lossy)",
                            if telemetry_policy.enabled {
                                "on"
                            } else {
                                "off"
                            },
                            telemetry_policy.reason,
                            telemetry::ENDPOINT,
                            telemetry::queue_count(config)
                        );
                    }
                    0
                }
                "preview" => {
                    let event = telemetry::preview("auto", std::io::stderr().is_terminal());
                    println!("{}", serde_json::to_string_pretty(&event).unwrap());
                    0
                }
                "off" => match telemetry::disable(config) {
                    Ok(()) => {
                        println!("telemetry off; queued summaries cleared");
                        0
                    }
                    Err(e) => app_error(args, outcome::CONFIG, "telemetry_error", &e),
                },
                "on" => match telemetry::enable(config) {
                    Ok(()) if config.telemetry.enabled => {
                        println!("telemetry on");
                        0
                    }
                    Ok(()) => app_error(
                        args,
                        outcome::CONFIG,
                        "telemetry_config_disabled",
                        "telemetry remains disabled by config.yaml; set telemetry.enabled: true",
                    ),
                    Err(e) => app_error(args, outcome::CONFIG, "telemetry_error", &e),
                },
                _ => app_error(
                    args,
                    outcome::USAGE,
                    "usage_error",
                    "usage: uhm telemetry [status|preview|on|off]",
                ),
            }
        }
        "feedback" => {
            let words = args.operands.iter().map(String::as_str).collect::<Vec<_>>();
            let feedback = words.first().copied().unwrap_or("");
            if !matches!(feedback, "good" | "bad") {
                return app_error(
                    args,
                    outcome::USAGE,
                    "usage_error",
                    "usage: uhm feedback good|bad [run-id]",
                );
            }
            match history::set_feedback(&config.paths.data_dir, feedback, words.get(1).copied()) {
                Ok(receipt) => {
                    if args.json {
                        println!(
                            "{}",
                            serde_json::json!({"namespace":"uhm","outcome":"feedback_recorded","exit_code":0,"feedback":feedback})
                        );
                    } else {
                        println!(
                            "feedback recorded: {} (no text or identifier sent)",
                            feedback
                        );
                    }
                    let _ = std::io::stdout().flush();
                    telemetry::feedback(config, telemetry_policy, &receipt);
                    0
                }
                Err(e) => app_error(args, outcome::NOT_EXECUTED, "feedback_unavailable", &e),
            }
        }
        "doctor" => {
            let network = args
                .prompt
                .split_whitespace()
                .any(|value| value == "network");
            let all_providers = args.prompt.split_whitespace().any(|value| value == "all");
            let report = doctor::gather(config, network, all_providers, telemetry_policy);
            let code = if doctor::healthy(&report) {
                0
            } else {
                outcome::CONFIG
            };
            if args.json {
                println!(
                    "{}",
                    serde_json::json!({"namespace":"uhm","outcome":"doctor","exit_code":code,"data":report})
                );
            } else {
                doctor::render(&report);
            }
            code
        }
        _ => outcome::USAGE,
    }
}
fn parse_utc_date(value: &str) -> Result<u64, String> {
    let fields = value
        .split('-')
        .map(str::parse::<i64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "--before must be YYYY-MM-DD")?;
    let max_day = if fields.len() == 3 {
        match fields[1] {
            2 if fields[0] % 400 == 0 || (fields[0] % 4 == 0 && fields[0] % 100 != 0) => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        }
    } else {
        0
    };
    if fields.len() != 3
        || !(1970..=9999).contains(&fields[0])
        || !(1..=12).contains(&fields[1])
        || !(1..=max_day).contains(&fields[2])
    {
        return Err("--before must be a valid YYYY-MM-DD date".into());
    }
    let mut year = fields[0];
    let month = fields[1];
    let day = fields[2];
    year -= i64::from(month <= 2);
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let shifted = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * shifted + 2) / 5 + day - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    u64::try_from(days.saturating_mul(86_400)).map_err(|_| "--before date is out of range".into())
}
fn app_error(args: &args::Args, code: i32, name: &str, message: &str) -> i32 {
    if args.json {
        println!(
            "{}",
            outcome::Outcome {
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
        eprintln!("{}: {}", ansi::critical("uhm"), message)
    }
    code
}
fn print_help() {
    println!("uhm — say what you need; get the result\n\nUsage:\n  uhm [options] <intent>\n  uhm run|ask|explain [options] <intent>\n  uhm repair <run-id|last> [feedback]\n  uhm recover <run-id|last> [guidance]\n  uhm undo <run-id|last> [--review]\n  uhm restore <run-id|last> --force\n  uhm recovery on|off|status|prune|pin|unpin|resume\n  uhm shell-init bash|zsh|fish\n  uhm context show [minimal|standard|full]\n  uhm telemetry [status|preview|on|off]\n  uhm feedback good|bad [run-id]\n  uhm history [list|show|search|replay|export|prune|clear|status]\n  uhm config [show|check]\n  uhm doctor [all] [network]\n\nExecution:\n  ordinary actions run and return their result\n  non-TTY jobs that may mutate existing state or file metadata pause with status 11; rerun with --force\n  --review    review with run/revise/edit/copy/cancel controls\n  --dry-run   return the exact proposal without executing\n  --force     authorize a non-interactive mutation and proceed after warnings\n  --recoverable capture bounded managed-file preimages for this job\n  --context <minimal|standard|full>\n  --local-input keep piped bytes on-device for a generated program\n  --input-format <label> describe local-only input without sending its content\n  --retain-program keep the private program workspace for debugging\n  --plain     cooked ASCII-safe UI with no styling or animation\n  --no-motion disable animation while retaining color and Unicode\n  --no-telemetry disable telemetry for this invocation\n  --json      machine-readable product outcomes (child stdout remains result data)\n\nOptions:\n      --provider <openai|cerebras>\n  -m, --model <id>\n      --shell <auto|bash|zsh|fish|pwsh>\n      --no-stream\n      --fresh\n  -v, --verbose\n  -h, --help\n  -V, --version\n\nEverything after the first intent word is user text. The -- separator is only needed when the intent itself starts with '-'.")
}
