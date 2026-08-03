//! Versioned first-use disclosure. The marker is persisted only after stderr is flushed.

use crate::{config::Config, dirs, render::ansi};
use std::io::Write;

pub const NOTICE_REVISION: u8 = 5;
pub const RENDERED_MARKER: &str = "first-use-notice-v5-rendered";

fn marker_value(config: &Config) -> String {
    let mut endpoints = vec![config.provider.adapter().endpoint()];
    if let Some(alternate) = &config.selection.alternate {
        endpoints.push(alternate.provider.adapter().endpoint());
    }
    endpoints.sort_unstable();
    endpoints.dedup();
    serde_json::json!({"revision":NOTICE_REVISION,"endpoints":endpoints}).to_string()
}

pub fn is_current(config: &Config) -> bool {
    std::fs::read_to_string(config.paths.data_dir.join("notice-revision"))
        .ok()
        .is_some_and(|value| value.trim() == marker_value(config))
}

pub fn ensure(config: &Config, telemetry_enabled: bool) -> Result<&'static str, String> {
    let marker = config.paths.data_dir.join("notice-revision");
    if is_current(config) {
        return Ok(RENDERED_MARKER);
    }

    let telemetry_state = if telemetry_enabled {
        "on"
    } else {
        "off for this invocation"
    };
    let receipt_state = if config.history.enabled {
        "on"
    } else {
        "off in config"
    };
    let primary = config.provider.adapter();
    let endpoint_summary = if let Some(alternate) = &config.selection.alternate {
        format!(
            "{} ({}) with authorized alternate {} ({})",
            config.provider,
            primary.endpoint(),
            alternate.provider,
            alternate.provider.adapter().endpoint()
        )
    } else {
        format!("{} ({})", config.provider, primary.endpoint())
    };
    let mut stderr = std::io::stderr().lock();
    writeln!(stderr, "{}", ansi::info("uhm, before we zip:"))
        .map_err(|e| format!("render first-use notice: {}", e))?;
    writeln!(
        stderr,
        "  The selected provider receives your prompt, explicitly piped input, and selected context (standard by default): {}.", endpoint_summary
    )
    .map_err(|e| format!("render first-use notice: {}", e))?;
    writeln!(
        stderr,
        "  Python 3 path/version support may be sent for program routing. Use --local-input to keep piped content local to the generated program."
    )
    .map_err(|e| format!("render first-use notice: {}", e))?;
    writeln!(stderr,"  If you explicitly request a program repair, the accepted proposal's provider receives the prior model-authored proposal plus a stable contract code or coarse runtime outcome; local-only bytes, resolved paths, credentials, and child output stay local.")
        .map_err(|e| format!("render first-use notice: {}",e))?;
    writeln!(stderr,"  Optional shell integration adds only invocation-time parent cwd and previous status. One-entry shell history is off by default and always previewed before sending.")
        .map_err(|e| format!("render first-use notice: {}",e))?;
    writeln!(
        stderr,
        "  Private {} history is {}: at most {} events / {} days. Inspect: uhm history status. Clear: uhm history clear --all.",
        config.history.detail.as_str(),
        receipt_state, config.history.max_records, config.history.max_age_days
    )
    .map_err(|e| format!("render first-use notice: {}", e))?;
    writeln!(
        stderr,
        "  Content-free telemetry is {} (coarse platform, route, decision, effect, process/parent outcome, latency, and cache enums). Cloudflare processes the connection. Opt out: uhm telemetry off.",
        telemetry_state
    )
    .map_err(|e| format!("render first-use notice: {}", e))?;
    writeln!(
        stderr,
        "  You remain responsible for actions; warnings are convenience signals, not a safety guarantee."
    )
    .map_err(|e| format!("render first-use notice: {}", e))?;
    stderr
        .flush()
        .map_err(|e| format!("flush first-use notice: {}", e))?;

    dirs::ensure_private_dir(&config.paths.data_dir)?;
    write_private_atomic(&marker, marker_value(config).as_bytes())?;
    Ok(RENDERED_MARKER)
}

fn write_private_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("notice marker has no parent")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("create notice marker: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("set notice marker permissions: {}", e))?;
    }
    file.write_all(bytes)
        .map_err(|e| format!("write notice marker: {}", e))?;
    file.as_file()
        .sync_all()
        .map_err(|e| format!("sync notice marker: {}", e))?;
    file.persist(path)
        .map_err(|e| format!("publish notice marker: {}", e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dirs::Paths;

    #[test]
    fn marker_is_versioned_and_private() {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::test(Paths {
            config_file: root.path().join("config"),
            data_dir: root.path().join("data"),
            cache_dir: root.path().join("cache"),
        });
        config.history.max_records = 7;
        assert_eq!(ensure(&config, true).unwrap(), RENDERED_MARKER);
        assert_eq!(
            std::fs::read_to_string(root.path().join("data/notice-revision")).unwrap(),
            marker_value(&config)
        );
        assert!(is_current(&config));
        config.selection.alternate = Some(crate::config::ModelCandidate {
            provider: crate::provider::ProviderId::Cerebras,
            model: "gpt-oss-120b".into(),
        });
        assert!(
            !is_current(&config),
            "adding a disclosed endpoint must require a new notice"
        );
    }
}
