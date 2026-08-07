//! Standalone release updater. This deliberately runs before configuration
//! loading: updating UHM never requires a model-provider credential.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const REPO_SLUG: &str = "nibzard/uhm";
const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/nibzard/uhm/releases/latest";
const INSTALLER: &[u8] = include_bytes!("../docs/install.sh");
const MAX_METADATA_BYTES: u64 = 256 * 1024;
/// Upper bound on the fetched checksum manifest. The real manifest is a few
/// hundred bytes; this only caps a hostile feed that never terminates.
const MAX_CHECKSUM_BYTES: u64 = 64 * 1024;
/// Upper bound on the fetched detached minisign signature (a few hundred bytes).
const MAX_SIGNATURE_BYTES: u64 = 8 * 1024;

/// Minisign (ed25519) public key — base64 of the 42-byte public-key blob — whose
/// private counterpart signs each release's `SHA256SUMS`. The updater fetches
/// `SHA256SUMS` and `SHA256SUMS.minisig`, verifies the detached signature under
/// this key, and hands the *authenticated* manifest to the installer so the
/// archive is checked against checksums an attacker cannot substitute. This
/// closes the self-attested-checksums gap: a compromised release feed or MITM
/// could otherwise ship a `SHA256SUMS` that matches a tampered binary.
///
/// While this constant equals [`RELEASE_PUBLIC_KEY_PLACEHOLDER`] the updater
/// cannot authenticate releases and falls back to the prior TLS-plus-checksum
/// path with a prominent warning; once a real key is compiled in, verification
/// is enforced fail-closed (a missing, malformed, or non-verifying signature
/// refuses the update). Generate a keypair and rotate this constant per
/// docs/reference/release-signing.md.
const RELEASE_PUBLIC_KEY: &str = RELEASE_PUBLIC_KEY_PLACEHOLDER;
const RELEASE_PUBLIC_KEY_PLACEHOLDER: &str = "REPLACE_WITH_MINISIGN_PUBLIC_KEY";

fn signature_verification_enabled() -> bool {
    RELEASE_PUBLIC_KEY != RELEASE_PUBLIC_KEY_PLACEHOLDER
}

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
                (Some(left), Some(right)) => cmp_prerelease(left, right),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Compare two semver prerelease suffixes per the spec: split on `.`, compare
/// identifier by identifier. Numeric identifiers compare numerically and rank
/// below non-numeric ones; otherwise identifiers compare by ASCII. A shorter
/// run ranks below a longer one once the shared prefix ties. A plain
/// whole-string compare gets `rc.10` < `rc.2`, which can skip a valid update.
fn cmp_prerelease(left: &str, right: &str) -> Ordering {
    let mut l = left.split('.');
    let mut r = right.split('.');
    loop {
        match (l.next(), r.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(a), Some(b)) => match cmp_prerelease_identifier(a, b) {
                Ordering::Equal => continue,
                non_equal => return non_equal,
            },
        }
    }
}

fn cmp_prerelease_identifier(a: &str, b: &str) -> Ordering {
    let a_numeric = !a.is_empty() && a.bytes().all(|byte| byte.is_ascii_digit());
    let b_numeric = !b.is_empty() && b.bytes().all(|byte| byte.is_ascii_digit());
    match (a_numeric, b_numeric) {
        // Numeric identifiers compare by integer value; comparing the
        // zero-stripped forms by length then lexicographically matches integer
        // ordering for non-negative values.
        (true, true) => {
            let na = a.trim_start_matches('0');
            let nb = b.trim_start_matches('0');
            na.len().cmp(&nb.len()).then(na.cmp(nb))
        }
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => a.cmp(b),
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

/// Fetch a single release asset (e.g. `SHA256SUMS`, `SHA256SUMS.minisig`) over
/// uhm's own rustls agent — the same trust root, proxy policy, and deadline
/// discipline that governs the version-decision fetch. The asset URL redirects
/// from github.com to the CDN; ureq follows that redirect under the same TLS
/// configuration. `limit` bounds a hostile feed.
fn fetch_release_bytes(tag: &str, asset: &str, limit: u64) -> Result<Vec<u8>, String> {
    let url = format!("https://github.com/{REPO_SLUG}/releases/download/{tag}/{asset}");
    let agent = crate::http::agent_for(
        &url,
        crate::http::Timeouts::uniform(Duration::from_secs(30)),
    )
    .map_err(|error| format!("fetch {asset}: {error}"))?;
    let response = agent
        .get(&url)
        .set("User-Agent", concat!("uhm/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|error| format!("fetch {asset}: {}", agent.classify_error(error)))?;
    let mut body = Vec::new();
    response
        .into_reader()
        .take(limit + 1)
        .read_to_end(&mut body)
        .map_err(|error| format!("read {asset}: {error}"))?;
    if body.len() as u64 > limit {
        return Err(format!("{asset} exceeded the {limit}-byte limit"));
    }
    Ok(body)
}

/// Verify a detached minisign `signature_text` over `message` under the base64
/// public key. Only modern prehashed signatures (minisign's default) are
/// accepted; legacy non-prehashed signatures are refused. Returns a
/// user-facing message on every failure path so callers can fail closed.
fn verify_minisign(
    public_key_b64: &str,
    message: &[u8],
    signature_text: &str,
) -> Result<(), String> {
    let public_key = minisign_verify::PublicKey::from_base64(public_key_b64)
        .map_err(|error| format!("compiled-in release public key is malformed: {error}"))?;
    let signature = minisign_verify::Signature::decode(signature_text)
        .map_err(|error| format!("SHA256SUMS.minisig is malformed: {error}"))?;
    public_key
        .verify(message, &signature, false)
        .map_err(|error| format!("SHA256SUMS signature verification failed: {error}"))
}

/// Stage authenticated checksum bytes in an owner-private temporary file the
/// installer reads in place of fetching its own manifest. `tempfile` creates a
/// uniquely named, mode-0600 file, so a concurrent `uhm` invocation cannot
/// collide and another local user cannot substitute the contents.
fn stage_authenticated_checksums(bytes: &[u8]) -> Result<PathBuf, String> {
    let mut file = tempfile::Builder::new()
        .prefix("uhm-checksums-")
        .rand_bytes(12)
        .tempfile_in(std::env::temp_dir())
        .map_err(|error| format!("stage authenticated checksums: {error}"))?;
    file.write_all(bytes)
        .map_err(|error| format!("stage authenticated checksums: {error}"))?;
    let (_handle, path) = file
        .keep()
        .map_err(|error| format!("stage authenticated checksums: {error}"))?;
    Ok(path)
}

/// Authenticate the release checksum manifest before the installer trusts it.
///
/// Returns `Ok(None)` when release signing is not yet configured (the public key
/// is still the placeholder) so the caller falls back to the prior
/// TLS-plus-checksum path with a warning. Returns `Ok(Some(path))` — a temporary
/// file holding the verified manifest — when the detached minisign signature
/// over `SHA256SUMS` verifies under the compiled-in key. Returns `Err` whenever
/// signing *is* configured but the signature is missing, oversized, malformed,
/// or fails verification: a configured key makes verification fail-closed, so a
/// stripped or forged signature cannot silently downgrade the install.
fn authenticate_checksums(tag: &str) -> Result<Option<PathBuf>, String> {
    if !signature_verification_enabled() {
        return Ok(None);
    }
    let sums = fetch_release_bytes(tag, "SHA256SUMS", MAX_CHECKSUM_BYTES)?;
    let signature_bytes = fetch_release_bytes(tag, "SHA256SUMS.minisig", MAX_SIGNATURE_BYTES)?;
    let signature_text = std::str::from_utf8(&signature_bytes)
        .map_err(|error| format!("SHA256SUMS.minisig is not valid UTF-8: {error}"))?;
    verify_minisign(RELEASE_PUBLIC_KEY, &sums, signature_text)?;
    stage_authenticated_checksums(&sums).map(Some)
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

fn install(
    tag: &str,
    current_executable: &Path,
    authenticated_checksums: Option<&Path>,
) -> Result<std::process::Output, String> {
    let directory = install_dir(current_executable)?;
    let mut command = Command::new("/bin/sh");
    command
        .arg("-s")
        .env("UHM_VERSION", tag)
        .env("UHM_INSTALL_DIR", directory)
        .env_remove("UHM_TARGET")
        // The installer fetches the executable with curl/wget, which otherwise
        // honour CURL_CA_BUNDLE / SSL_CERT_FILE / SSL_CERT_DIR — a broader,
        // env-spoofable trust root than uhm's own strict TLS policy (it appends
        // roots only via UHM_CA_BUNDLE and surfaces SSL_CERT_FILE errors).
        // Strip them so the most dangerous byte transfer is verified against the
        // same system roots that governed the version-decision fetch.
        .env_remove("CURL_CA_BUNDLE")
        .env_remove("SSL_CERT_FILE")
        .env_remove("SSL_CERT_DIR");
    // When the checksum manifest was authenticated against the pinned release
    // key, hand the installer the exact verified bytes so it does not fetch a
    // substitutable copy of its own; the installer still downloads the archive
    // over TLS and checks it against this manifest.
    if let Some(path) = authenticated_checksums {
        command.env("UHM_SHA256SUMS_FILE", path);
    }
    let mut child = command
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
            // Authenticate the release checksum manifest before installing.
            //   Ok(None)  signing not configured yet -> warn, fall back to the
            //              prior TLS-plus-checksum path (the installer fetches
            //              its own SHA256SUMS).
            //   Ok(Some)  manifest verified -> the installer trusts these exact
            //              bytes instead of fetching a substitutable copy.
            //   Err       signing IS configured but the signature is missing,
            //              malformed, or does not verify -> refuse the update.
            let mut installer_stderr = Vec::new();
            let authenticated = match authenticate_checksums(&latest_tag) {
                Ok(None) => {
                    installer_stderr.extend_from_slice(
                        b"uhm: release signatures are not yet configured; this update is \
                         verified by TLS and checksum only (see docs/reference/release-signing.md).\n",
                    );
                    None
                }
                Ok(Some(path)) => Some(path),
                Err(error) => return Err(error),
            };
            let result = install(&latest_tag, &executable, authenticated.as_deref());
            if let Some(path) = authenticated {
                let _ = std::fs::remove_file(path);
            }
            let output = result?;
            installer_stderr.extend_from_slice(&output.stderr);
            Ok(UpdateResult {
                outcome: "updated",
                current_version: current.into(),
                latest_version: latest_display,
                installed_path: Some(executable),
                installer_stdout: output.stdout,
                installer_stderr,
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
    fn prerelease_versions_compare_by_semver_rules() {
        use std::cmp::Ordering;
        let v = |s: &str| parse_version(s).unwrap();
        assert_eq!(v("1.0.0-rc.1").cmp(&v("1.0.0-rc.2")), Ordering::Less);
        // numeric, not lexicographic: rc.10 outranks rc.2 (a plain string
        // compare would order them backwards and skip a real update).
        assert_eq!(v("1.0.0-rc.2").cmp(&v("1.0.0-rc.10")), Ordering::Less);
        assert_eq!(v("1.0.0-alpha").cmp(&v("1.0.0-beta")), Ordering::Less);
        // numeric identifiers rank below alphabetic ones.
        assert_eq!(v("1.0.0-1").cmp(&v("1.0.0-alpha")), Ordering::Less);
        // a longer prerelease with a tied prefix outranks a shorter one.
        assert_eq!(v("1.0.0-rc.1.1").cmp(&v("1.0.0-rc.1")), Ordering::Greater);
    }

    #[test]
    fn updater_only_replaces_a_binary_named_uhm() {
        assert_eq!(
            install_dir(Path::new("/tmp/bin/uhm")).unwrap(),
            Path::new("/tmp/bin")
        );
        assert!(install_dir(Path::new("/tmp/bin/not-uhm")).is_err());
    }

    /// A real prehashed minisign test vector (public key + signature over the
    /// literal bytes `b"test"`) exercises the verify-before-install property
    /// directly: the shipped binary cannot otherwise reach this path until a key
    /// is configured, so the property would be unguarded without this test.
    /// Reusing the minisign-verify crate's published vector avoids shipping a
    /// throwaway private key.
    #[test]
    fn release_signature_verifies_the_manifest_and_rejects_tampering() {
        let public_key = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
        let signature = "untrusted comment: signature from minisign secret key
RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=
trusted comment: timestamp:1556193335\tfile:test
y/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg==";

        // The signed manifest verifies; flipping a single byte does not.
        verify_minisign(public_key, b"test", signature).unwrap();
        assert!(verify_minisign(public_key, b"tampered-manifest", signature).is_err());

        // A signature under a different key is rejected (wrong key material —
        // last base64 char flipped, still a valid 42-byte key).
        assert!(verify_minisign(
            "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO4",
            b"test",
            signature,
        )
        .is_err());

        // A malformed or missing signature cannot pass verification.
        assert!(verify_minisign(public_key, b"test", "").is_err());
        assert!(verify_minisign(public_key, b"test", "garbage not a minisig").is_err());
    }
}
