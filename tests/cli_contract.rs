use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_uhm")
}

fn configured(home: &Path, yaml: &str, arguments: &[&str]) -> Output {
    let config_dir = home.join("config/uhm");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("config.yaml"), yaml).unwrap();
    let data_dir = home.join("data/uhm");
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(data_dir.join("notice-revision"), "3").unwrap();
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

#[test]
fn prose_only_routes_never_execute_a_local_alias() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("must-not-exist");
    let command = format!("touch {}", marker.display());
    let yaml = format!("aliases:\n  inspect: '{}'\n", command.replace('\'', "''"));

    for route in ["ask", "explain"] {
        let output = configured(temp.path(), &yaml, &[route, "inspect"]);
        assert_eq!(output.status.code(), Some(10));
        assert!(String::from_utf8_lossy(&output.stderr).contains("prose-only"));
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
    assert!(notice.contains("OpenAI receives"));
    assert!(notice.contains("uhm telemetry off"));
    assert!(notice.contains("not a safety guarantee"));

    let second = configured(
        temp.path(),
        "aliases:\n  notice-noop: true\n",
        &["--plain", "notice-noop"],
    );
    assert!(!String::from_utf8_lossy(&second.stderr).contains("OpenAI receives"));
}

#[test]
fn plain_first_use_notice_contains_no_terminal_controls() {
    let temp = tempfile::tempdir().unwrap();
    let output = configured_fresh(temp.path(), "", &["--plain", "config", "show"]);
    assert!(!output.stderr.contains(&0x1b));
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
