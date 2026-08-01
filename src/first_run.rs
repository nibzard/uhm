//! Versioned first-use disclosure. The marker is persisted only after stderr is flushed.

use crate::{config::Config, dirs, render::ansi};
use std::io::Write;

pub const NOTICE_REVISION: u8 = 1;
pub const RENDERED_MARKER: &str = "first-use-notice-v1-rendered";

pub fn ensure(config: &Config, telemetry_enabled: bool) -> Result<&'static str, String> {
    let marker = config.paths.data_dir.join("notice-revision");
    if std::fs::read_to_string(&marker)
        .ok()
        .is_some_and(|value| value.trim() == NOTICE_REVISION.to_string())
    {
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
    let mut stderr = std::io::stderr().lock();
    writeln!(stderr, "{}", ansi::info("uhm, before we zip:"))
        .map_err(|e| format!("render first-use notice: {}", e))?;
    writeln!(
        stderr,
        "  OpenAI receives your prompt, explicitly piped input, and selected context (standard by default)."
    )
    .map_err(|e| format!("render first-use notice: {}", e))?;
    writeln!(
        stderr,
        "  Private metadata receipts are {}: at most {} records / {} days. Clear: uhm history clear.",
        receipt_state, config.history.max_records, config.history.max_age_days
    )
    .map_err(|e| format!("render first-use notice: {}", e))?;
    writeln!(
        stderr,
        "  Content-free telemetry is {} (coarse platform, route, decision, effect, outcome, latency, and cache enums). Cloudflare processes the connection. Opt out: uhm telemetry off.",
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
    write_private_atomic(&marker, NOTICE_REVISION.to_string().as_bytes())?;
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
            NOTICE_REVISION.to_string()
        );
    }
}
