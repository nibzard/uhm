// ABOUTME: Runs every fenced `uhm` example from README.md and docs/ under both
// ABOUTME: `zsh -c` and `bash -c` against a stub uhm on PATH, failing on expansion errors.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Shell diagnostics that mean the command line broke before or instead of
/// reaching `uhm`. The defect class lives at expansion time, so `zsh -n`
/// cannot see it; only live execution under `-c` can.
const EXPANSION_MARKERS: &[&str] = &[
    "no matches found",
    "bad pattern",
    "unmatched",
    "event not found",
    "command not found",
    "unexpected EOF",
    "parse error",
];

struct Example {
    file: String,
    line: usize,
    command: String,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn markdown_files(root: &Path) -> Vec<PathBuf> {
    let mut files = vec![root.join("README.md")];
    collect_markdown(&root.join("docs"), &mut files);
    files.sort();
    files
}

fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_markdown(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "md") {
            out.push(path);
        }
    }
}

fn is_shell_fence(info: &str) -> bool {
    matches!(info, "sh" | "bash" | "zsh" | "shell")
}

/// A line invokes `uhm` when the word sits in command position: first word,
/// after a pipe, or inside a command substitution.
fn invokes_uhm(command: &str) -> bool {
    command == "uhm"
        || command.starts_with("uhm ")
        || command.contains("| uhm ")
        || command.contains("|uhm ")
        || command.contains("$(uhm ")
        || command.contains("(uhm ")
}

/// Synopsis placeholders such as `<run-id|last>` or `[run-id]` mark lines the
/// reader substitutes into, not lines meant to be copied verbatim.
fn has_placeholder(command: &str) -> bool {
    let bytes = command.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'<'
            && bytes
                .get(index + 1)
                .is_some_and(|next| next.is_ascii_alphabetic())
            && bytes[index + 1..].contains(&b'>')
        {
            return true;
        }
        if *byte == b'[' {
            if let Some(length) = bytes[index + 1..].iter().position(|inner| *inner == b']') {
                let inner = &command[index + 1..index + 1 + length];
                if !inner.is_empty()
                    && inner.bytes().any(|c| c.is_ascii_alphabetic())
                    && inner
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
                {
                    return true;
                }
            }
        }
    }
    false
}

fn fenced_examples(path: &Path, root: &Path) -> Vec<Example> {
    let text = fs::read_to_string(path).unwrap();
    let file = path
        .strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string();
    let lines: Vec<&str> = text.lines().collect();
    let mut examples = Vec::new();
    let mut fence: Option<String> = None;
    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if let Some(info) = trimmed.strip_prefix("```") {
            fence = match fence {
                Some(_) => None,
                None => Some(info.trim().to_string()),
            };
            index += 1;
            continue;
        }
        let Some(info) = &fence else {
            index += 1;
            continue;
        };
        if !is_shell_fence(info) || trimmed.is_empty() || trimmed.starts_with('#') {
            index += 1;
            continue;
        }
        let line = index + 1;
        let mut command = trimmed.to_string();
        while command.ends_with('\\') && index + 1 < lines.len() {
            command.pop();
            index += 1;
            command.push(' ');
            command.push_str(lines[index].trim());
        }
        if invokes_uhm(&command) && !has_placeholder(&command) {
            examples.push(Example {
                file: file.clone(),
                line,
                command,
            });
        }
        index += 1;
        continue;
    }
    examples
}

fn write_stub(directory: &Path, name: &str, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join(name);
    fs::write(&path, body).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn zsh_available() -> bool {
    Command::new("zsh")
        .args(["-f", "-c", "exit 0"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

#[test]
fn documented_uhm_examples_survive_zsh_and_bash_expansion() {
    let root = repo_root();
    let mut examples = Vec::new();
    for path in markdown_files(&root) {
        examples.extend(fenced_examples(&path, &root));
    }
    assert!(
        examples.len() >= 30,
        "extraction found only {} fenced uhm examples; the extractor regressed",
        examples.len()
    );

    let temp = tempfile::tempdir().unwrap();
    let stub_dir = temp.path().join("bin");
    fs::create_dir_all(&stub_dir).unwrap();
    let log = temp.path().join("uhm-argv.log");
    // The stub records that it was reached and absorbs any piped stdin.
    write_stub(
        &stub_dir,
        "uhm",
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$UHM_STUB_LOG\"\ncat >/dev/null\nexit 0\n",
    );
    // Producers documented on the left of a pipe must not touch the network
    // or require repository state; the assertions only concern expansion.
    write_stub(&stub_dir, "git", "#!/bin/sh\nexit 0\n");
    write_stub(&stub_dir, "curl", "#!/bin/sh\nexit 0\n");

    // The working directory deliberately contains no `*.md` match, so an
    // unquoted glob in a documented intent fails under zsh here exactly as it
    // does in a user's empty directory. `cat` of an absent file is tolerated:
    // the pipeline still reaches the stub and reports no expansion error.
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    for fixture in ["data.csv", "tutorial-data.csv", "report.txt"] {
        fs::write(workspace.join(fixture), "amount\n42\n").unwrap();
    }

    let path_value = format!("{}:/usr/bin:/bin", stub_dir.display());
    let mut shells: Vec<(&str, Vec<&str>)> = vec![("bash", vec!["-c"])];
    if zsh_available() {
        // -f skips rc files so the result reflects zsh defaults, where an
        // unmatched glob is a hard error before the command runs.
        shells.push(("zsh", vec!["-f", "-c"]));
    } else {
        eprintln!("skipping zsh: no zsh on this runner; bash coverage still applies");
    }

    let mut failures = Vec::new();
    for (shell, shell_arguments) in &shells {
        for example in &examples {
            let _ = fs::remove_file(&log);
            let output = Command::new(shell)
                .args(shell_arguments)
                .arg(&example.command)
                .current_dir(&workspace)
                .env("PATH", &path_value)
                .env("UHM_STUB_LOG", &log)
                .env_remove("BASH_ENV")
                .stdin(Stdio::null())
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            let marker = EXPANSION_MARKERS
                .iter()
                .find(|marker| stderr.contains(**marker));
            let reached = fs::metadata(&log)
                .map(|meta| meta.len() > 0)
                .unwrap_or(false);
            if marker.is_some() || !reached {
                failures.push(format!(
                    "{} rejected {}:{}: `{}`\n  status: {:?}, uhm reached: {}, stderr: {}",
                    shell,
                    example.file,
                    example.line,
                    example.command,
                    output.status.code(),
                    reached,
                    stderr.trim()
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "documented uhm examples broke under live shell expansion:\n{}",
        failures.join("\n")
    );
}
