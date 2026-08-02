//! Invocation grammar. The first intent word is an intentional parsing boundary:
//! every following token is opaque user text, even when it starts with `-`.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Args {
    pub subcommand: Option<String>,
    pub prompt: String,
    pub model: Option<String>,
    pub shell: Option<String>,
    pub context: Option<String>,
    pub review: bool,
    pub dry_run: bool,
    pub force: bool,
    pub plain: bool,
    pub json: bool,
    pub no_stream: bool,
    pub no_telemetry: bool,
    pub no_motion: bool,
    pub local_input: bool,
    pub input_format: Option<String>,
    pub retain_program: bool,
    pub recoverable: bool,
    pub fresh: bool,
    pub verbose: bool,
    pub help: bool,
    pub version: bool,
    pub control_dir: Option<String>,
    pub control_nonce: Option<String>,
    pub last_history_entry: Option<String>,
    pub integration_shell: Option<String>,
    pub parent_cwd: Option<String>,
    pub parent_status: Option<i32>,
}

impl Args {
    /// True for subcommands that perform no outbound work (no OpenAI request and
    /// no telemetry send), so the first-use disclosure can be skipped for them.
    /// `doctor` is local only without the `network` operand; `feedback` and any
    /// model-routed verb (`run`/`ask`/`explain`/`repair`/`recover`/bare intent)
    /// send data and return false.
    pub fn is_local_only(&self) -> bool {
        match self.subcommand.as_deref() {
            Some(
                "config" | "context" | "history" | "telemetry" | "undo" | "restore" | "recovery",
            ) => true,
            Some("doctor") => !self
                .prompt
                .split_whitespace()
                .any(|value| value == "network"),
            _ => false,
        }
    }
}

const VERBS: &[&str] = &[
    "run",
    "ask",
    "explain",
    "history",
    "config",
    "context",
    "telemetry",
    "feedback",
    "repair",
    "recover",
    "undo",
    "restore",
    "recovery",
    "shell-init",
    "shell-control-open",
    "shell-validate",
    "shell-ack",
    "shell-clean",
    "shell-history-enabled",
    "doctor",
    "help",
    "version",
];

/// Basenames accepted for `--shell`, mirroring `command::normalize_shell` (which
/// matches on the path basename and treats `auto`/empty as "detect"). Kept here
/// rather than imported because `command` depends on `args`, so importing it
/// back would cycle. `auto` and empty are accepted separately at the call site.
const VALID_SHELLS: &[&str] = &["sh", "bash", "zsh", "fish", "pwsh", "powershell"];

pub fn parse_from(argv: Vec<String>) -> Result<Args, String> {
    let mut out = Args::default();
    let mut intent = Vec::new();
    let mut i = 1;
    let mut opaque = false;

    while i < argv.len() {
        let arg = &argv[i];
        if opaque {
            intent.push(arg.clone());
            i += 1;
            continue;
        }
        if arg == "--" {
            opaque = true;
            i += 1;
            continue;
        }

        match arg.as_str() {
            "-h" | "--help" => out.help = true,
            "-V" | "--version" => out.version = true,
            "--review" => out.review = true,
            "--dry-run" => out.dry_run = true,
            "--force" => out.force = true,
            "--plain" => out.plain = true,
            "--json" => out.json = true,
            "--no-stream" => out.no_stream = true,
            "--no-telemetry" => out.no_telemetry = true,
            "--no-motion" => out.no_motion = true,
            "--local-input" => out.local_input = true,
            "--retain-program" => out.retain_program = true,
            "--recoverable" => out.recoverable = true,
            "--fresh" | "--no-cache" => out.fresh = true,
            "-v" | "--verbose" => out.verbose = true,
            "-m" | "--model" => {
                i += 1;
                out.model = Some(argv.get(i).ok_or("--model needs a value")?.clone());
            }
            "--shell" => {
                i += 1;
                out.shell = Some(argv.get(i).ok_or("--shell needs a value")?.clone());
            }
            "--context" => {
                i += 1;
                out.context = Some(argv.get(i).ok_or("--context needs a value")?.clone());
            }
            "--input-format" => {
                i += 1;
                out.input_format = Some(argv.get(i).ok_or("--input-format needs a value")?.clone());
            }
            "--uhm-control-dir" => {
                i += 1;
                out.control_dir = Some(
                    argv.get(i)
                        .ok_or("--uhm-control-dir needs a value")?
                        .clone(),
                );
            }
            "--uhm-control-nonce" => {
                i += 1;
                out.control_nonce = Some(
                    argv.get(i)
                        .ok_or("--uhm-control-nonce needs a value")?
                        .clone(),
                );
            }
            "--uhm-last-history" => {
                i += 1;
                out.last_history_entry = Some(
                    argv.get(i)
                        .ok_or("--uhm-last-history needs a value")?
                        .clone(),
                );
            }
            "--uhm-shell" => {
                i += 1;
                out.integration_shell =
                    Some(argv.get(i).ok_or("--uhm-shell needs a value")?.clone());
            }
            "--uhm-parent-cwd" => {
                i += 1;
                out.parent_cwd = Some(argv.get(i).ok_or("--uhm-parent-cwd needs a value")?.clone());
            }
            "--uhm-parent-status" => {
                i += 1;
                out.parent_status = Some(
                    argv.get(i)
                        .ok_or("--uhm-parent-status needs a value")?
                        .parse()
                        .map_err(|_| "--uhm-parent-status needs an integer")?,
                );
            }
            _ if arg.starts_with("--model=") => {
                out.model = Some(arg[8..].to_string());
            }
            _ if arg.starts_with("--shell=") => {
                out.shell = Some(arg[8..].to_string());
            }
            _ if arg.starts_with("--context=") => {
                out.context = Some(arg[10..].to_string());
            }
            _ if arg.starts_with("--input-format=") => {
                out.input_format = Some(arg[15..].to_string());
            }
            _ if out.subcommand.is_none() && VERBS.contains(&arg.as_str()) => {
                out.subcommand = Some(arg.clone());
            }
            _ if arg.starts_with('-') => {
                return Err(format!(
                    "unknown option '{}'; put -- before intent that starts with '-'",
                    arg
                ));
            }
            _ => {
                intent.push(arg.clone());
                opaque = true;
            }
        }
        i += 1;
    }

    if out.review && out.dry_run {
        return Err("--review and --dry-run cannot be used together".into());
    }
    if out.review && out.force {
        return Err("--review and --force cannot be used together".into());
    }
    if out.dry_run && out.force {
        return Err("--dry-run and --force cannot be used together".into());
    }
    if let Some(format) = &out.input_format {
        if format.is_empty()
            || format.len() > 64
            || format
                .chars()
                .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '.' | '/')))
        {
            return Err("--input-format must be a 1-64 character format label".into());
        }
    }
    // Validate enumerated overrides at parse time so a bad `--shell`/`--context`
    // surfaces as a usage error (exit 2) before API-key resolution (exit 13).
    if let Some(shell) = &out.shell {
        let name = std::path::Path::new(shell)
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or(shell);
        if !(shell == "auto" || shell.is_empty() || VALID_SHELLS.contains(&name)) {
            return Err(format!(
                "unsupported shell '{}'; valid: auto, {}",
                shell,
                VALID_SHELLS.join(", ")
            ));
        }
    }
    if let Some(context) = &out.context {
        crate::context::Mode::parse(context)?;
    }
    out.prompt = intent.join(" ");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pv(words: &[&str]) -> Result<Args, String> {
        parse_from(words.iter().map(|s| (*s).to_string()).collect())
    }

    #[test]
    fn run_is_the_command_verb() {
        let a = pv(&["uhm", "run", "find", "big", "files"]).unwrap();
        assert_eq!(a.subcommand.as_deref(), Some("run"));
        assert_eq!(a.prompt, "find big files");
    }

    #[test]
    fn program_privacy_and_debug_flags_are_explicit() {
        let args = parse_from(vec![
            "uhm".into(),
            "--local-input".into(),
            "--input-format=text/csv".into(),
            "--retain-program".into(),
            "count".into(),
        ])
        .unwrap();
        assert!(args.local_input);
        assert!(args.retain_program);
        assert_eq!(args.input_format.as_deref(), Some("text/csv"));
        assert!(!args.recoverable);
        assert!(
            parse_from(vec!["uhm".into(), "--recoverable".into(), "rewrite".into()])
                .unwrap()
                .recoverable
        );
        assert!(parse_from(vec![
            "uhm".into(),
            "--input-format=bad label".into(),
            "count".into()
        ])
        .is_err());
    }

    #[test]
    fn prompt_is_opaque_after_first_word() {
        let a = pv(&["uhm", "find", "--force", "-y", "--model=x"]).unwrap();
        assert!(!a.force);
        assert_eq!(a.prompt, "find --force -y --model=x");
    }

    #[test]
    fn explicit_boundary_allows_leading_hyphen_intent() {
        let a = pv(&["uhm", "--plain", "--", "--find", "this"]).unwrap();
        assert!(a.plain);
        assert_eq!(a.prompt, "--find this");
    }

    #[test]
    fn bare_natural_language_is_the_primary_invocation() {
        let a = pv(&["uhm", "list", "the", "three", "biggest", "files"]).unwrap();
        assert_eq!(a.subcommand, None);
        assert_eq!(a.prompt, "list the three biggest files");

        let ask = pv(&["uhm", "ask", "write", "a", "one-line", "summary"]).unwrap();
        assert_eq!(ask.subcommand.as_deref(), Some("ask"));
        assert_eq!(ask.prompt, "write a one-line summary");
    }

    #[test]
    fn removed_yes_flag_is_rejected() {
        assert!(pv(&["uhm", "-y", "list", "files"]).is_err());
        assert!(pv(&["uhm", "--yes", "list", "files"]).is_err());
    }

    #[test]
    fn flags_are_allowed_after_a_subcommand_until_intent() {
        let a = pv(&["uhm", "run", "--review", "list", "--all"]).unwrap();
        assert!(a.review);
        assert_eq!(a.prompt, "list --all");
    }

    #[test]
    fn management_operands_are_opaque() {
        let a = pv(&["uhm", "config", "show", "--raw"]).unwrap();
        assert_eq!(a.subcommand.as_deref(), Some("config"));
        assert_eq!(a.prompt, "show --raw");
    }

    #[test]
    fn private_integration_values_preserve_spaces_and_precede_intent() {
        let args = parse_from(vec![
            "uhm".into(),
            "--uhm-control-dir".into(),
            "/tmp/a directory".into(),
            "--uhm-control-nonce".into(),
            "ab".repeat(32),
            "--uhm-last-history".into(),
            "printf 'secret value'".into(),
            "do".into(),
            "work".into(),
        ])
        .unwrap();
        assert_eq!(args.control_dir.as_deref(), Some("/tmp/a directory"));
        assert_eq!(
            args.last_history_entry.as_deref(),
            Some("printf 'secret value'")
        );
        assert_eq!(args.prompt, "do work");
    }

    #[test]
    fn documented_flag_like_prompts_reach_the_intended_mode() {
        let ask = pv(&["uhm", "ask", "what", "does", "-V", "mean"]).unwrap();
        assert_eq!(ask.subcommand.as_deref(), Some("ask"));
        assert_eq!(ask.prompt, "what does -V mean");

        let explain = pv(&["uhm", "explain", "git", "log", "-p"]).unwrap();
        assert_eq!(explain.subcommand.as_deref(), Some("explain"));
        assert_eq!(explain.prompt, "git log -p");

        let dictated = pv(&["uhm", "say", "-y", "--help", "--system", "verbatim"]).unwrap();
        assert!(!dictated.help);
        assert_eq!(dictated.prompt, "say -y --help --system verbatim");
    }

    #[test]
    fn invalid_shell_override_is_a_usage_error() {
        assert!(pv(&["uhm", "--shell", "cmd", "x"]).is_err());
        assert!(pv(&["uhm", "--shell=cmd", "x"]).is_err());
        assert!(pv(&["uhm", "--shell", "/bin/nope", "x"]).is_err());
    }

    #[test]
    fn valid_shell_overrides_are_accepted() {
        for shell in ["auto", "sh", "bash", "zsh", "fish", "pwsh", "powershell"] {
            assert!(pv(&["uhm", "--shell", shell, "x"]).is_ok(), "{shell}");
        }
        // Full paths are accepted because the downstream matcher uses the basename.
        assert!(pv(&["uhm", "--shell", "/bin/bash", "x"]).is_ok());
    }

    #[test]
    fn invalid_context_override_is_a_usage_error() {
        assert!(pv(&["uhm", "--context", "enormous", "x"]).is_err());
        assert!(pv(&["uhm", "--context=huge", "x"]).is_err());
    }

    #[test]
    fn valid_context_overrides_are_accepted() {
        for mode in ["minimal", "standard", "full"] {
            assert!(pv(&["uhm", "--context", mode, "x"]).is_ok(), "{mode}");
        }
    }

    #[test]
    fn local_only_classification_matches_outbound_risk() {
        let local = |sub: &str| Args {
            subcommand: Some(sub.into()),
            ..Default::default()
        };
        for sub in [
            "config",
            "context",
            "history",
            "telemetry",
            "undo",
            "restore",
            "recovery",
        ] {
            assert!(local(sub).is_local_only(), "{sub}");
        }
        assert!(local("doctor").is_local_only(), "doctor without network");

        let doctor_network = Args {
            subcommand: Some("doctor".into()),
            prompt: "network".into(),
            ..Default::default()
        };
        assert!(
            !doctor_network.is_local_only(),
            "doctor network is outbound"
        );

        assert!(
            !local("feedback").is_local_only(),
            "feedback sends telemetry"
        );

        let bare = Args {
            prompt: "list the three biggest files".into(),
            ..Default::default()
        };
        assert!(!bare.is_local_only(), "bare intent is outbound");
    }
}
