//! Restrained review output. Product UI goes to stderr; command bytes stay exact.

use crate::action::Effect;
use crate::render::{ansi, highlight};
use crate::safety::Tier;

pub fn preview(command: &str, summary: &str, tier: Tier, effects: &[Effect], reasons: &[String]) {
    eprintln!("{}", ansi::bold("Proposed command"));
    eprintln!("{}", highlight::highlight(command, tier));
    if !summary.is_empty() {
        eprintln!("{}", ansi::sanitize_untrusted(summary));
    }
    if !effects.is_empty() {
        let labels: Vec<_> = effects.iter().map(Effect::label).collect();
        eprintln!("{} {}", ansi::yellow("Effects:"), labels.join(", "));
    }
    for reason in dedup(reasons) {
        eprintln!(
            "{} {}",
            ansi::dim("Detected:"),
            ansi::sanitize_untrusted_inline(&reason)
        );
    }
}

fn dedup(reasons: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for reason in reasons {
        if !out.contains(reason) {
            out.push(reason.clone());
        }
    }
    out
}
