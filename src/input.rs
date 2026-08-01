//! Bounded, byte-exact stdin spool.

use serde_json::{json, Value};
use std::io::{IsTerminal, Read};

#[derive(Debug, Clone, Default)]
pub struct Spool {
    bytes: Vec<u8>,
    piped: bool,
}
impl Spool {
    pub fn read(max: usize) -> Result<Self, String> {
        if std::io::stdin().is_terminal() {
            return Ok(Self::default());
        }
        let mut bytes = Vec::new();
        std::io::stdin()
            .take((max + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("read stdin: {}", e))?;
        if bytes.len() > max {
            return Err(format!("stdin exceeds configured {} byte limit", max));
        }
        Ok(Self { bytes, piped: true })
    }
    #[cfg(test)]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self { bytes, piped: true }
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn model_value(&self) -> Value {
        match std::str::from_utf8(&self.bytes) {
            Ok(text) => {
                json!({"present":self.piped,"byte_count":self.bytes.len(),"utf8":true,"text":text})
            }
            Err(_) => json!({"present":self.piped,"byte_count":self.bytes.len(),"utf8":false}),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn non_utf8_never_enters_json() {
        let s = Spool::from_bytes(vec![0xff, 0]);
        let v = s.model_value();
        assert_eq!(v["utf8"], false);
        assert!(v.get("text").is_none());
        assert_eq!(s.bytes(), &[0xff, 0]);
    }
}
