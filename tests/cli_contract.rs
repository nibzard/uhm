use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_uhm")
}

fn configured_command(home: &Path, yaml: &str, arguments: &[&str]) -> Command {
    let config_dir = home.join("config/uhm");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("config.yaml"), yaml).unwrap();
    let data_dir = home.join("data/uhm");
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(
        data_dir.join("notice-revision"),
        r#"{"endpoints":["https://api.openai.com/v1/responses"],"revision":5}"#,
    )
    .unwrap();
    let mut command = Command::new(binary());
    command
        .args(arguments)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("TERM", "dumb");
    command
}

fn configured(home: &Path, yaml: &str, arguments: &[&str]) -> Output {
    configured_command(home, yaml, arguments).output().unwrap()
}

fn configured_fresh(home: &Path, yaml: &str, arguments: &[&str]) -> Output {
    let config_dir = home.join("config/uhm");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("config.yaml"), yaml).unwrap();
    Command::new(binary())
        .args(arguments)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("TERM", "dumb")
        .output()
        .unwrap()
}

#[cfg(target_os = "linux")]
fn reviewed_through_closed_pty(home: &Path, arguments: &str) -> Output {
    let command = format!("{} {arguments}", binary());
    Command::new("script")
        .args(["-qefc", &command, "/dev/null"])
        .stdin(Stdio::null())
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("TERM", "dumb")
        .output()
        .unwrap()
}

/// Drive an interactive review through a real pty. `read_line_cooked` opens
/// `/dev/tty`, so keystrokes must reach the controlling terminal rather than
/// stdin, and the writing end must stay open until the child has read.
fn reviewed_with_keystrokes(home: &Path, keys: &str, arguments: &str) -> Output {
    let target = format!("{} {arguments}", binary());
    let piped = if cfg!(target_os = "macos") {
        format!("{{ printf '{keys}'; sleep 1; }} | script -q /dev/null {target}")
    } else {
        format!("{{ printf '{keys}'; sleep 1; }} | script -qefc '{target}' /dev/null")
    };
    Command::new("/bin/sh")
        .args(["-c", &piped])
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("TERM", "dumb")
        .output()
        .unwrap()
}

fn review_fixture(temp: &Path) -> std::path::PathBuf {
    let target = temp.join("existing.txt");
    fs::write(&target, "original\n").unwrap();
    let config_dir = temp.join("config/uhm");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.yaml"),
        format!(
            "aliases:\n  overwrite: 'printf changed > {}'\n",
            target.display()
        ),
    )
    .unwrap();
    let data_dir = temp.join("data/uhm");
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(
        data_dir.join("notice-revision"),
        r#"{"endpoints":["https://api.openai.com/v1/responses"],"revision":5}"#,
    )
    .unwrap();
    target
}

#[test]
fn review_advertises_every_option_it_can_honor() {
    if Command::new("script").arg("--version").output().is_err() && cfg!(not(target_os = "macos")) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let target = review_fixture(temp.path());
    let output = reviewed_with_keystrokes(temp.path(), "q\\n", "--plain overwrite");
    let rendered = String::from_utf8_lossy(&output.stdout);
    assert!(
        rendered.contains("Run, revise, edit, copy, cancel? [R/v/e/c/q]"),
        "{rendered}"
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "original\n");
}

#[test]
fn an_unrecognized_review_key_cancels_without_executing() {
    if Command::new("script").arg("--version").output().is_err() && cfg!(not(target_os = "macos")) {
        return;
    }
    for keys in ["q\\n", "zzz\\n"] {
        let temp = tempfile::tempdir().unwrap();
        let target = review_fixture(temp.path());
        let output = reviewed_with_keystrokes(temp.path(), keys, "--plain overwrite");
        let rendered = String::from_utf8_lossy(&output.stdout);
        assert!(rendered.contains("cancelled by user"), "{keys}: {rendered}");
        assert_eq!(fs::read_to_string(&target).unwrap(), "original\n");
    }
}

#[test]
fn copying_from_review_emits_the_command_without_executing_it() {
    if Command::new("script").arg("--version").output().is_err() && cfg!(not(target_os = "macos")) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let target = review_fixture(temp.path());
    let output = reviewed_with_keystrokes(temp.path(), "c\\n", "--plain overwrite");
    let rendered = String::from_utf8_lossy(&output.stdout);
    assert!(rendered.contains("printf changed >"), "{rendered}");
    assert_eq!(fs::read_to_string(&target).unwrap(), "original\n");
}

#[test]
fn exact_child_status_wins() {
    let temp = tempfile::tempdir().unwrap();
    let output = configured(
        temp.path(),
        "aliases:\n  child-seven: exit 7\n",
        &["child-seven"],
    );
    assert_eq!(output.status.code(), Some(7));
}

#[test]
fn signal_status_uses_shell_convention() {
    let temp = tempfile::tempdir().unwrap();
    let output = configured(
        temp.path(),
        "aliases:\n  signal-self: kill -TERM $$\n",
        &["--force", "signal-self"],
    );
    assert_eq!(output.status.code(), Some(143));
}

#[test]
fn forced_consequential_json_is_one_machine_readable_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let output = configured(
        temp.path(),
        "aliases:\n  signal-self: kill -TERM $$\n",
        &["--json", "--force", "signal-self"],
    );
    assert_eq!(output.status.code(), Some(143));
    let receipt: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(receipt["namespace"], "uhm.child");
    assert!(receipt["message"]
        .as_str()
        .unwrap()
        .contains("controls processes"));
}

#[test]
fn json_disambiguates_an_executed_child_on_stderr() {
    let temp = tempfile::tempdir().unwrap();
    let output = configured(
        temp.path(),
        "aliases:\n  child-seven: exit 7\n",
        &["--json", "child-seven"],
    );
    assert_eq!(output.status.code(), Some(7));
    assert!(output.stdout.is_empty());
    let receipt: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(receipt["namespace"], "uhm.child");
    assert_eq!(receipt["exit_code"], 7);
    assert_eq!(receipt["executed"], true);
}

#[test]
fn nonexecuted_review_is_nonzero_and_contains_no_controls() {
    let temp = tempfile::tempdir().unwrap();
    let output = configured(
        temp.path(),
        "aliases:\n  inspect: printf hello\n",
        &["--plain", "--review", "inspect"],
    );
    assert_eq!(output.status.code(), Some(11));
    assert!(!output.stderr.contains(&0x1b));
    assert!(String::from_utf8_lossy(&output.stderr).contains("confirmation is required"));
}

#[cfg(target_os = "linux")]
#[test]
fn tty_eof_cancels_explicit_and_automatic_review_without_execution() {
    if Command::new("script").arg("--version").output().is_err() {
        return;
    }
    for arguments in ["--plain --review overwrite", "--plain overwrite"] {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("existing.txt");
        fs::write(&target, "original\n").unwrap();
        let config_dir = temp.path().join("config/uhm");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.yaml"),
            format!(
                "aliases:\n  overwrite: 'printf changed > {}'\n",
                target.display()
            ),
        )
        .unwrap();
        let data_dir = temp.path().join("data/uhm");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(
            data_dir.join("notice-revision"),
            r#"{"endpoints":["https://api.openai.com/v1/responses"],"revision":5}"#,
        )
        .unwrap();

        let output = reviewed_through_closed_pty(temp.path(), arguments);
        assert_eq!(output.status.code(), Some(11), "{arguments}");
        assert_eq!(fs::read_to_string(&target).unwrap(), "original\n");
        assert!(String::from_utf8_lossy(&output.stdout).contains("cancelled without execution"));
    }
}

#[test]
fn metadata_mutations_pause_without_a_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("existing.txt");
    fs::write(&target, "original\n").unwrap();
    let yaml = format!(
        "aliases:\n  retime: 'touch {}'\n  remode: 'chmod 600 {}'\n",
        target.display(),
        target.display()
    );
    for alias in ["retime", "remode"] {
        let output = configured(temp.path(), &yaml, &["--plain", alias]);
        assert_eq!(output.status.code(), Some(11), "{alias}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("no terminal is available"));
    }
}

#[test]
fn local_parent_shell_alias_diagnostic_names_the_local_source() {
    for command in ["cd /tmp", "export UHM_TEST=1", "source ./env.sh"] {
        let temp = tempfile::tempdir().unwrap();
        let output = configured(
            temp.path(),
            &format!("aliases:\n  parent: '{command}'\n"),
            &["--plain", "parent"],
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(11));
        assert!(stderr.contains("local alias contains parent-shell source"));
        assert!(!stderr.contains("model returned"));
    }
}

#[test]
fn doctor_environment_lists_names_never_values() {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(binary())
        .args(["--json", "doctor", "environment"])
        .env("HOME", temp.path())
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("XDG_DATA_HOME", temp.path().join("data"))
        .env("XDG_CACHE_HOME", temp.path().join("cache"))
        .env("AWS_SECRET_ACCESS_KEY", "must-never-appear")
        .output()
        .unwrap();
    assert!(output.status.success());
    let body = String::from_utf8(output.stdout).unwrap();
    assert!(body.contains("AWS_SECRET_ACCESS_KEY"));
    assert!(!body.contains("must-never-appear"));
}

#[test]
fn requested_missing_containment_fails_before_shell_execution() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("must-not-exist");
    let yaml = format!(
        "execution:\n  containment: bubblewrap\naliases:\n  contained: 'touch {}'\n",
        marker.display()
    );
    let output = configured_command(temp.path(), &yaml, &["--force", "contained"])
        .env("PATH", temp.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(11));
    let expected = if cfg!(target_os = "linux") {
        "bwrap` is not available"
    } else {
        "available only on Linux"
    };
    assert!(String::from_utf8_lossy(&output.stderr).contains(expected));
    assert!(!marker.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn bubblewrap_containment_executes_inside_the_working_directory() {
    let probe = Command::new("bwrap")
        .args(["--ro-bind", "/", "/", "--", "/bin/true"])
        .output();
    if !probe.is_ok_and(|output| output.status.success()) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let output = configured(
        temp.path(),
        "execution:\n  containment: bubblewrap\naliases:\n  contained: 'printf contained'\n",
        &["contained"],
    );
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(output.stdout, b"contained");
}

#[test]
fn prose_only_routes_never_execute_a_local_alias() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("must-not-exist");
    let command = format!("touch {}", marker.display());
    let yaml = format!("aliases:\n  inspect: '{}'\n", command.replace('\'', "''"));

    for route in ["ask", "explain"] {
        let output = configured(temp.path(), &yaml, &[route, "inspect"]);
        assert_eq!(output.status.code(), Some(10));
        assert!(String::from_utf8_lossy(&output.stderr).contains("cannot execute local actions"));
        assert!(!marker.exists());
    }
}

#[test]
fn dry_run_command_channel_is_exact() {
    let temp = tempfile::tempdir().unwrap();
    let command = "printf  '%s\\n'  'snow 雪'";
    let yaml = format!("aliases:\n  exact: '{}'\n", command.replace('\'', "''"));
    let output = configured(temp.path(), &yaml, &["--dry-run", "exact"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, command.as_bytes());
    assert!(output.stderr.is_empty());
}

#[test]
fn invalid_config_stops_before_work() {
    let temp = tempfile::tempdir().unwrap();
    let output = configured(
        temp.path(),
        "model: test\nmodle: typo\n",
        &["config", "check"],
    );
    assert_eq!(output.status.code(), Some(13));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("config.yaml"));
    assert!(error.contains("unknown field"));
}

#[test]
fn json_configuration_error_is_namespaced() {
    let temp = tempfile::tempdir().unwrap();
    let output = configured(
        temp.path(),
        "unknown_setting: true\n",
        &["--json", "config", "check"],
    );
    assert_eq!(output.status.code(), Some(13));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["namespace"], "uhm");
    assert_eq!(value["outcome"], "configuration_error");
    assert!(output.stderr.is_empty());
}

#[test]
fn successful_management_json_is_namespaced() {
    let temp = tempfile::tempdir().unwrap();
    let output = configured(
        temp.path(),
        "model: test-model\n",
        &["--json", "config", "show"],
    );
    assert_eq!(output.status.code(), Some(0));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["namespace"], "uhm");
    assert_eq!(value["outcome"], "config");
    assert_eq!(value["data"]["values"]["model"], "test-model");
    assert!(output.stderr.is_empty());
}

#[test]
fn doctor_reports_thirteen_when_a_non_optional_check_fails() {
    // `uhm doctor` exit reflects overall health (Fix 2): on a keyless supported
    // host the API-key check is "missing", which is not benign, so both the
    // process exit and the JSON `exit_code` must be 13. This pins the main.rs
    // `doctor::healthy(&report)` -> exit wiring that the pure unit tests of
    // `healthy()` never reach.
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("config/uhm");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("config.yaml"), "model: test-model\n").unwrap();
    let output = Command::new(binary())
        .args(["--json", "doctor"])
        .env("HOME", temp.path())
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("XDG_DATA_HOME", temp.path().join("data"))
        .env("XDG_CACHE_HOME", temp.path().join("cache"))
        .env("TERM", "dumb")
        .env_remove("OPENAI_API_KEY")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(13));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["namespace"], "uhm");
    assert_eq!(value["outcome"], "doctor");
    assert_eq!(value["exit_code"], 13);
    let missing = value["data"]["checks"]
        .as_array()
        .expect("doctor report has a checks array")
        .iter()
        .any(|check| check["status"].as_str() == Some("missing"));
    assert!(missing, "a keyless host reports at least one failing check");
}

#[test]
fn unresolved_paths_never_fall_back_to_the_working_directory() {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(binary())
        .args(["config", "check"])
        .current_dir(temp.path())
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CACHE_HOME")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(13));
    assert!(!temp.path().join("uhm").exists());
}

#[test]
fn removed_short_authority_flag_is_a_usage_error() {
    let output = Command::new(binary())
        .args(["-y", "do", "work"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown option"));
}

#[test]
fn shell_init_is_static_and_needs_no_configuration_or_notice() {
    let output = Command::new(binary())
        .args(["shell-init", "bash"])
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("integration v1"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("__uhm_binary"));
    assert!(output.stderr.is_empty());

    let extra = Command::new(binary())
        .args(["shell-init", "bash", "extra"])
        .env_remove("HOME")
        .output()
        .unwrap();
    assert_eq!(extra.status.code(), Some(2));
    assert!(extra.stdout.is_empty());
}

#[test]
fn one_entry_shell_history_is_off_and_cancellation_precedes_any_model_request() {
    let temp = tempfile::tempdir().unwrap();
    let disabled = configured(
        temp.path(),
        "",
        &["--uhm-last-history", "private sentinel", "fix", "it"],
    );
    assert_eq!(disabled.status.code(), Some(2));
    assert!(!String::from_utf8_lossy(&disabled.stderr).contains("private sentinel"));

    let enabled = configured(
        temp.path(),
        "shell_context:\n  last_history_entry: true\n",
        &["--uhm-last-history", "private sentinel", "fix", "it"],
    );
    assert_eq!(enabled.status.code(), Some(11));
    let stderr = String::from_utf8_lossy(&enabled.stderr);
    assert!(stderr.contains("private sentinel"));
    assert!(stderr.contains("terminal is required"));
    assert!(!stderr.contains("API key"));
}

#[test]
fn first_use_notice_precedes_work_and_is_rendered_once() {
    let temp = tempfile::tempdir().unwrap();
    let first = configured_fresh(
        temp.path(),
        "aliases:\n  notice-noop: true\n",
        &["--plain", "notice-noop"],
    );
    assert_eq!(first.status.code(), Some(0));
    let notice = String::from_utf8_lossy(&first.stderr);
    assert!(notice.contains("selected provider receives"));
    assert!(notice.contains("uhm telemetry off"));
    assert!(notice.contains("not a safety guarantee"));

    let second = configured(
        temp.path(),
        "aliases:\n  notice-noop: true\n",
        &["--plain", "notice-noop"],
    );
    assert!(!String::from_utf8_lossy(&second.stderr).contains("selected provider receives"));
}

#[test]
fn concurrent_first_use_renders_exactly_one_notice() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("config/uhm");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.yaml"),
        "aliases:\n  notice-noop: true\n",
    )
    .unwrap();

    let mut children = Vec::new();
    for _ in 0..20 {
        children.push(
            Command::new(binary())
                .args(["--plain", "notice-noop"])
                .env("HOME", temp.path())
                .env("XDG_CONFIG_HOME", temp.path().join("config"))
                .env("XDG_DATA_HOME", temp.path().join("data"))
                .env("XDG_CACHE_HOME", temp.path().join("cache"))
                .env("TERM", "dumb")
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    let outputs: Vec<_> = children
        .into_iter()
        .map(|child| child.wait_with_output().unwrap())
        .collect();
    assert!(outputs.iter().all(|output| output.status.success()));
    let notices = outputs
        .iter()
        .filter(|output| {
            String::from_utf8_lossy(&output.stderr).contains("selected provider receives")
        })
        .count();
    assert_eq!(notices, 1);
}

#[test]
fn local_command_on_a_fresh_config_skips_the_first_use_notice() {
    // `config show` does no outbound work, so on a fresh install it must neither
    // print the first-use data notice nor persist its marker (Fix 4). Asserting
    // the notice text is absent — not just the ESC byte — pins the contract: a
    // plain-mode notice contains no 0x1b either way, so the byte check alone is
    // blind to a regression that re-renders the notice for local commands.
    let temp = tempfile::tempdir().unwrap();
    let output = configured_fresh(temp.path(), "", &["--plain", "config", "show"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(!output.stderr.contains(&0x1b));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("selected provider receives"),
        "local command must not render the first-use notice: {stderr:?}"
    );
    assert!(
        !temp.path().join("data/uhm/notice-revision").exists(),
        "local command must not persist the notice marker"
    );
}

#[test]
fn history_replay_restores_the_notice_gate_before_completion_telemetry() {
    let temp = tempfile::tempdir().unwrap();
    let yaml = "history:\n  detail: diagnostic\naliases:\n  replay-noop: true\n";
    let original = configured_fresh(temp.path(), yaml, &["--plain", "replay-noop"]);
    assert_eq!(original.status.code(), Some(0));

    let marker = temp.path().join("data/uhm/notice-revision");
    fs::remove_file(&marker).unwrap();
    let replay = configured_fresh(
        temp.path(),
        yaml,
        &["--plain", "history", "replay", "last", "--review"],
    );

    assert_eq!(replay.status.code(), Some(11));
    assert!(
        String::from_utf8_lossy(&replay.stderr).contains("selected provider receives"),
        "replay must render the disclosure before its completion path can send telemetry"
    );
    assert!(marker.exists(), "replay must persist the notice marker");
}

#[test]
fn recovery_is_off_by_default_and_enablement_is_explicit() {
    let temp = tempfile::tempdir().unwrap();
    let status = configured(temp.path(), "", &["--json", "recovery", "status"]);
    assert!(status.status.success());
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["enabled"], false);
    let enabled = configured(temp.path(), "", &["recovery", "on"]);
    assert!(enabled.status.success());
    assert!(String::from_utf8_lossy(&enabled.stderr)
        .contains("duplicates eligible managed file preimages"));
    let status = configured(temp.path(), "", &["--json", "recovery", "status"]);
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["enabled"], true);
}

#[test]
fn forced_restore_requires_the_literal_authority_flag() {
    let temp = tempfile::tempdir().unwrap();
    let output = configured(temp.path(), "", &["restore", "last"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("literal --force"));
}

#[test]
fn disabled_recovery_creates_no_recovery_timeline_events() {
    let temp = tempfile::tempdir().unwrap();
    let run = configured(
        temp.path(),
        "aliases:\n  local-noop: true\n",
        &["local-noop"],
    );
    assert!(run.status.success());
    let history = configured(
        temp.path(),
        "aliases:\n  local-noop: true\n",
        &["--json", "history", "show", "last"],
    );
    assert!(history.status.success());
    let events: serde_json::Value = serde_json::from_slice(&history.stdout).unwrap();
    assert!(events.as_array().unwrap().iter().all(|event| {
        !event["kind"]
            .as_str()
            .is_some_and(|kind| kind.starts_with("recovery_"))
    }));
}

#[test]
fn execution_finished_records_the_full_application_version() {
    let temp = tempfile::tempdir().unwrap();
    let yaml = "aliases:\n  local-noop: true\n";
    assert!(configured(temp.path(), yaml, &["local-noop"])
        .status
        .success());
    let history = configured(temp.path(), yaml, &["--json", "history", "show", "last"]);
    let events: serde_json::Value = serde_json::from_slice(&history.stdout).unwrap();
    let finished = events
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["kind"] == "execution_finished")
        .unwrap();
    assert_eq!(finished["app_version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn one_invocation_reports_identical_history_corruption_once() {
    let temp = tempfile::tempdir().unwrap();
    let yaml = "aliases:\n  local-noop: true\n";
    assert!(configured(temp.path(), yaml, &["local-noop"])
        .status
        .success());
    let journal = temp.path().join("data/uhm/history.v1.jsonl");
    let mut bytes = fs::read(&journal).unwrap();
    bytes[0] = b'[';
    fs::write(&journal, bytes).unwrap();

    let output = configured(temp.path(), yaml, &["local-noop"]);
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stderr.matches("history corruption at line 1").count(), 1);
}
