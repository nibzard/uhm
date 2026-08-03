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

/// Wrap prose to display cells, preserving words when possible and splitting a
/// single overlong token only when it cannot fit on an otherwise empty line.
pub fn wrap(value: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();

    for word in value.split_whitespace() {
        let separator = usize::from(!line.is_empty());
        if display_width(&line) + separator + display_width(word) <= width {
            if separator == 1 {
                line.push(' ');
            }
            line.push_str(word);
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        for ch in word.chars() {
            let mut encoded = [0; 4];
            let piece = ch.encode_utf8(&mut encoded);
            if !line.is_empty() && display_width(&line) + display_width(piece) > width {
                lines.push(std::mem::take(&mut line));
            }
            line.push(ch);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
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

    #[test]
    fn wraps_words_and_long_tokens_to_display_width() {
        for width in [10, 24, 80] {
            let lines = wrap("short words https://api.openai.com/v1/responses 雪", width);
            assert!(lines.iter().all(|line| display_width(line) <= width));
        }
    }
}
