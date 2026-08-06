//! Color discipline. Honors NO_COLOR / FORCE_COLOR / CLICOLOR_FORCE, rides the
//! terminal's 16-color palette so uhm matches whatever theme the user chose.

use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::render::capability;

static PLAIN: AtomicBool = AtomicBool::new(false);
static NO_MOTION: AtomicBool = AtomicBool::new(false);

pub fn set_plain(plain: bool) {
    PLAIN.store(plain, Ordering::Relaxed);
}

pub fn set_no_motion(no_motion: bool) {
    NO_MOTION.store(no_motion, Ordering::Relaxed);
}

fn enabled_env(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "on"))
}

pub fn plain_enabled() -> bool {
    PLAIN.load(Ordering::Relaxed)
        || enabled_env("UHM_PLAIN")
        || std::env::var("TERM").is_ok_and(|term| term == "dumb")
}

pub fn motion_enabled() -> bool {
    !plain_enabled()
        && !NO_MOTION.load(Ordering::Relaxed)
        && !enabled_env("UHM_NO_MOTION")
        && !enabled_env("NO_MOTION")
}

pub fn color_enabled() -> bool {
    if plain_enabled() {
        return false;
    }
    if std::env::var_os("NO_COLOR")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        return false;
    }
    if std::env::var_os("FORCE_COLOR")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        return true;
    }
    if let Some(c) = std::env::var_os("CLICOLOR_FORCE") {
        if !c.is_empty() {
            return true;
        }
    }
    std::io::stderr().is_terminal()
}

/// Unicode format characters (category `Cf`) and the direction overrides in
/// particular. `char::is_control` covers only category `Cc`, so these otherwise
/// reach the terminal verbatim: a right-to-left override reverses how an
/// operand renders without changing the bytes that run.
pub fn is_format_control(c: char) -> bool {
    matches!(c,
        '\u{00ad}'
        | '\u{061c}'
        | '\u{180e}'
        | '\u{200b}'..='\u{200f}'
        | '\u{202a}'..='\u{202e}'
        | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{206f}'
        | '\u{feff}'
        | '\u{fff9}'..='\u{fffb}'
        | '\u{110bd}'
        | '\u{110cd}'
        | '\u{1d173}'..='\u{1d17a}'
        | '\u{e0000}'..='\u{e007f}')
}

/// Render untrusted model/server text without allowing it to emit terminal
/// control sequences or visually rewrite a confirmation prompt.
pub fn sanitize_untrusted(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' => out.push('\n'),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() || is_format_control(c) => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// Single-line variant for command previews and history rows, where embedded
/// newlines could otherwise forge adjacent UI lines.
pub fn sanitize_untrusted_inline(s: &str) -> String {
    sanitize_untrusted(s).replace('\n', "\\n")
}

fn wrap(pre: &str, s: &str, post: &str) -> String {
    if color_enabled() {
        format!("{}{}{}", pre, s, post)
    } else {
        s.to_string()
    }
}

pub fn dim(s: &str) -> String {
    wrap("\x1b[2m", s, "\x1b[22m")
}
pub fn bold(s: &str) -> String {
    wrap("\x1b[1m", s, "\x1b[22m")
}
pub fn green(s: &str) -> String {
    wrap("\x1b[32m", s, "\x1b[39m")
}
pub fn yellow(s: &str) -> String {
    wrap("\x1b[33m", s, "\x1b[39m")
}
pub fn magenta(s: &str) -> String {
    wrap("\x1b[35m", s, "\x1b[39m")
}

pub fn red(s: &str) -> String {
    wrap("\x1b[31m", s, "\x1b[39m")
}

// Semantic tokens. Product copy should use these instead of choosing colors.
pub fn primary(s: &str) -> String {
    bold(s)
}
pub fn muted(s: &str) -> String {
    dim(s)
}
pub fn success(s: &str) -> String {
    green(s)
}
pub fn warning(s: &str) -> String {
    yellow(s)
}
pub fn critical(s: &str) -> String {
    red(s)
}
pub fn info(s: &str) -> String {
    magenta(s)
}
pub fn focus(s: &str) -> String {
    undercurl(s)
}

/// Squiggly underline (SGR `4:3`). Emits codes only on terminals that speak the
/// styled-underline protocol; otherwise returns the text untouched.
pub fn undercurl(s: &str) -> String {
    if color_enabled() && capability::supports_styled_underline() {
        format!("\x1b[4:3m{}\x1b[4:0m", s)
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untrusted_controls_are_visible_not_executable() {
        assert_eq!(
            sanitize_untrusted("ok\x1b]52;c;bad\x07\rno"),
            "ok\\u{1b}]52;c;bad\\u{7}\\rno"
        );
        assert_eq!(sanitize_untrusted_inline("first\nsecond"), "first\\nsecond");
    }

    #[test]
    fn direction_and_invisible_formatting_are_escaped() {
        // `char::is_control` is category Cc only, so these reach the review card
        // unless they are handled explicitly. A bidi override can reverse the
        // rendered operand of a command the user is about to approve.
        for (raw, escaped) in [
            ("a\u{202e}b", "a\\u{202e}b"),
            ("a\u{2066}b", "a\\u{2066}b"),
            ("a\u{200b}b", "a\\u{200b}b"),
            ("a\u{00ad}b", "a\\u{ad}b"),
            ("a\u{feff}b", "a\\u{feff}b"),
        ] {
            assert_eq!(sanitize_untrusted(raw), escaped, "{raw:?}");
        }
        // Ordinary non-ASCII text is untouched.
        assert_eq!(
            sanitize_untrusted("caf\u{e9} \u{2192} ok"),
            "caf\u{e9} \u{2192} ok"
        );
    }
}
