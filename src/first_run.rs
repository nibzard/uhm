//! Versioned first-use disclosure. The marker is persisted only after stderr is flushed.

use crate::{
    config::Config,
    dirs,
    render::{ansi, layout},
};
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

    // Serialize the check-render-persist sequence. Without this lock, two
    // first requests can both pass the initial check and render the notice.
    dirs::ensure_private_dir(&config.paths.data_dir)?;
    let lock_path = config.paths.data_dir.join("notice.lock");
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let notice_lock = options
        .open(&lock_path)
        .map_err(|e| format!("open first-use notice lock: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        notice_lock
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("set first-use notice lock permissions: {e}"))?;
    }
    notice_lock
        .lock()
        .map_err(|e| format!("lock first-use notice: {e}"))?;
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
    let width = layout::columns().min(88);
    let sections = [
        (
            "Sends",
            format!(
                "The selected provider receives your prompt, explicitly piped input, selected context (standard by default), and Python 3 support details: {}.",
                endpoint_summary
            ),
        ),
        (
            "Repairs",
            "Only when requested: the prior model proposal and a stable contract code or coarse runtime outcome. Local-only bytes, resolved paths, credentials, and child output stay local.".into(),
        ),
        (
            "Shell",
            "Optional integration adds the parent cwd and previous status. One history entry is off by default and always previewed before sending.".into(),
        ),
        (
            "History",
            format!(
                "Private {} history is {}; up to {} events / {} days.",
                config.history.detail.as_str(),
                receipt_state,
                config.history.max_records,
                config.history.max_age_days
            ),
        ),
        (
            "Telemetry",
            format!(
                "Content-free telemetry is {}; coarse platform, route, decision, effect, outcome, latency, and cache categories. Cloudflare processes the connection.",
                telemetry_state
            ),
        ),
        (
            "Controls",
            "Opt out: uhm telemetry off; keep piped content local: --local-input; inspect: uhm context show / uhm history status; clear: uhm history clear --all".into(),
        ),
        (
            "Safety",
            "You remain responsible for actions; warnings are convenience signals, not a safety guarantee.".into(),
        ),
    ];
    let mut stderr = std::io::stderr().lock();
    let heading = if ansi::plain_enabled() {
        "uhm: first request"
    } else {
        "uhm · first request"
    };
    writeln!(stderr, "{}", ansi::primary(heading))
        .map_err(|e| format!("render first-use notice: {e}"))?;
    writeln!(
        stderr,
        "{}",
        ansi::muted("A one-time summary of what leaves this device.")
    )
    .map_err(|e| format!("render first-use notice: {e}"))?;
    writeln!(stderr).map_err(|e| format!("render first-use notice: {e}"))?;
    for (label, value) in sections {
        write_section(&mut stderr, label, &value, width)?;
    }
    writeln!(stderr).map_err(|e| format!("render first-use notice: {e}"))?;
    stderr
        .flush()
        .map_err(|e| format!("flush first-use notice: {}", e))?;

    write_private_atomic(&marker, marker_value(config).as_bytes())?;
    Ok(RENDERED_MARKER)
}

fn write_section(
    output: &mut impl Write,
    label: &str,
    value: &str,
    width: usize,
) -> Result<(), String> {
    const LABEL_WIDTH: usize = 11;
    let content_width = width.saturating_sub(LABEL_WIDTH + 3).max(8);
    for (index, line) in layout::wrap(value, content_width).iter().enumerate() {
        if index == 0 {
            let padded_label = format!("{label:<LABEL_WIDTH$}");
            writeln!(output, "  {} {}", ansi::info(&padded_label), line)
        } else {
            writeln!(output, "  {:LABEL_WIDTH$} {}", "", line)
        }
        .map_err(|e| format!("render first-use notice: {e}"))?;
    }
    Ok(())
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

    #[test]
    fn notice_sections_fit_narrow_and_wide_terminals() {
        for width in [40, 80, 160] {
            let mut output = Vec::new();
            write_section(
                &mut output,
                "Controls",
                "Inspect: uhm context show; opt out: uhm telemetry off",
                width,
            )
            .unwrap();
            let rendered = String::from_utf8(output).unwrap();
            assert!(rendered
                .lines()
                .all(|line| layout::display_width(line) <= width));
        }
    }
}
