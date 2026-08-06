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

#[test]
fn update_is_a_builtin_and_validates_usage_before_loading_api_credentials() {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(binary())
        .args(["update", "unexpected"])
        .env("HOME", temp.path())
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("XDG_DATA_HOME", temp.path().join("data"))
        .env("XDG_CACHE_HOME", temp.path().join("cache"))
        .env_remove("OPENAI_API_KEY")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("usage: uhm update"), "{stderr}");
    assert!(!stderr.contains("API key"), "{stderr}");
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

/// Same pty harness as `reviewed_with_keystrokes`, but with the child's
/// stdout redirected to a file so the command channel is not a terminal
/// while `/dev/tty` still serves the review keystrokes.
fn reviewed_with_keystrokes_stdout_to_file(
    home: &Path,
    keys: &str,
    arguments: &str,
    stdout_file: &Path,
) -> Output {
    let target = format!("{} {arguments} > {}", binary(), stdout_file.display());
    let piped = if cfg!(target_os = "macos") {
        format!("{{ printf '{keys}'; sleep 1; }} | script -q /dev/null /bin/sh -c '{target}'")
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
        rendered.contains("Run [R], revise [v], edit [e], copy [c], cancel [q]?"),
        "{rendered}"
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "original\n");
}

#[test]
fn an_affirmative_review_key_executes_the_reviewed_command() {
    if Command::new("script").arg("--version").output().is_err() && cfg!(not(target_os = "macos")) {
        return;
    }
    for keys in ["y\\n", "yes\\n"] {
        let temp = tempfile::tempdir().unwrap();
        let target = review_fixture(temp.path());
        let output = reviewed_with_keystrokes(temp.path(), keys, "--plain overwrite");
        let rendered = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "changed",
            "{keys}: {rendered}"
        );
    }
}

#[test]
fn the_advertised_cancel_key_cancels_without_executing() {
    if Command::new("script").arg("--version").output().is_err() && cfg!(not(target_os = "macos")) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let target = review_fixture(temp.path());
    let output = reviewed_with_keystrokes(temp.path(), "q\\n", "--plain overwrite");
    let rendered = String::from_utf8_lossy(&output.stdout);
    assert!(rendered.contains("cancelled by user"), "{rendered}");
    assert_eq!(fs::read_to_string(&target).unwrap(), "original\n");
}

#[test]
fn unrecognized_review_input_reprompts_once_then_cancels() {
    if Command::new("script").arg("--version").output().is_err() && cfg!(not(target_os = "macos")) {
        return;
    }
    for keys in ["n\\nn\\n", "no\\nno\\n", "zzz\\nzzz\\n"] {
        let temp = tempfile::tempdir().unwrap();
        let target = review_fixture(temp.path());
        let output = reviewed_with_keystrokes(temp.path(), keys, "--plain overwrite");
        let rendered = String::from_utf8_lossy(&output.stdout);
        assert!(
            rendered.matches("cancel [q]?").count() >= 2,
            "{keys}: expected one re-prompt restating the options: {rendered}"
        );
        assert!(rendered.contains("cancelled by user"), "{keys}: {rendered}");
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "original\n",
            "{keys}: {rendered}"
        );
    }
}

#[test]
fn a_reprompted_review_still_accepts_an_affirmative() {
    if Command::new("script").arg("--version").output().is_err() && cfg!(not(target_os = "macos")) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let target = review_fixture(temp.path());
    let output = reviewed_with_keystrokes(temp.path(), "zzz\\ny\\n", "--plain overwrite");
    let rendered = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "changed",
        "{rendered}"
    );
}

#[test]
fn review_eof_cancels_without_execution() {
    if Command::new("script").arg("--version").output().is_err() && cfg!(not(target_os = "macos")) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let target = review_fixture(temp.path());
    let output = reviewed_with_keystrokes(temp.path(), "", "--plain overwrite");
    let rendered = String::from_utf8_lossy(&output.stdout);
    assert!(
        rendered.contains("cancelled without execution"),
        "{rendered}"
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "original\n");
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
fn copying_from_review_on_a_terminal_ends_with_a_newline() {
    if Command::new("script").arg("--version").output().is_err() && cfg!(not(target_os = "macos")) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let target = review_fixture(temp.path());
    let output = reviewed_with_keystrokes(temp.path(), "c\\n", "--plain overwrite");
    let rendered = String::from_utf8_lossy(&output.stdout).replace('\r', "");
    let command = format!("printf changed > {}", target.display());
    assert!(rendered.ends_with(&format!("{command}\n")), "{rendered:?}");
    assert_eq!(fs::read_to_string(&target).unwrap(), "original\n");
}

#[test]
fn copying_from_review_into_a_pipe_keeps_the_exact_bytes() {
    if Command::new("script").arg("--version").output().is_err() && cfg!(not(target_os = "macos")) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let target = review_fixture(temp.path());
    let copied = temp.path().join("copied.txt");
    reviewed_with_keystrokes_stdout_to_file(temp.path(), "c\\n", "--plain overwrite", &copied);
    let command = format!("printf changed > {}", target.display());
    assert_eq!(fs::read(&copied).unwrap(), command.as_bytes());
    assert_eq!(fs::read_to_string(&target).unwrap(), "original\n");
}

#[test]
fn dry_run_on_a_terminal_ends_with_a_newline() {
    if Command::new("script").arg("--version").output().is_err() && cfg!(not(target_os = "macos")) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let target = review_fixture(temp.path());
    let output = reviewed_with_keystrokes(temp.path(), "", "--plain --dry-run overwrite");
    let rendered = String::from_utf8_lossy(&output.stdout).replace('\r', "");
    let command = format!("printf changed > {}", target.display());
    assert!(rendered.ends_with(&format!("{command}\n")), "{rendered:?}");
    assert_eq!(fs::read_to_string(&target).unwrap(), "original\n");
}

#[test]
fn an_open_idle_stdin_pipe_never_stalls_the_job() {
    use std::time::{Duration, Instant};
    let temp = tempfile::tempdir().unwrap();
    let mut sleeper = Command::new("sleep")
        .arg("15")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let idle_pipe = sleeper.stdout.take().unwrap();
    let start = Instant::now();
    let output = configured_command(
        temp.path(),
        "aliases:\n  idle-noop: true\n",
        &["--plain", "idle-noop"],
    )
    .stdin(Stdio::from(idle_pipe))
    .output()
    .unwrap();
    let elapsed = start.elapsed();
    let _ = sleeper.kill();
    let _ = sleeper.wait();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        elapsed < Duration::from_secs(8),
        "job took {elapsed:?} against a 15 s idle stdin producer"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("1000 ms"), "{stderr}");
    assert!(stderr.contains("without piped input"), "{stderr}");
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
    let events: Vec<serde_json::Value> = String::from_utf8_lossy(&history.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(events.iter().all(|event| {
        !event["kind"]
            .as_str()
            .is_some_and(|kind| kind.starts_with("recovery_"))
    }));
}

/// Writes a shaped recovery manifest directly into the run store, with the
/// private permissions the recovery module validates, so recovery-command
/// tests can cover states the alias-only test harness cannot reach.
fn fabricate_recovery_manifest(
    home: &Path,
    run: &str,
    state: &str,
    selection_sequence: u64,
    with_snapshot: bool,
) {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = home.join("data/uhm/runs").join(run);
    fs::create_dir_all(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    let snapshot_file = if with_snapshot {
        let snapshots = dir.join("snapshots");
        fs::create_dir_all(&snapshots).unwrap();
        fs::set_permissions(&snapshots, fs::Permissions::from_mode(0o700)).unwrap();
        let snapshot = snapshots.join("output-000.preimage");
        fs::write(&snapshot, b"before").unwrap();
        fs::set_permissions(&snapshot, fs::Permissions::from_mode(0o600)).unwrap();
        serde_json::Value::String("output-000.preimage".into())
    } else {
        serde_json::Value::Null
    };
    let captured_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let manifest = serde_json::json!({
        "schema_version": 1,
        "run_id": run,
        "created_at": captured_at,
        "updated_at": captured_at,
        "state": state,
        "pinned": false,
        "forced_restore": false,
        "selection_sequence": selection_sequence,
        "expires_at": captured_at + 14 * 86_400,
        "retirement_acknowledged": false,
        "preparation_lease_until": 0,
        "items": [{
            "id": "output-000",
            "destination": home.join("managed-target.txt"),
            "staging": home.join(".uhm-stage-managed-target.txt"),
            "existed": true,
            "snapshot_file": snapshot_file,
            "preimage_hash": null,
            "staged_hash": null,
            "postimage_hash": null,
            "preimage_bytes": 6,
            "preimage_mode": 384,
            "postimage_mode": 384,
            "device": 0,
            "inode": 0,
            "modified_seconds": 0,
            "modified_nanoseconds": 0,
            "state": "committed"
        }],
        "reason": null
    });
    let path = dir.join("recovery.json");
    fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn undo_restore_and_recover_name_the_recovery_setting_when_capture_is_off() {
    let temp = tempfile::tempdir().unwrap();
    for arguments in [
        vec!["undo", "last"],
        vec!["restore", "last", "--force"],
        vec!["recover", "last"],
    ] {
        let output = configured(temp.path(), "", &arguments);
        assert_eq!(output.status.code(), Some(13), "{arguments:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("uhm recovery on"),
            "{arguments:?}: {stderr}"
        );
        assert!(stderr.contains("--recoverable"), "{arguments:?}: {stderr}");
    }
}

#[test]
fn enabled_recovery_without_a_manifest_keeps_the_distinct_absence_message() {
    let temp = tempfile::tempdir().unwrap();
    assert!(configured(temp.path(), "", &["recovery", "on"])
        .status
        .success());
    let output = configured(temp.path(), "", &["undo", "last"]);
    assert_eq!(output.status.code(), Some(11));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no retained recovery manifest is available"),
        "{stderr}"
    );
    assert!(!stderr.contains("uhm recovery on"), "{stderr}");
}

#[test]
fn undo_last_skips_a_newer_restored_manifest_and_names_the_choice() {
    let temp = tempfile::tempdir().unwrap();
    assert!(configured(temp.path(), "", &["recovery", "on"])
        .status
        .success());
    fabricate_recovery_manifest(temp.path(), "run-restorable1", "available", 100, false);
    fabricate_recovery_manifest(temp.path(), "run-shadowing99", "restored", 200, false);
    let output = configured(temp.path(), "", &["undo", "last"]);
    assert_eq!(output.status.code(), Some(11));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("run run-restorable1"), "{stderr}");
    assert!(stderr.contains("run-shadowing99"), "{stderr}");
    assert!(stderr.contains("restored"), "{stderr}");
}

#[test]
fn non_interactive_undo_names_the_verified_and_forced_paths() {
    let temp = tempfile::tempdir().unwrap();
    assert!(configured(temp.path(), "", &["recovery", "on"])
        .status
        .success());
    fabricate_recovery_manifest(temp.path(), "run-restorable1", "available", 100, false);
    let output = configured(temp.path(), "", &["undo", "last"]);
    assert_eq!(output.status.code(), Some(11));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("uhm undo run-restorable1"), "{stderr}");
    assert!(
        stderr.contains("uhm restore run-restorable1 --force"),
        "{stderr}"
    );
}

#[test]
fn recovery_lifecycle_messages_name_the_prune_form_that_works() {
    let temp = tempfile::tempdir().unwrap();
    let on = configured(temp.path(), "", &["recovery", "on"]);
    assert!(on.status.success());
    assert!(String::from_utf8_lossy(&on.stderr).contains("uhm recovery prune --all"));
    let off = configured(temp.path(), "", &["recovery", "off"]);
    assert!(off.status.success());
    assert!(String::from_utf8_lossy(&off.stdout).contains("uhm recovery prune --all"));
}

#[test]
fn prune_reports_retained_in_cap_snapshots_and_all_removes_them() {
    let temp = tempfile::tempdir().unwrap();
    assert!(configured(temp.path(), "", &["recovery", "on"])
        .status
        .success());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    fabricate_recovery_manifest(temp.path(), "run-retained01", "available", now, true);
    let plain = configured(temp.path(), "", &["recovery", "prune"]);
    assert!(plain.status.success());
    let rendered = String::from_utf8_lossy(&plain.stdout);
    assert!(rendered.contains("pruned 0 snapshots"), "{rendered}");
    assert!(rendered.contains("uhm recovery prune --all"), "{rendered}");
    let status = configured(temp.path(), "", &["--json", "recovery", "status"]);
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["snapshots"], 1);
    let dry = configured(
        temp.path(),
        "",
        &["recovery", "prune", "--dry-run", "--all"],
    );
    assert!(dry.status.success());
    assert!(String::from_utf8_lossy(&dry.stdout).contains("would prune 1 snapshots"));
    let status = configured(temp.path(), "", &["--json", "recovery", "status"]);
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["snapshots"], 1, "dry run must remove nothing");
    let all = configured(temp.path(), "", &["recovery", "prune", "--all"]);
    assert!(all.status.success());
    let rendered = String::from_utf8_lossy(&all.stdout);
    assert!(rendered.contains("pruned 1 snapshots"), "{rendered}");
    assert!(rendered.contains("1 manifests finalized"), "{rendered}");
    assert!(!temp
        .path()
        .join("data/uhm/runs/run-retained01/recovery.json")
        .exists());
    let status = configured(temp.path(), "", &["--json", "recovery", "status"]);
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["snapshots"], 0);
    assert_eq!(value["manifests"], 0);
}

#[test]
fn history_disabled_prune_leaves_expiry_pending_without_acknowledgment() {
    let temp = tempfile::tempdir().unwrap();
    let run = "run-no-history01";
    fabricate_recovery_manifest(temp.path(), run, "available", 1, true);
    let yaml = "history:\n  enabled: false\n";

    let first = configured(temp.path(), yaml, &["recovery", "prune", "--all"]);
    assert!(first.status.success());
    let stderr = String::from_utf8_lossy(&first.stderr);
    assert!(stderr.contains("remains pending"), "{stderr}");
    assert!(
        stderr.contains("no durable RecoveryExpired event"),
        "{stderr}"
    );

    let manifest_path = temp
        .path()
        .join("data/uhm/runs")
        .join(run)
        .join("recovery.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["state"], "expired");
    assert_eq!(manifest["retirement_acknowledged"], false);
    assert!(!temp
        .path()
        .join("data/uhm/runs")
        .join(run)
        .join("snapshots/output-000.preimage")
        .exists());
    assert!(!temp.path().join("data/uhm/history.v1.jsonl").exists());

    // A later explicit management pass is still not authority to finalize
    // while event recording remains a configured no-op.
    let retry = configured(temp.path(), yaml, &["recovery", "prune", "--all"]);
    assert!(retry.status.success());
    assert!(String::from_utf8_lossy(&retry.stderr).contains("remains pending"));
    let pending: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(pending["retirement_acknowledged"], false);
}

#[test]
fn recoverable_on_the_shell_route_names_the_reason_and_stops_without_force() {
    let temp = tempfile::tempdir().unwrap();
    let yaml = "aliases:\n  local-noop: true\n";
    let output = configured(temp.path(), yaml, &["--recoverable", "local-noop"]);
    assert_eq!(output.status.code(), Some(11));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("shell execution has a receipt but no controlled preimage"),
        "{stderr}"
    );
    assert!(stderr.contains("--force"), "{stderr}");
    let forced = configured(
        temp.path(),
        yaml,
        &["--recoverable", "--force", "local-noop"],
    );
    assert_eq!(forced.status.code(), Some(0));
}

#[test]
fn shell_review_renders_the_recovery_line_when_capture_is_requested() {
    if Command::new("script").arg("--version").output().is_err() && cfg!(not(target_os = "macos")) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let target = review_fixture(temp.path());
    let output = reviewed_with_keystrokes(temp.path(), "q\\n", "--plain --recoverable overwrite");
    let rendered = String::from_utf8_lossy(&output.stdout);
    assert!(
        rendered.contains("Recovery: best_effort_only"),
        "{rendered}"
    );
    assert!(
        rendered.contains("shell execution has a receipt but no controlled preimage"),
        "{rendered}"
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "original\n");
}

#[test]
fn execution_finished_records_the_full_application_version() {
    let temp = tempfile::tempdir().unwrap();
    let yaml = "aliases:\n  local-noop: true\n";
    assert!(configured(temp.path(), yaml, &["local-noop"])
        .status
        .success());
    let history = configured(temp.path(), yaml, &["--json", "history", "show", "last"]);
    let events: Vec<serde_json::Value> = String::from_utf8_lossy(&history.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let finished = events
        .iter()
        .find(|event| event["kind"] == "execution_finished")
        .unwrap();
    assert_eq!(finished["app_version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn history_list_renders_outcome_and_local_time_without_retained_content() {
    let temp = tempfile::tempdir().unwrap();
    let yaml = "history:\n  detail: full\naliases:\n  sentinel-intent-marker: true\n";
    assert!(configured(temp.path(), yaml, &["sentinel-intent-marker"])
        .status
        .success());
    let epoch_prefix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string()[..8]
        .to_string();
    let list = configured(temp.path(), yaml, &["history", "list"]);
    assert!(list.status.success());
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(stdout.contains("success"), "outcome missing: {stdout}");
    assert!(
        !stdout.contains(&epoch_prefix),
        "raw epoch leaked: {stdout}"
    );
    let dated = stdout.split_whitespace().any(|token| {
        token.len() == 10
            && token.as_bytes()[4] == b'-'
            && token.as_bytes()[7] == b'-'
            && token.bytes().filter(u8::is_ascii_digit).count() == 8
    });
    assert!(dated, "no local date column: {stdout}");
    assert!(
        !stdout.contains("sentinel-intent-marker"),
        "retained intent leaked into the listing: {stdout}"
    );
}

#[test]
fn history_show_renders_event_blocks_by_default_and_raw_jsonl_under_json() {
    let temp = tempfile::tempdir().unwrap();
    let yaml = "history:\n  detail: full\naliases:\n  sentinel-intent-marker: true\n";
    assert!(configured(temp.path(), yaml, &["sentinel-intent-marker"])
        .status
        .success());
    let show = configured(temp.path(), yaml, &["history", "show", "last"]);
    assert!(show.status.success());
    let rendered = String::from_utf8_lossy(&show.stdout);
    assert!(rendered.contains("request_created"), "{rendered}");
    assert!(rendered.contains("execution_finished"), "{rendered}");
    assert!(rendered.contains("ago"), "no relative time: {rendered}");
    assert!(
        rendered.contains("exit_category: success"),
        "no per-kind fields: {rendered}"
    );
    assert!(
        !rendered.contains("sentinel-intent-marker"),
        "rendered view printed a field the redacted export withholds: {rendered}"
    );
    let raw = configured(temp.path(), yaml, &["--json", "history", "show", "last"]);
    assert!(raw.status.success());
    let lines: Vec<serde_json::Value> = String::from_utf8_lossy(&raw.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("every --json line is one JSON event"))
        .collect();
    assert!(!lines.is_empty());
    assert!(lines
        .iter()
        .any(|event| event["data"]["intent"] == "sentinel-intent-marker"));
}

#[test]
fn help_marks_repair_and_recover_as_requiring_retained_history() {
    let output = Command::new(binary()).arg("--help").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let repair = stdout
        .lines()
        .find(|line| line.contains("uhm repair <"))
        .expect("repair form is advertised");
    assert!(repair.contains("history.detail"), "{repair}");
    let recover = stdout
        .lines()
        .find(|line| line.contains("uhm recover <"))
        .expect("recover form is advertised");
    assert!(recover.contains("history.detail"), "{recover}");
}

#[test]
fn doctor_reports_repair_recover_and_undo_usability_on_a_default_install() {
    let temp = tempfile::tempdir().unwrap();
    let output = configured(temp.path(), "", &["--plain", "doctor"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("repair/recover"), "{stdout}");
    assert!(stdout.contains("undo/restore"), "{stdout}");
    assert!(
        stdout.contains("history.detail: full"),
        "the unusable repair/recover report must name the setting: {stdout}"
    );
    assert!(
        stdout.contains("uhm recovery on"),
        "the unusable undo report must name the command: {stdout}"
    );
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
