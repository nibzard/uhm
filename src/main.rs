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
mod outcome;
mod prompt;
mod render;
mod safety;
mod secret;
mod shell;
mod sse;
mod tty;

use std::io::{IsTerminal, Read, Write};

use context::Provider as _;
use render::{ansi, markdown, spinner, sync};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    std::process::exit(run(std::env::args().collect()));
}

#[derive(Clone, Copy)]
enum Mode {
    Auto,
    Run,
    Ask,
    Explain,
    Manage,
}

fn resolve_mode(args: &args::Args) -> Mode {
    match args.subcommand.as_deref() {
        Some("run") => Mode::Run,
        Some("ask") => Mode::Ask,
        Some("explain") => Mode::Explain,
        Some("history" | "config" | "context" | "doctor") => Mode::Manage,
        _ => Mode::Auto,
    }
}

fn run(argv: Vec<String>) -> i32 {
    let args = match args::parse_from(argv) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("uhm: {}", error);
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
        Ok(config) => config,
        Err(error) => {
            return application_error(&args, outcome::CONFIG, "configuration_error", &error);
        }
    };
    let mode = resolve_mode(&args);
    if matches!(mode, Mode::Manage) {
        return run_management(
            args.subcommand.as_deref().unwrap_or_default(),
            &args,
            &config,
        );
    }

    let user = match input_text(&args) {
        Ok(text) if !text.trim().is_empty() => text,
        Ok(_) if std::io::stdin().is_terminal() => {
            eprint!(
                "{}",
                if ansi::plain_enabled() {
                    "uhm> "
                } else {
                    "uhm› "
                }
            );
            let _ = std::io::stderr().flush();
            match tty::read_line_cooked() {
                Some(text) if !text.trim().is_empty() => text,
                _ => return outcome::USAGE,
            }
        }
        Ok(_) => {
            eprintln!("uhm: empty input");
            return outcome::USAGE;
        }
        Err(error) => {
            return application_error(&args, outcome::USAGE, "input_error", &error);
        }
    };

    let local_alias = matches!(mode, Mode::Auto | Mode::Run)
        && config
            .aliases
            .iter()
            .any(|(name, _)| name.trim() == user.trim());
    let key = match secret::resolve_key() {
        Ok(key) => key,
        Err(_) if local_alias => String::new(),
        Err(error) => {
            return application_error(&args, outcome::CONFIG, "credential_error", &error);
        }
    };
    if args.verbose && !key.is_empty() {
        eprintln!("uhm: using key {}", secret::mask(&key));
    }
    let api_config = make_api_config(&config, key);
    match mode {
        Mode::Ask => prose_mode(
            &api_config,
            &config,
            &args,
            prompt::answer_system(),
            &user,
            false,
        ),
        Mode::Explain => prose_mode(
            &api_config,
            &config,
            &args,
            prompt::explain_system(),
            &user,
            true,
        ),
        Mode::Run | Mode::Auto => command::handle(
            &args,
            &config,
            &api_config,
            &user,
            matches!(mode, Mode::Run),
        ),
        Mode::Manage => unreachable!(),
    }
}

fn make_api_config(config: &config::Config, key: String) -> api::ApiConfig {
    api::ApiConfig {
        base_url: config.base_url.clone(),
        model: config.model.clone(),
        key,
        max_tokens: config.max_completion_tokens,
        reasoning_effort: config.reasoning_effort.clone(),
    }
}

fn input_text(args: &args::Args) -> Result<String, String> {
    let piped = if !std::io::stdin().is_terminal() {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|error| format!("read stdin: {}", error))?;
        text
    } else {
        String::new()
    };
    Ok(combine_input(&args.prompt, &piped))
}

fn combine_input(prompt: &str, piped: &str) -> String {
    match (prompt.trim().is_empty(), piped.trim().is_empty()) {
        (true, true) => String::new(),
        (false, true) => prompt.to_string(),
        (true, false) => piped.to_string(),
        (false, false) => format!("{}\n\nContext from stdin:\n{}", prompt, piped),
    }
}

fn prose_mode(
    api_config: &api::ApiConfig,
    config: &config::Config,
    args: &args::Args,
    system: &str,
    user: &str,
    render_markdown: bool,
) -> i32 {
    if args.json || args.no_stream || !config.stream || render_markdown {
        let mut spinner = spinner::Spinner::start("thinking");
        let result = api::collect_answer(
            api_config,
            system,
            user,
            None,
            config.stream && !args.no_stream && !args.json,
            |_| {},
        );
        spinner.stop();
        return match result {
            Ok(text) if args.json => {
                println!(
                    "{}",
                    outcome::Outcome {
                        namespace: "uhm",
                        outcome: "answer",
                        exit_code: 0,
                        executed: false,
                        command: None,
                        message: Some(&text),
                    }
                    .json()
                );
                0
            }
            Ok(text) => {
                let safe = ansi::sanitize_untrusted(&text);
                if render_markdown && std::io::stdout().is_terminal() && !ansi::plain_enabled() {
                    print!("{}", sync::wrap(&markdown::render(&safe)));
                } else {
                    print!("{}", safe);
                }
                if !text.ends_with('\n') {
                    println!();
                }
                0
            }
            Err(error) => {
                if args.json {
                    println!(
                        "{}",
                        outcome::Outcome {
                            namespace: "uhm",
                            outcome: "model_error",
                            exit_code: outcome::MODEL,
                            executed: false,
                            command: None,
                            message: Some(&error),
                        }
                        .json()
                    );
                } else {
                    eprintln!("uhm: {}", error);
                }
                outcome::MODEL
            }
        };
    }

    let mut first = true;
    let mut progress = spinner::Spinner::start("thinking");
    let result = api::stream_answer(api_config, system, user, |token| {
        if first {
            first = false;
            progress.stop();
        }
        print!("{}", ansi::sanitize_untrusted(token));
        let _ = std::io::stdout().flush();
    });
    progress.stop();
    println!();
    match result {
        Ok(()) => 0,
        Err(error) => {
            if args.json {
                println!(
                    "{}",
                    outcome::Outcome {
                        namespace: "uhm",
                        outcome: "model_error",
                        exit_code: outcome::MODEL,
                        executed: false,
                        command: None,
                        message: Some(&error),
                    }
                    .json()
                );
            } else {
                eprintln!("uhm: {}", error);
            }
            outcome::MODEL
        }
    }
}

fn application_error(args: &args::Args, code: i32, name: &str, message: &str) -> i32 {
    if args.json {
        println!(
            "{}",
            outcome::Outcome {
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

fn run_management(verb: &str, args: &args::Args, config: &config::Config) -> i32 {
    if args.json {
        return run_management_json(verb, args, config);
    }
    match verb {
        "config" => {
            let operation = args.prompt.split_whitespace().next().unwrap_or("show");
            match operation {
                "check" => {
                    println!("config OK: {}", config.paths.config_file.display());
                    0
                }
                "show" | "" => {
                    println!("config: {}", config.paths.config_file.display());
                    for (key, value, source) in config.show_lines() {
                        println!("{:<24} {:<28} ({})", key, value, source);
                    }
                    0
                }
                _ => {
                    eprintln!("uhm: usage: uhm config [show|check]");
                    outcome::USAGE
                }
            }
        }
        "history" => {
            let count = args
                .prompt
                .split_whitespace()
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(config.history_lines)
                .min(200);
            let entries = history::recent(&config.paths.data_dir, count);
            for entry in entries {
                let command = entry["command"].as_str().unwrap_or("");
                let exit = entry["exit"].as_i64().unwrap_or_default();
                println!(
                    "[exit {}] {}",
                    exit,
                    ansi::sanitize_untrusted_inline(command)
                );
            }
            0
        }
        "context" => {
            if config.context_mode == "request_only" {
                println!("context_mode=request_only\n(no machine context is sent)");
                return 0;
            }
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
            println!(
                "{}",
                context::SystemProvider
                    .gather(&shell, config.include_ls, config.context_timeout_ms)
                    .render()
            );
            0
        }
        "doctor" => {
            println!("config      OK  {}", config.paths.config_file.display());
            println!("data dir    OK  {}", config.paths.data_dir.display());
            println!("cache dir   OK  {}", config.paths.cache_dir.display());
            match secret::resolve_key() {
                Ok(_) => println!("API key     OK"),
                Err(error) => println!("API key     missing ({})", error),
            }
            0
        }
        _ => outcome::USAGE,
    }
}

fn run_management_json(verb: &str, args: &args::Args, config: &config::Config) -> i32 {
    let (outcome_name, data) = match verb {
        "config" => {
            let operation = args.prompt.split_whitespace().next().unwrap_or("show");
            if !matches!(operation, "" | "show" | "check") {
                return application_error(
                    args,
                    outcome::USAGE,
                    "usage_error",
                    "usage: uhm config [show|check]",
                );
            }
            (
                if operation == "check" {
                    "config_valid"
                } else {
                    "config"
                },
                serde_json::json!({
                    "path": config.paths.config_file,
                    "values": config,
                    "sources": config.show_lines().into_iter().map(|(key, _, source)| (key, source)).collect::<std::collections::BTreeMap<_, _>>()
                }),
            )
        }
        "history" => {
            let count = args
                .prompt
                .split_whitespace()
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(config.history_lines)
                .min(200);
            (
                "history",
                serde_json::Value::Array(history::recent(&config.paths.data_dir, count)),
            )
        }
        "context" => {
            let value = if config.context_mode == "request_only" {
                String::new()
            } else {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
                context::SystemProvider
                    .gather(&shell, config.include_ls, config.context_timeout_ms)
                    .render()
            };
            (
                "context",
                serde_json::json!({"mode": config.context_mode, "value": value}),
            )
        }
        "doctor" => (
            "doctor",
            serde_json::json!({
                "config_path": config.paths.config_file,
                "data_dir": config.paths.data_dir,
                "cache_dir": config.paths.cache_dir,
                "api_key": if secret::resolve_key().is_ok() { "configured" } else { "missing" }
            }),
        ),
        _ => {
            return application_error(
                args,
                outcome::USAGE,
                "usage_error",
                "unknown management command",
            )
        }
    };
    println!(
        "{}",
        serde_json::json!({
            "namespace": "uhm",
            "outcome": outcome_name,
            "exit_code": 0,
            "executed": false,
            "data": data
        })
    );
    0
}

fn print_help() {
    println!(
        "uhm — say what you need; get the result\n\n\
Usage:\n  uhm [options] -- <intent>\n  uhm run [options] -- <intent>\n  uhm ask [options] -- <question>\n  uhm explain [options] -- <command>\n  uhm history [n]\n  uhm config [show|check]\n  uhm context\n  uhm doctor\n\n\
Execution:\n  ordinary requests run immediately when no consequential effects are detected\n  --review    always show the proposal and ask before running\n  --dry-run   print the exact command without running it\n  --force     run after warnings without confirmation\n  --plain     disable styling, animation, and terminal control sequences\n  --json      emit a namespaced machine-readable outcome\n\n\
Options:\n  -m, --model <id>\n      --shell <auto|bash|zsh|fish|pwsh>\n      --no-stream\n      --fresh\n  -v, --verbose\n  -h, --help\n  -V, --version\n\n\
Everything after the first intent word is user text. Put -- before intent that starts with '-'."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combines_prompt_and_piped_context_without_trimming_payload() {
        assert_eq!(
            combine_input("summarize", "  document\n"),
            "summarize\n\nContext from stdin:\n  document\n"
        );
    }
}
