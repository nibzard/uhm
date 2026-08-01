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
    pub fn is_piped(&self) -> bool {
        self.piped
    }
    #[cfg(test)]
    pub fn model_value(&self) -> Value {
        self.model_value_for(false, None)
    }
    pub fn model_value_for(&self, local_only: bool, declared_format: Option<&str>) -> Value {
        if local_only {
            return json!({
                "present":self.piped,
                "byte_count":self.bytes.len(),
                "utf8":std::str::from_utf8(&self.bytes).is_ok(),
                "local_only":true,
                "declared_format":declared_format
            });
        }
        match std::str::from_utf8(&self.bytes) {
            Ok(text) => {
                json!({"present":self.piped,"byte_count":self.bytes.len(),"utf8":true,"text":text,"local_only":false,"declared_format":declared_format})
            }
            Err(_) => {
                json!({"present":self.piped,"byte_count":self.bytes.len(),"utf8":false,"local_only":false,"declared_format":declared_format})
            }
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

    #[test]
    fn local_input_never_places_content_in_model_json() {
        let private = "sentinel local content";
        let spool = Spool::from_bytes(private.as_bytes().to_vec());
        let value = spool.model_value_for(true, Some("text/plain"));
        assert_eq!(value["local_only"], true);
        assert_eq!(value["byte_count"], private.len());
        assert_eq!(value["declared_format"], "text/plain");
        assert!(!value.to_string().contains(private));
        assert!(value.get("text").is_none());
    }
}
