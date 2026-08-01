//! Terminal capability probes (std-only). Width via COLUMNS or `stty size`;
//! Ghostty detection for optional flourishes. Everything degrades gracefully.

use std::process::Command;

pub fn cols() -> usize {
    if let Some(c) = std::env::var_os("COLUMNS") {
        if let Ok(s) = c.into_string() {
            if let Ok(n) = s.trim().parse::<usize>() {
                if (10..=1000).contains(&n) {
                    return n;
                }
            }
        }
    }
    if let Ok(out) = Command::new("stty").arg("size").output() {
        let s = String::from_utf8_lossy(&out.stdout);
        if let Some(col) = s.split_whitespace().nth(1) {
            if let Ok(n) = col.parse::<usize>() {
                if (10..=1000).contains(&n) {
                    return n;
                }
            }
        }
    }
    80
}

/// Terminals implementing the styled-underline protocol (SGR `4:x`). Undercurls
/// are only emitted when this is true, so we never print stray escape codes on
/// terminals that don't understand them.
pub fn supports_styled_underline() -> bool {
    matches!(
        term_program(),
        Some(tp) if matches!(tp.as_str(), "ghostty" | "kitty" | "WezTerm" | "konsole" | "tmux")
    ) || env_flag("UHM_UNDERLINE")
}

/// Terminals that honor OSC 8 hyperlinks (clickable URLs).
pub fn supports_hyperlinks() -> bool {
    matches!(
        term_program(),
        Some(tp) if matches!(tp.as_str(), "ghostty" | "iTerm.app" | "WezTerm" | "kitty" | "konsole")
    ) || env_flag("UHM_LINKS")
}

fn term_program() -> Option<String> {
    std::env::var_os("TERM_PROGRAM").and_then(|v| v.into_string().ok())
}

fn env_flag(name: &str) -> bool {
    std::env::var_os(name)
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}
