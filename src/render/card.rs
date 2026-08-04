//! Restrained review output. Product UI goes to stderr; command bytes stay exact.

use crate::action::Effect;
use crate::render::{ansi, highlight, layout};
use crate::safety::Tier;

pub fn preview(
    command: &str,
    summary: &str,
    tier: Tier,
    declared: &[Effect],
    detected: &[Effect],
    reasons: &[String],
) {
    eprintln!("{}", ansi::primary("Proposed command"));
    eprintln!("{}", highlight::highlight(command, tier));
    if !summary.is_empty() {
        eprintln!("{}", ansi::sanitize_untrusted(summary));
    }
    if !(declared.is_empty() && detected.is_empty()) {
        eprintln!(
            "{}",
            layout::labeled(
                &ansi::focus("Effects:"),
                &ansi::warning(&effects_line(declared, detected)),
                layout::columns()
            )
        );
    }
    for reason in dedup(reasons) {
        eprintln!(
            "{} {}",
            ansi::muted("Detected:"),
            ansi::sanitize_untrusted_inline(&reason)
        );
    }
}

/// Union of the declared and detected effect sets, kept in full because
/// under-reporting is the dangerous direction. A locally detected effect
/// renders plain; a declared-only effect is marked "(declared)" so the line
/// never implies an observation the local scan did not make.
pub fn effects_line(declared: &[Effect], detected: &[Effect]) -> String {
    let mut seen: Vec<&Effect> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    for effect in detected {
        if !seen.contains(&effect) {
            seen.push(effect);
            labels.push(effect.label().into());
        }
    }
    for effect in declared {
        if !seen.contains(&effect) {
            seen.push(effect);
            labels.push(format!("{} (declared)", effect.label()));
        }
    }
    labels.join(", ")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_line_marks_declared_only_effects_and_keeps_the_union() {
        assert_eq!(
            effects_line(
                &[Effect::WriteLocal, Effect::ReadLocal],
                &[Effect::ReadLocal]
            ),
            "reads local data, writes local data (declared)"
        );
    }

    #[test]
    fn effects_line_leaves_detected_effects_unmarked() {
        assert_eq!(
            effects_line(&[], &[Effect::NetworkRead]),
            "uses the network"
        );
        assert_eq!(
            effects_line(&[Effect::ReadLocal], &[Effect::ReadLocal]),
            "reads local data"
        );
    }
}
