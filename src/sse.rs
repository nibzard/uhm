//! Server-Sent Events reader: splits curl/ureq stdout into whole JSON frames.
//! Each `data: {...}` line is a complete JSON object, so we never parse fragments.

use std::io::{BufRead, BufReader, Read};

pub fn read_stream<R: Read, F: FnMut(&str)>(reader: R, mut on_token: F) -> Result<(), String> {
    let mut br = BufReader::new(reader);
    let mut line = String::new();
    let mut completed = false;
    loop {
        line.clear();
        let n = br
            .read_line(&mut line)
            .map_err(|e| format!("read: {}", e))?;
        if n == 0 {
            break;
        }
        let l = line.trim_end_matches(['\n', '\r']);
        if l.is_empty() {
            continue;
        }
        let data = if let Some(d) = l.strip_prefix("data: ") {
            d
        } else if let Some(d) = l.strip_prefix("data:") {
            d
        } else {
            continue; // ignore event:/id:/comment lines
        };
        let data = data.trim();
        if data.is_empty() {
            continue;
        }
        if data == "[DONE]" {
            completed = true;
            break;
        }
        let j: serde_json::Value =
            serde_json::from_str(data).map_err(|e| format!("invalid SSE data: {}", e))?;
        if let Some(message) = j
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            return Err(format!(
                "API stream error: {}",
                crate::render::ansi::sanitize_untrusted(message)
            ));
        }
        if let Some(content) = j
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .and_then(|ch| ch.get("delta"))
            .and_then(|d| d.get("content"))
            .and_then(|c| c.as_str())
        {
            if !content.is_empty() {
                on_token(content);
            }
        }
    }
    if completed {
        Ok(())
    } else {
        Err("API stream ended before [DONE]".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_tokens_until_done() {
        let input = b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]\n";
        let mut out = String::new();
        read_stream(&input[..], |token| out.push_str(token)).unwrap();
        assert_eq!(out, "hi");
    }

    #[test]
    fn rejects_truncated_malformed_and_error_streams() {
        assert!(read_stream(&b"data: {}\n"[..], |_| {}).is_err());
        assert!(read_stream(&b"data: nope\n\ndata: [DONE]\n"[..], |_| {}).is_err());
        let error = b"data: {\"error\":{\"message\":\"bad request\"}}\n\ndata: [DONE]\n";
        assert!(read_stream(&error[..], |_| {})
            .unwrap_err()
            .contains("bad request"));
    }
}
