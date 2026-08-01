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
mod history;
mod http;
mod input;
mod outcome;
mod parent_shell;
mod prompt;
mod render;
mod safety;
mod secret;
mod shell;
mod sse;
mod tty;

use render::ansi;
use std::io::Write;
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
    if matches!(
        args.subcommand.as_deref(),
        Some("history" | "config" | "context" | "doctor")
    ) {
        return management(&args, &config);
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
    command::handle(&args, &config, &api, &request, route, &stdin)
}

fn management(args: &args::Args, config: &config::Config) -> i32 {
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
        "doctor" => {
            println!("config      OK  {}", config.paths.config_file.display());
            println!("data dir    OK  {}", config.paths.data_dir.display());
            println!("Responses   OK  {}", api::ENDPOINT);
            println!(
                "API key     {}",
                if secret::resolve_key().is_ok() {
                    "OK"
                } else {
                    "missing"
                }
            );
            0
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
        eprintln!("uhm: {}", message)
    }
    code
}
fn print_help() {
    println!("uhm — say what you need; get the result\n\nUsage:\n  uhm [options] -- <intent>\n  uhm run|ask|explain [options] -- <intent>\n  uhm context show [minimal|standard|full]\n  uhm history [status|clear]\n  uhm config [show|check]\n  uhm doctor\n\nExecution:\n  ordinary actions run and return their result\n  --review    review with run/revise/edit/copy/cancel controls\n  --dry-run   return the exact proposal without executing\n  --force     proceed after warnings without confirmation\n  --context <minimal|standard|full>\n  --plain     disable styling and animation\n  --json      machine-readable product outcomes (child stdout remains result data)\n\nOptions:\n  -m, --model <id>\n      --shell <auto|bash|zsh|fish|pwsh>\n      --no-stream\n      --fresh\n  -v, --verbose\n  -h, --help\n  -V, --version\n\nEverything after the first intent word is user text. Put -- before intent that starts with '-'.")
}
