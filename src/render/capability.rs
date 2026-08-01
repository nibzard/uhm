//! Terminal capability probes (std-only). Width via COLUMNS or `stty size`;
//! Ghostty detection for optional flourishes. Everything degrades gracefully.

/// Terminals implementing the styled-underline protocol (SGR `4:x`). Undercurls
/// are only emitted when this is true, so we never print stray escape codes on
/// terminals that don't understand them.
pub fn supports_styled_underline() -> bool {
    matches!(
        term_program(),
        Some(tp) if matches!(tp.as_str(), "ghostty" | "kitty" | "WezTerm" | "konsole" | "tmux")
    ) || env_flag("UHM_UNDERLINE")
}

fn term_program() -> Option<String> {
    std::env::var_os("TERM_PROGRAM").and_then(|v| v.into_string().ok())
}

fn env_flag(name: &str) -> bool {
    std::env::var_os(name)
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}
