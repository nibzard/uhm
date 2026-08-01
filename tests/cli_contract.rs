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
