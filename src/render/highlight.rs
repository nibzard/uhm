//! Command syntax highlight — word-classify (preserves spacing exactly).
//! When the command is destructive+, lethal tokens get a squiggly undercurl on
//! terminals that speak the styled-underline protocol.

use crate::render::{ansi, capability};
use crate::safety::Tier;

pub fn highlight(cmd: &str, tier: Tier) -> String {
    if !ansi::color_enabled() {
        return cmd.to_string();
    }
    let underline =
        tier.severity() >= Tier::Destructive.severity() && capability::supports_styled_underline();
    let mut out = String::with_capacity(cmd.len() + 32);
    let mut first_word = true;
    let mut start = 0;
    while start < cmd.len() {
        let is_space = cmd[start..].chars().next().unwrap().is_whitespace();
        let mut end = start;
        for (offset, ch) in cmd[start..].char_indices() {
            if ch.is_whitespace() != is_space {
                end = start + offset;
                break;
            }
            end = start + offset + ch.len_utf8();
        }
        let word = &cmd[start..end];
        if is_space {
            out.push_str(word);
            start = end;
            continue;
        }
        let styled = if is_operator(word) {
            ansi::dim(word)
        } else if first_word {
            ansi::bold(word)
        } else if word.starts_with('-') {
            ansi::green(word)
        } else if word.starts_with('"') || word.starts_with('\'') {
            ansi::yellow(word)
        } else if is_number(word) {
            ansi::magenta(word)
        } else {
            word.to_string()
        };
        if underline && (first_word || is_lethal_token(word)) {
            out.push_str(&ansi::undercurl(&styled));
        } else {
            out.push_str(&styled);
        }
        first_word = false;
        start = end;
    }
    out
}

fn is_operator(w: &str) -> bool {
    matches!(
        w,
        "|" | "||" | "&&" | ";" | "&" | ">" | ">>" | "<" | "<<" | "2>" | "2>>" | "&>" | "|&"
    )
}

fn is_number(w: &str) -> bool {
    !w.is_empty()
        && w.chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
        && w.chars().any(|c| c.is_ascii_digit())
}

/// Tokens that draw the eye when a command is destructive: the force/hard/nuke
/// flags, signal-9, and raw device files or the filesystem root.
fn is_lethal_token(w: &str) -> bool {
    let lw = w.to_lowercase();
    matches!(
        lw.as_str(),
        "--force"
            | "--hard"
            | "--no-preserve-root"
            | "-rf"
            | "-fr"
            | "-rfv"
            | "-frv"
            | "--delete"
            | "-9"
            | "--purge"
            | "--destroy"
    ) || lw.contains("/dev/")
        || lw == "/"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn restore(name: &str, value: Option<std::ffi::OsString>) {
        if let Some(value) = value {
            std::env::set_var(name, value);
        } else {
            std::env::remove_var(name);
        }
    }

    #[test]
    fn plain_without_color() {
        let _lock = ENV_LOCK.lock().unwrap();
        let old = std::env::var_os("NO_COLOR");
        std::env::set_var("NO_COLOR", "1");
        assert_eq!(highlight("ls -la", Tier::None), "ls -la");
        restore("NO_COLOR", old);
    }

    #[test]
    fn lethal_token_detected() {
        assert!(is_lethal_token("--force"));
        assert!(is_lethal_token("-rf"));
        assert!(is_lethal_token("/dev/sda"));
        assert!(!is_lethal_token("-la"));
    }

    #[test]
    fn destructive_command_gets_undercurl() {
        let _lock = ENV_LOCK.lock().unwrap();
        let old_no_color = std::env::var_os("NO_COLOR");
        let old_force = std::env::var_os("FORCE_COLOR");
        let old_underline = std::env::var_os("UHM_UNDERLINE");
        // Force color + the styled-underline protocol, then expect SGR 4:3 on the
        // command word and the lethal flag, but NOT on the harmless operand.
        std::env::remove_var("NO_COLOR");
        std::env::set_var("FORCE_COLOR", "1");
        std::env::set_var("UHM_UNDERLINE", "1");
        let r = highlight("rm -rf /tmp/x", Tier::Irreversible);
        assert!(r.contains("\x1b[4:3m"), "expected undercurl, got: {:?}", r);
        assert!(r.contains("\x1b[4:0m"), "expected undercurl reset");
        restore("NO_COLOR", old_no_color);
        restore("FORCE_COLOR", old_force);
        restore("UHM_UNDERLINE", old_underline);
    }

    #[test]
    fn safe_command_has_no_undercurl() {
        let _lock = ENV_LOCK.lock().unwrap();
        let old_no_color = std::env::var_os("NO_COLOR");
        let old_force = std::env::var_os("FORCE_COLOR");
        let old_underline = std::env::var_os("UHM_UNDERLINE");
        std::env::remove_var("NO_COLOR");
        std::env::set_var("FORCE_COLOR", "1");
        std::env::set_var("UHM_UNDERLINE", "1");
        let r = highlight("ls -la", Tier::None);
        assert!(
            !r.contains("\x1b[4:3m"),
            "safe command must not undercurl: {:?}",
            r
        );
        restore("NO_COLOR", old_no_color);
        restore("FORCE_COLOR", old_force);
        restore("UHM_UNDERLINE", old_underline);
    }

    #[test]
    fn highlighting_preserves_all_command_bytes_after_stripping_ansi() {
        let _lock = ENV_LOCK.lock().unwrap();
        let old_no_color = std::env::var_os("NO_COLOR");
        let old_force = std::env::var_os("FORCE_COLOR");
        std::env::remove_var("NO_COLOR");
        std::env::set_var("FORCE_COLOR", "1");
        let commands = [
            "printf  '%s\\n'  'hello world'",
            "echo one\n  | sed 's/o/ø/'",
            "变量='雪'\tprintf '%s' \"$变量\"",
            "printf x > out && printf y >> out",
            "cat <<'EOF'\nspaces  stay\n雪\nEOF",
        ];
        for command in commands {
            assert_eq!(strip_ansi(&highlight(command, Tier::Low)), command);
        }
        restore("NO_COLOR", old_no_color);
        restore("FORCE_COLOR", old_force);
    }

    fn strip_ansi(input: &str) -> String {
        let mut out = String::new();
        let mut chars = input.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\u{1b}' && chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            } else {
                out.push(ch);
            }
        }
        out
    }
}
