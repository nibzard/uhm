//! Invocation grammar. The first intent word is an intentional parsing boundary:
//! every following token is opaque user text, even when it starts with `-`.

#[derive(Debug, Default, PartialEq, Eq)]
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
    pub fresh: bool,
    pub verbose: bool,
    pub help: bool,
    pub version: bool,
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
    "doctor",
    "help",
    "version",
];

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
    fn documented_flag_like_prompts_reach_the_intended_mode() {
        let ask = pv(&["uhm", "ask", "--", "what", "does", "-V", "mean"]).unwrap();
        assert_eq!(ask.subcommand.as_deref(), Some("ask"));
        assert_eq!(ask.prompt, "what does -V mean");

        let explain = pv(&["uhm", "explain", "--", "git", "log", "-p"]).unwrap();
        assert_eq!(explain.subcommand.as_deref(), Some("explain"));
        assert_eq!(explain.prompt, "git log -p");

        let dictated = pv(&["uhm", "say", "-y", "--help", "--system", "verbatim"]).unwrap();
        assert!(!dictated.help);
        assert_eq!(dictated.prompt, "say -y --help --system verbatim");
    }
}
