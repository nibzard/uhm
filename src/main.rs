//! uhm — say what you need; get the result.

mod action;
mod api;
mod args;
mod cache;
mod clock;
mod command;
mod config;
mod context;
mod dirs;
mod doctor;
mod first_run;
mod history;
mod http;
mod input;
mod outcome;
mod parent_shell;
mod program;
mod prompt;
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
        let words = args.prompt.split_whitespace().collect::<Vec<_>>();
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
    let config = match config::load(args.model.as_deref()) {
        Ok(v) => v,
        Err(e) => return app_error(&args, outcome::CONFIG, "configuration_error", &e),
    };
    let telemetry_policy = telemetry::policy(&config, args.no_telemetry);
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
    let approved_history = if let Some(entry) = args.last_history_entry.as_deref() {
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
    let disclosure_marker = match first_run::ensure(&config, telemetry_policy.enabled) {
        Ok(marker) => marker,
        Err(e) => return app_error(&args, outcome::CONFIG, "notice_error", &e),
    };
    let mut preset_action = None;
    let mut related_run_id = None;
    if args.subcommand.as_deref() == Some("history") {
        let words = args.prompt.split_whitespace().collect::<Vec<_>>();
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
        let mut parts = args.prompt.splitn(2, " -- ");
        let selected = parts.next().unwrap_or("last").trim();
        let feedback = parts.next().map(str::trim).filter(|v| !v.is_empty());
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
                eprintln!("uhm: repair will send only the retained original intent, typed proposal, coarse outcome, and supplied feedback for run {}", id);
                related_run_id = Some(id);
                args.prompt = seed;
                args.review = true;
            }
            Err(e) => return app_error(&args, outcome::NOT_EXECUTED, "repair_unavailable", &e),
        }
    }
    if matches!(
        args.subcommand.as_deref(),
        Some("history" | "config" | "context" | "telemetry" | "feedback" | "doctor")
    ) {
        return management(&args, &config, &telemetry_policy);
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
    let key = match secret::resolve_key() {
        Ok(v) => v,
        Err(_) if alias || preset_action.is_some() => String::new(),
        Err(e) => return app_error(&args, outcome::CONFIG, "credential_error", &e),
    };
    if args.verbose && !key.is_empty() {
        eprintln!("uhm: using key {}", secret::mask(&key));
    }
    let api = api::ApiConfig {
        model: config.model.clone(),
        key,
        max_tokens: config.max_completion_tokens,
        reasoning_effort: config.reasoning_effort.clone(),
        request_max_bytes: config.request_max_bytes,
        response_max_bytes: config.response_max_bytes,
    };
    let route = match args.subcommand.as_deref() {
        Some("run") => "run",
        Some("ask") => "ask",
        Some("explain") => "explain",
        Some("repair") => "repair",
        _ => "auto",
    };
    let mut interaction = telemetry::Interaction::new(
        route,
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
    let words = args.prompt.split_whitespace().collect::<Vec<_>>();
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
) -> i32 {
    match args.subcommand.as_deref().unwrap_or("") {
        "config" => {
            let op = args.prompt.split_whitespace().next().unwrap_or("show");
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
            let words = args.prompt.split_whitespace().collect::<Vec<_>>();
            let op = words.first().copied().unwrap_or("status");
            match op {
                "status" | "" => match history::status(&config.paths.data_dir, &config.history) {
                    Ok(value) => {
                        if args.json {
                            println!("{}", serde_json::to_string(&value).unwrap())
                        } else {
                            println!("history {} · {} detail\n{} events across {} runs · {} bytes\njournal: {}\nretention: {} events / {} days / {} bytes\noutput capture: {} · path redaction: {}{}",if value.enabled{"enabled"}else{"disabled"},value.detail,value.events,value.runs,value.bytes,value.journal.display(),value.max_records,value.max_age_days,value.max_bytes,if value.capture_output{"on"}else{"off"},if value.redact_paths{"on"}else{"off"},if value.truncated_final_line{"\nwarning: truncated final line detected"}else{""})
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
                    let needle = args
                        .prompt
                        .strip_prefix("search -- ")
                        .or_else(|| args.prompt.strip_prefix("search "))
                        .unwrap_or("");
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
                "clear" if words.contains(&"--all") => match history::clear(&config.paths.data_dir)
                {
                    Ok(()) => {
                        if args.json {
                            println!(
                                "{}",
                                serde_json::json!({"namespace":"uhm","outcome":"history_cleared","exit_code":0})
                            )
                        } else {
                            println!("history cleared")
                        };
                        0
                    }
                    Err(e) => app_error(args, outcome::CONFIG, "history_error", &e),
                },
                "clear" if words.get(1) == Some(&"--before") && words.len() == 3 => {
                    let cutoff = match parse_utc_date(words[2]) {
                        Ok(value) => value,
                        Err(e) => return app_error(args, outcome::USAGE, "usage_error", &e),
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
            let words = args.prompt.split_whitespace().collect::<Vec<_>>();
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
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
            let snapshot = context::gather(mode, &shell, config.context_timeout_ms);
            let value = serde_json::json!({"prompt":"<user intent>","stdin":{"present":"<depends on invocation>"},"context":snapshot,"disclosure":context::disclosure_payload()});
            println!("{}", serde_json::to_string_pretty(&value).unwrap());
            0
        }
        "telemetry" => {
            let op = args.prompt.split_whitespace().next().unwrap_or("status");
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
            let words = args.prompt.split_whitespace().collect::<Vec<_>>();
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
            let report = doctor::gather(config, network, telemetry_policy);
            let supported = report.supported;
            if args.json {
                println!(
                    "{}",
                    serde_json::json!({"namespace":"uhm","outcome":"doctor","exit_code":if supported {0} else {outcome::CONFIG},"data":report})
                );
            } else {
                doctor::render(&report);
            }
            if supported {
                0
            } else {
                outcome::CONFIG
            }
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
    println!("uhm — say what you need; get the result\n\nUsage:\n  uhm [options] -- <intent>\n  uhm run|ask|explain [options] -- <intent>\n  uhm repair <run-id|last> [-- <feedback>]\n  uhm shell-init bash|zsh|fish\n  uhm context show [minimal|standard|full]\n  uhm telemetry [status|preview|on|off]\n  uhm feedback good|bad [run-id]\n  uhm history [list|show|search|replay|export|prune|clear|status]\n  uhm config [show|check]\n  uhm doctor [network]\n\nExecution:\n  ordinary actions run and return their result\n  --review    review with run/revise/edit/copy/cancel controls\n  --dry-run   return the exact proposal without executing\n  --force     proceed after warnings without confirmation\n  --context <minimal|standard|full>\n  --local-input keep piped bytes on-device for a generated program\n  --input-format <label> describe local-only input without sending its content\n  --retain-program keep the private program workspace for debugging\n  --plain     cooked ASCII-safe UI with no styling or animation\n  --no-motion disable animation while retaining color and Unicode\n  --no-telemetry disable telemetry for this invocation\n  --json      machine-readable product outcomes (child stdout remains result data)\n\nOptions:\n  -m, --model <id>\n      --shell <auto|bash|zsh|fish|pwsh>\n      --no-stream\n      --fresh\n  -v, --verbose\n  -h, --help\n  -V, --version\n\nEverything after the first intent word is user text. Put -- before intent that starts with '-'.")
}
