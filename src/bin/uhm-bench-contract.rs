//! Private benchmark bridge to the production action contract.

use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Read;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    tool: String,
    arguments: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreflightEnvelope {
    tool: String,
    arguments: Value,
    piped_input_present: bool,
}

fn main() {
    let operation = std::env::args().nth(1).unwrap_or_default();
    let result = match operation.as_str() {
        "describe" => uhm_cli::contract::description(),
        "qualification-context" => uhm_cli::capabilities::qualification_context(),
        "validate-qualification-manifest" => {
            let mut raw = Vec::new();
            let result = std::io::stdin()
                .take(2 * 1024 * 1024 + 1)
                .read_to_end(&mut raw);
            if result.is_err() || raw.len() > 2 * 1024 * 1024 {
                json!({"valid":false,"message":"qualification manifest exceeds 2097152 bytes"})
            } else {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                match uhm_cli::capabilities::validate_manifest_bytes(&raw, now) {
                    Ok(()) => json!({"valid":true}),
                    Err(message) => json!({"valid":false,"message":message}),
                }
            }
        }
        "validate" => {
            let mut raw = Vec::new();
            if std::io::stdin()
                .take(96 * 1024 + 1)
                .read_to_end(&mut raw)
                .is_err()
                || raw.len() > 96 * 1024
            {
                json!({"valid":false,"rejection":{"code":"envelope_too_large","message":"validation envelope exceeds 98304 bytes"}})
            } else {
                match serde_json::from_slice::<Envelope>(&raw) {
                    Ok(value) => {
                        match uhm_cli::contract::decode_and_validate(&value.tool, value.arguments) {
                            Ok(action) => json!({"valid":true,"action":action}),
                            Err(message) => {
                                json!({"valid":false,"rejection":{"code":uhm_cli::contract::rejection_code(&message),"message":message}})
                            }
                        }
                    }
                    Err(error) => {
                        json!({"valid":false,"rejection":{"code":"invalid_envelope","message":error.to_string()}})
                    }
                }
            }
        }
        "preflight" => {
            let mut raw = Vec::new();
            if std::io::stdin()
                .take(96 * 1024 + 1)
                .read_to_end(&mut raw)
                .is_err()
                || raw.len() > 96 * 1024
            {
                json!({"valid":false,"diagnostics":[{"code":"bounds_exceeded","severity":"hard_error","message":"preflight envelope exceeds 98304 bytes"}]})
            } else {
                match serde_json::from_slice::<PreflightEnvelope>(&raw) {
                    Ok(value) => {
                        match uhm_cli::contract::decode_and_validate(&value.tool, value.arguments) {
                            Ok(uhm_cli::action::ProposedAction::Program { program }) => {
                                let diagnostics = uhm_cli::program::preflight(
                                    &program,
                                    &uhm_cli::runtime::inventory(),
                                    value.piped_input_present,
                                );
                                let valid = !diagnostics.iter().any(|diagnostic| {
                                    diagnostic.severity
                                        == uhm_cli::program::DiagnosticSeverity::HardError
                                });
                                json!({"valid":valid,"diagnostics":diagnostics})
                            }
                            Ok(_) => json!({"valid":true,"diagnostics":[]}),
                            Err(message) => {
                                json!({"valid":false,"diagnostics":[{"code":uhm_cli::contract::rejection_code(&message),"severity":"hard_error","message":message}]})
                            }
                        }
                    }
                    Err(error) => {
                        json!({"valid":false,"diagnostics":[{"code":"invalid_envelope","severity":"hard_error","message":error.to_string()}]})
                    }
                }
            }
        }
        _ => {
            eprintln!("usage: uhm-bench-contract describe|qualification-context|validate-qualification-manifest|validate|preflight");
            std::process::exit(2);
        }
    };
    println!(
        "{}",
        serde_json::to_string(&result).expect("contract result is serializable")
    );
}
