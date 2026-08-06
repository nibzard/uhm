//! Standalone release updater. This deliberately runs before configuration
//! loading: updating UHM never requires a model-provider credential.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/nibzard/uhm/releases/latest";
const INSTALLER: &[u8] = include_bytes!("../docs/install.sh");
const MAX_METADATA_BYTES: u64 = 256 * 1024;

#[derive(Debug, Serialize)]
pub struct UpdateResult {
    pub outcome: &'static str,
    pub current_version: String,
    pub latest_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_path: Option<PathBuf>,
    #[serde(skip)]
    installer_stdout: Vec<u8>,
    #[serde(skip)]
    installer_stderr: Vec<u8>,
}

impl UpdateResult {
    pub fn render(&self) {
        if !self.installer_stdout.is_empty() {
            let _ = std::io::stdout().write_all(&self.installer_stdout);
        }
        if !self.installer_stderr.is_empty() {
            let _ = std::io::stderr().write_all(&self.installer_stderr);
        }
        match self.outcome {
            "updated" => println!(
                "uhm updated: {} -> {}",
                self.current_version, self.latest_version
            ),
            "ahead" => println!(
                "uhm {} is newer than the latest published release {}; not downgrading",
                self.current_version, self.latest_version
            ),
            _ => println!("uhm {} is already up to date", self.current_version),
        }
    }
}

#[derive(Debug, Deserialize)]
struct LatestRelease {
    tag_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Version {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Option<String>,
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.prerelease, &other.prerelease) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(left), Some(right)) => left.cmp(right),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn parse_version(value: &str) -> Result<Version, String> {
    let value = value.strip_prefix('v').unwrap_or(value);
    let value = value.split_once('+').map_or(value, |(core, _)| core);
    let (core, prerelease) = value
        .split_once('-')
        .map_or((value, None), |(core, suffix)| (core, Some(suffix)));
    let mut fields = core.split('.');
    let parse = |field: Option<&str>| {
        field
            .filter(|field| !field.is_empty())
            .ok_or_else(|| format!("invalid release version: {value}"))?
            .parse::<u64>()
            .map_err(|_| format!("invalid release version: {value}"))
    };
    let version = Version {
        major: parse(fields.next())?,
        minor: parse(fields.next())?,
        patch: parse(fields.next())?,
        prerelease: prerelease.map(str::to_owned),
    };
    if fields.next().is_some() || prerelease.is_some_and(str::is_empty) {
        return Err(format!("invalid release version: {value}"));
    }
    Ok(version)
}

fn latest_release() -> Result<(String, Version), String> {
    let agent = crate::http::agent_for(
        LATEST_RELEASE_URL,
        crate::http::Timeouts::uniform(Duration::from_secs(15)),
    )
    .map_err(|error| format!("query latest release: {error}"))?;
    let response = agent
        .get(LATEST_RELEASE_URL)
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", concat!("uhm/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|error| format!("query latest release: {}", agent.classify_error(error)))?;
    let mut body = Vec::new();
    response
        .into_reader()
        .take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|error| format!("read latest release metadata: {error}"))?;
    if body.len() as u64 > MAX_METADATA_BYTES {
        return Err("latest release metadata exceeded 256 KiB".into());
    }
    let release: LatestRelease = serde_json::from_slice(&body)
        .map_err(|error| format!("parse latest release metadata: {error}"))?;
    let version = parse_version(&release.tag_name)?;
    Ok((release.tag_name, version))
}

fn install_dir(current_executable: &Path) -> Result<&Path, String> {
    if current_executable
        .file_name()
        .and_then(|name| name.to_str())
        != Some("uhm")
    {
        return Err(format!(
            "cannot safely replace executable {}; reinstall from https://github.com/nibzard/uhm/releases",
            current_executable.display()
        ));
    }
    current_executable
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "cannot resolve the current executable directory".into())
}

fn install(tag: &str, current_executable: &Path) -> Result<std::process::Output, String> {
    let directory = install_dir(current_executable)?;
    let mut child = Command::new("/bin/sh")
        .arg("-s")
        .env("UHM_VERSION", tag)
        .env("UHM_INSTALL_DIR", directory)
        .env_remove("UHM_TARGET")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("start embedded installer: {error}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| "open embedded installer input".to_string())?
        .write_all(INSTALLER)
        .map_err(|error| format!("write embedded installer: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("wait for embedded installer: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "installer exited with status {}{}{}",
            output.status,
            if detail.trim().is_empty() { "" } else { ": " },
            detail.trim()
        ));
    }
    Ok(output)
}

pub fn run(current: &str) -> Result<UpdateResult, String> {
    let current_version = parse_version(current)?;
    let (latest_tag, latest_version) = latest_release()?;
    let latest_display = latest_tag
        .strip_prefix('v')
        .unwrap_or(&latest_tag)
        .to_owned();
    match current_version.cmp(&latest_version) {
        Ordering::Greater => Ok(UpdateResult {
            outcome: "ahead",
            current_version: current.into(),
            latest_version: latest_display,
            installed_path: None,
            installer_stdout: Vec::new(),
            installer_stderr: Vec::new(),
        }),
        Ordering::Equal => Ok(UpdateResult {
            outcome: "current",
            current_version: current.into(),
            latest_version: latest_display,
            installed_path: None,
            installer_stdout: Vec::new(),
            installer_stderr: Vec::new(),
        }),
        Ordering::Less => {
            let executable = std::env::current_exe()
                .map_err(|error| format!("resolve current executable: {error}"))?;
            let output = install(&latest_tag, &executable)?;
            Ok(UpdateResult {
                outcome: "updated",
                current_version: current.into(),
                latest_version: latest_display,
                installed_path: Some(executable),
                installer_stdout: output.stdout,
                installer_stderr: output.stderr,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_versions_compare_without_downgrading_newer_builds() {
        assert!(parse_version("0.6.3").unwrap() > parse_version("v0.6.0").unwrap());
        assert!(parse_version("v1.0.0").unwrap() > parse_version("0.99.99").unwrap());
        assert!(parse_version("1.0.0").unwrap() > parse_version("1.0.0-rc.1").unwrap());
        assert_eq!(
            parse_version("v1.2.3+build.7").unwrap(),
            parse_version("1.2.3").unwrap()
        );
    }

    #[test]
    fn malformed_release_versions_are_rejected() {
        for value in ["", "v1", "1.2", "1.2.3.4", "one.two.three", "1.2.3-"] {
            assert!(parse_version(value).is_err(), "{value}");
        }
    }

    #[test]
    fn updater_only_replaces_a_binary_named_uhm() {
        assert_eq!(
            install_dir(Path::new("/tmp/bin/uhm")).unwrap(),
            Path::new("/tmp/bin")
        );
        assert!(install_dir(Path::new("/tmp/bin/not-uhm")).is_err());
    }
}
