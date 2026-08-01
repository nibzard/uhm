//! Tiny Markdown → ANSI renderer for ask/explain output. Handles headings,
//! fenced code, bold, inline code, lists, blockquotes, and rules. Not a full
//! CommonMark — just enough to make LLM answers read well in the terminal.

use crate::render::{ansi, capability};

pub fn render(s: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in s.lines() {
        let t = line.trim_end();

        if t.trim_start().starts_with("```") {
            in_fence = !in_fence;
            out.push_str(&ansi::dim(t));
            out.push('\n');
            continue;
        }
        if in_fence {
            out.push_str(&format!("{} {}", ansi::dim("│"), line));
            out.push('\n');
            continue;
        }

        if let Some(h) = strip_heading(t, "# ") {
            out.push_str(&ansi::bold(h));
        } else if let Some(h) = strip_heading(t, "## ") {
            out.push_str(&ansi::bold(h));
        } else if let Some(h) = strip_heading(t, "### ") {
            out.push_str(&ansi::bold(h));
        } else if is_rule(t) {
            let w = capability::cols().saturating_sub(2).max(10);
            out.push_str(&ansi::dim(&"─".repeat(w)));
        } else if let Some(item) = strip_prefixes(t, &["- ", "* ", "+ "]) {
            out.push_str(&format!("  {} {}", ansi::accent("•"), render_inline(item)));
        } else if let Some((num, rest)) = ordered_item(t) {
            out.push_str(&format!(
                "  {} {}",
                ansi::accent(&format!("{}.", num)),
                render_inline(rest)
            ));
        } else if let Some(q) = t.strip_prefix("> ") {
            out.push_str(&format!("{} {}", ansi::dim("▏"), render_inline(q)));
        } else if t.is_empty() {
            out.push('\n');
            continue;
        } else {
            out.push_str(&render_inline(t));
        }
        out.push('\n');
    }
    out
}

fn strip_heading<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    line.strip_prefix(marker)
}

fn is_rule(t: &str) -> bool {
    matches!(t, "---" | "***" | "___")
}

fn strip_prefixes<'a>(t: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    for p in prefixes {
        if let Some(rest) = t.strip_prefix(p) {
            return Some(rest);
        }
    }
    None
}

fn ordered_item(t: &str) -> Option<(usize, &str)> {
    let b = t.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 || i + 1 >= b.len() {
        return None;
    }
    if b[i] == b'.' && b[i + 1] == b' ' {
        let n = t[..i].parse::<usize>().ok()?;
        Some((n, &t[i + 2..]))
    } else {
        None
    }
}

/// Inline: `` `code` ``, `**bold**`, and bare URLs (OSC 8 links). Italic is
/// skipped (ambiguous with `*`).
fn render_inline(text: &str) -> String {
    let cs: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < cs.len() {
        // bare http(s) URL → clickable OSC 8 link where supported
        if let Some(end) = url_end(&cs, i) {
            let url: String = cs[i..end].iter().collect();
            out.push_str(&ansi::link(&url, &url));
            i = end;
            continue;
        }
        if cs[i] == '`' {
            if let Some(end) = find_char(&cs, '`', i + 1) {
                let code: String = cs[i + 1..end].iter().collect();
                out.push_str(&ansi::green(&code));
                i = end + 1;
                continue;
            }
        }
        if i + 1 < cs.len() && cs[i] == '*' && cs[i + 1] == '*' {
            if let Some(end) = find_double(&cs, '*', i + 2) {
                let inner: String = cs[i + 2..end].iter().collect();
                out.push_str(&ansi::bold(&inner));
                i = end + 2;
                continue;
            }
        }
        out.push(cs[i]);
        i += 1;
    }
    out
}

/// If `cs[i..]` begins with an `http://`/`https://` URL, return the index just
/// past it; otherwise None. Trailing punctuation (`.` `,` `)` `;`) is excluded.
fn url_end(cs: &[char], i: usize) -> Option<usize> {
    let http = ['h', 't', 't', 'p', ':', '/', '/'];
    let https = ['h', 't', 't', 'p', 's', ':', '/', '/'];
    let scheme = if starts(cs, i, &https) {
        https.len()
    } else if starts(cs, i, &http) {
        http.len()
    } else {
        return None;
    };
    let mut j = i + scheme;
    while j < cs.len() && !cs[j].is_whitespace() {
        j += 1;
    }
    // strip trailing punctuation that clearly isn't part of the URL
    while j > i + scheme && matches!(cs[j - 1], '.' | ',' | ')' | ';' | ':' | '!' | '?') {
        j -= 1;
    }
    Some(j)
}

fn starts(cs: &[char], i: usize, pat: &[char]) -> bool {
    i + pat.len() <= cs.len() && cs[i..i + pat.len()] == *pat
}

fn find_char(cs: &[char], c: char, from: usize) -> Option<usize> {
    (from..cs.len()).find(|&i| cs[i] == c)
}

fn find_double(cs: &[char], c: char, from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < cs.len() {
        if cs[i] == c && cs[i + 1] == c {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_without_panic() {
        let md = "# Title\n\nSome **bold** and `code`.\n\n- a\n- b\n\n```sh\necho hi\n```\n";
        let r = render(md);
        assert!(r.contains("Title"));
        assert!(r.contains("echo hi"));
    }

    #[test]
    fn url_end_extracts_and_strips_trailing_punct() {
        let cs: Vec<char> = "see https://example.com/a?x=1). more".chars().collect();
        let end = url_end(&cs, 4).unwrap();
        let url: String = cs[4..end].iter().collect();
        assert_eq!(url, "https://example.com/a?x=1"); // trailing ). stripped
    }

    #[test]
    fn url_end_rejects_non_url() {
        let cs: Vec<char> = "just text here".chars().collect();
        assert!(url_end(&cs, 0).is_none());
    }

    #[test]
    fn bare_url_becomes_osc8_link_when_supported() {
        std::env::set_var("UHM_LINKS", "1");
        let r = render_inline("docs at https://example.com/page end");
        assert!(
            r.contains("\x1b]8;;https://example.com/page\x1b\\"),
            "expected OSC 8 sequence, got: {:?}",
            r
        );
        std::env::remove_var("UHM_LINKS");
    }
}
