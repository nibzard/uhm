//! Small display-cell-aware layout helpers for narrow and Unicode terminals.

use unicode_width::UnicodeWidthStr;

pub fn display_width(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}

pub fn columns() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| (20..=1000).contains(value))
        .unwrap_or(80)
}

pub fn labeled(label: &str, value: &str, width: usize) -> String {
    if display_width(label) + 1 + display_width(value) <= width {
        format!("{} {}", label, value)
    } else {
        format!("{}\n  {}", label, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_display_cells_not_bytes() {
        assert_eq!(display_width("雪"), 2);
        assert_eq!(display_width("e\u{301}"), 1);
        assert_eq!(display_width("🙂"), 2);
    }

    #[test]
    fn layouts_are_stable_at_release_widths() {
        for width in [40, 80, 160] {
            let value = labeled("Effects:", "reads local files, network access", width);
            assert!(!value.contains('\r'));
            assert!(value.lines().all(|line| display_width(line) <= width));
        }
    }
}
