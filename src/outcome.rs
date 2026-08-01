//! Stable application outcomes. Child-process statuses are never remapped.

use serde::Serialize;

pub const USAGE: i32 = 2;
pub const MODEL: i32 = 10;
pub const NOT_EXECUTED: i32 = 11;
pub const CLARIFICATION: i32 = 12;
pub const CONFIG: i32 = 13;

#[derive(Serialize)]
pub struct Outcome<'a> {
    pub namespace: &'static str,
    pub outcome: &'a str,
    pub exit_code: i32,
    pub executed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<&'a str>,
}

impl<'a> Outcome<'a> {
    pub fn json(&self) -> String {
        serde_json::to_string(self).expect("Outcome is always serializable")
    }
}
