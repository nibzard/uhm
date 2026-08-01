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
    let args = match args::parse_from(argv) {
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
    let config = match config::load(args.model.as_deref()) {
        Ok(v) => v,
        Err(e) => return app_error(&args, outcome::CONFIG, "configuration_error", &e),
    };
    let telemetry_policy = telemetry::policy(&config, args.no_telemetry);
    let disclosure_marker = match first_run::ensure(&config, telemetry_policy.enabled) {
        Ok(marker) => marker,
        Err(e) => return app_error(&args, outcome::CONFIG, "notice_error", &e),
    };
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
        Err(_) if alias => String::new(),
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
    );
    let _ = std::io::stdout().flush();
    telemetry::complete(&config, &telemetry_policy, interaction);
    code
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
            let op = args.prompt.split_whitespace().next().unwrap_or("status");
            match op {
                "status" | "" => {
                    let records =
                        history::recent(&config.paths.data_dir, config.history.max_records);
                    let value = serde_json::json!({"enabled":config.history.enabled,"records":records.len(),"max_records":config.history.max_records,"max_age_days":config.history.max_age_days,"path":config.paths.data_dir.join("history.jsonl")});
                    if args.json {
                        println!("{}", value)
                    } else {
                        println!(
                            "history {}: {} metadata receipts (max {}, {} days)",
                            if config.history.enabled {
                                "enabled"
                            } else {
                                "disabled"
                            },
                            records.len(),
                            config.history.max_records,
                            config.history.max_age_days
                        )
                    }
                    0
                }
                "clear" => match history::clear(&config.paths.data_dir) {
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
                _ => app_error(
                    args,
                    outcome::USAGE,
                    "usage_error",
                    "usage: uhm history [status|clear]",
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
            let feedback = args.prompt.split_whitespace().next().unwrap_or("");
            if !matches!(feedback, "good" | "bad") {
                return app_error(
                    args,
                    outcome::USAGE,
                    "usage_error",
                    "usage: uhm feedback good|bad",
                );
            }
            match history::set_latest_feedback(&config.paths.data_dir, feedback) {
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
    println!("uhm — say what you need; get the result\n\nUsage:\n  uhm [options] -- <intent>\n  uhm run|ask|explain [options] -- <intent>\n  uhm context show [minimal|standard|full]\n  uhm telemetry [status|preview|on|off]\n  uhm feedback good|bad\n  uhm history [status|clear]\n  uhm config [show|check]\n  uhm doctor [network]\n\nExecution:\n  ordinary actions run and return their result\n  --review    review with run/revise/edit/copy/cancel controls\n  --dry-run   return the exact proposal without executing\n  --force     proceed after warnings without confirmation\n  --context <minimal|standard|full>\n  --local-input keep piped bytes on-device for a generated program\n  --input-format <label> describe local-only input without sending its content\n  --retain-program keep the private program workspace for debugging\n  --plain     cooked ASCII-safe UI with no styling or animation\n  --no-motion disable animation while retaining color and Unicode\n  --no-telemetry disable telemetry for this invocation\n  --json      machine-readable product outcomes (child stdout remains result data)\n\nOptions:\n  -m, --model <id>\n      --shell <auto|bash|zsh|fish|pwsh>\n      --no-stream\n      --fresh\n  -v, --verbose\n  -h, --help\n  -V, --version\n\nEverything after the first intent word is user text. Put -- before intent that starts with '-'.")
}
