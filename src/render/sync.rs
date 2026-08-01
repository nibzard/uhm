//! Terminal sync-output (DECSET 2026) suppresses flicker during redraws.
//! Used by the REPL (Phase 7). Terminals that don't understand it ignore it.

pub fn begin() -> &'static str {
    "\x1b[?2026h"
}
pub fn end() -> &'static str {
    "\x1b[?2026l"
}
pub fn wrap(body: &str) -> String {
    if crate::render::ansi::plain_enabled()
        || !std::io::IsTerminal::is_terminal(&std::io::stdout())
        || std::env::var_os("UHM_SYNC_OUTPUT").is_none()
    {
        body.to_string()
    } else {
        format!("{}{}{}", begin(), body, end())
    }
}
