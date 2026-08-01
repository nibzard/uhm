//! Color discipline. Honors NO_COLOR / FORCE_COLOR / CLICOLOR_FORCE, rides the
//! terminal's 16-color palette so uhm matches whatever theme the user chose.

use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::render::capability;

static PLAIN: AtomicBool = AtomicBool::new(false);

pub fn set_plain(plain: bool) {
    PLAIN.store(plain, Ordering::Relaxed);
}

pub fn plain_enabled() -> bool {
    PLAIN.load(Ordering::Relaxed)
        || std::env::var_os("UHM_PLAIN").is_some_and(|v| !v.is_empty())
        || std::env::var("TERM").is_ok_and(|term| term == "dumb")
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

/// Render untrusted model/server text without allowing it to emit terminal
/// control sequences or visually rewrite a confirmation prompt.
pub fn sanitize_untrusted(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' => out.push('\n'),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => {
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
pub fn accent(s: &str) -> String {
    wrap("\x1b[36m", s, "\x1b[39m")
} // cyan
pub fn green(s: &str) -> String {
    wrap("\x1b[32m", s, "\x1b[39m")
}
pub fn yellow(s: &str) -> String {
    wrap("\x1b[33m", s, "\x1b[39m")
}
pub fn magenta(s: &str) -> String {
    wrap("\x1b[35m", s, "\x1b[39m")
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

/// OSC 8 hyperlink: the terminal renders `text` and opens `url` on click.
/// Falls back to plain text where OSC 8 isn't supported.
pub fn link(url: &str, text: &str) -> String {
    if capability::supports_hyperlinks() {
        format!("\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\", url, text)
    } else {
        text.to_string()
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
}
