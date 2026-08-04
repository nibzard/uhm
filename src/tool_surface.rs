//! Observed capability of tools an intent names, bounded and consented per binary.
//! Help output is untrusted data and never becomes policy.

use std::path::{Path, PathBuf};

/// At most this many tools are probed for one intent. Ordered by first mention,
/// so a long request cannot crowd out the tool it actually names first.
pub const MAX_TOOLS: usize = 3;
/// Retained help output per tool, truncated at a line boundary below this.
pub const MAX_HELP_BYTES: usize = 4096;
/// Ceiling across every tool in one request.
pub const MAX_TOTAL_BYTES: usize = 8192;
/// Longest intent token that can name an executable.
const MAX_TOKEN_BYTES: usize = 32;

/// A token that could name an executable. Deliberately strict: anything outside
/// this set is discarded rather than escaped, and nothing here can be a path, a
/// redirection, a substitution, or a separator.
fn is_candidate_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= MAX_TOKEN_BYTES
        && token.starts_with(|c: char| c.is_ascii_alphanumeric())
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'))
}

/// Punctuation prose wraps names in. Only these are trimmed, so a token opening
/// with a substitution or redirection character is rejected whole rather than
/// having the character stripped off and the remainder accepted.
const TRIMMED_PROSE: &[char] = &[
    '.', ',', ';', ':', '!', '?', '"', '\'', '`', '(', ')', '[', ']', '{', '}',
];

/// Intent words that could name an executable, in order of first mention and
/// without duplicates. Surrounding prose punctuation is trimmed; a token that
/// still contains anything unexpected is dropped, not sanitized.
pub fn tokens(intent: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for word in intent.split_whitespace() {
        let trimmed = word.trim_matches(|c: char| TRIMMED_PROSE.contains(&c));
        if is_candidate_token(trimmed) && !found.iter().any(|seen| seen == trimmed) {
            found.push(trimmed.to_owned());
        }
    }
    found
}

/// Truncate help output at the last line boundary that fits, so a tool never
/// contributes a partial line.
pub fn truncate_help(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.trim_end().to_owned();
    }
    let clipped = &text[..limit];
    match clipped.rfind('\n') {
        Some(index) => clipped[..index].trim_end().to_owned(),
        None => clipped.trim_end().to_owned(),
    }
}

/// Identity of a probed binary. A tool whose bytes changed is a different tool:
/// its consent is re-asked and its retained help is stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub modified_secs: u64,
}

impl Identity {
    pub fn resolve(name: &str, path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        if !metadata.is_file() {
            return None;
        }
        let modified_secs = metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        Some(Self {
            name: name.to_owned(),
            path: path.to_path_buf(),
            size: metadata.len(),
            modified_secs,
        })
    }

    /// Stable key for the consent and observation record.
    pub fn key(&self) -> String {
        format!(
            "{}|{}|{}",
            self.path.to_string_lossy(),
            self.size,
            self.modified_secs
        )
    }
}

/// Flags tried in order. The first successful, non-empty response wins. Every
/// one is a request for self-description, never an operand.
const HELP_FLAGS: [&str; 3] = ["--help", "help", "-h"];

const STORE_VERSION: u32 = 1;
const STORE_FILE: &str = "tool-surface.json";

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Store {
    version: u32,
    /// Keyed by `Identity::key`, so changed bytes are a new entry and the old
    /// decision never carries over to a different binary.
    tools: std::collections::BTreeMap<String, Record>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Record {
    allowed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    help: Option<String>,
}

/// One tool's self-description, ready to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    pub name: String,
    pub help: String,
}

fn load(data_dir: &Path) -> Store {
    std::fs::read_to_string(data_dir.join(STORE_FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<Store>(&text).ok())
        .filter(|store| store.version == STORE_VERSION)
        .unwrap_or_else(|| Store {
            version: STORE_VERSION,
            tools: Default::default(),
        })
}

fn save(data_dir: &Path, store: &Store) -> Result<(), String> {
    crate::dirs::ensure_private_dir(data_dir)?;
    let bytes =
        serde_json::to_vec(store).map_err(|e| format!("serialize tool surface store: {e}"))?;
    let path = data_dir.join(STORE_FILE);
    let temporary = data_dir.join(format!("{STORE_FILE}.tmp"));
    write_private(&temporary, &bytes)?;
    std::fs::rename(&temporary, &path).map_err(|e| format!("publish tool surface store: {e}"))
}

fn write_private(path: &Path, contents: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    file.write_all(contents)
        .map_err(|e| format!("write {}: {e}", path.display()))
}

fn probe(identity: &Identity, deadline: std::time::Instant) -> Option<String> {
    let path = identity.path.to_string_lossy().into_owned();
    HELP_FLAGS.iter().find_map(|flag| {
        crate::context::run(&[path.as_str(), flag], deadline)
            .map(|text| truncate_help(&text, MAX_HELP_BYTES))
            .filter(|text| !text.is_empty())
    })
}

/// Observed capability for the tools this intent names.
///
/// `ask` is consulted at most once per distinct binary and its answer is
/// persisted, so an allowed tool is probed silently afterwards and a declined
/// one is never asked about again until its bytes change. Probing runs local
/// programs, so a caller without a terminal must pass an `ask` that declines;
/// already-allowed tools still resolve from the record.
///
/// `search` is the directory list to resolve names against, passed in rather
/// than read from the environment so behavior is explicit and testable.
pub fn surface(
    intent: &str,
    data_dir: &Path,
    search: &[PathBuf],
    deadline: std::time::Instant,
    ask: &mut dyn FnMut(&Identity) -> bool,
) -> Vec<Observed> {
    let identities: Vec<Identity> = tokens(intent)
        .into_iter()
        .filter_map(|name| {
            crate::context::resolve_in(search, &name)
                .and_then(|path| Identity::resolve(&name, &path))
        })
        .take(MAX_TOOLS)
        .collect();
    if identities.is_empty() {
        return Vec::new();
    }
    let mut store = load(data_dir);
    let mut dirty = false;
    let mut observed = Vec::new();
    let mut total = 0usize;
    for identity in identities {
        let key = identity.key();
        let known = store.tools.get(&key);
        let allowed = match known {
            Some(record) => record.allowed,
            None => {
                let answer = ask(&identity);
                store.tools.insert(
                    key.clone(),
                    Record {
                        allowed: answer,
                        help: None,
                    },
                );
                dirty = true;
                answer
            }
        };
        if !allowed {
            continue;
        }
        let help = match store.tools.get(&key).and_then(|record| record.help.clone()) {
            Some(retained) => retained,
            None => {
                let Some(fresh) = probe(&identity, deadline) else {
                    continue;
                };
                store.tools.insert(
                    key.clone(),
                    Record {
                        allowed: true,
                        help: Some(fresh.clone()),
                    },
                );
                dirty = true;
                fresh
            }
        };
        if total.saturating_add(help.len()) > MAX_TOTAL_BYTES {
            continue;
        }
        total += help.len();
        observed.push(Observed {
            name: identity.name.clone(),
            help,
        });
    }
    if dirty {
        if let Err(error) = save(data_dir, &store) {
            crate::history::warn(&error);
        }
    }
    observed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_keep_plain_command_names_in_order_without_duplicates() {
        assert_eq!(
            tokens("start steel session and open hacker news"),
            vec!["start", "steel", "session", "and", "open", "hacker", "news"]
        );
        assert_eq!(
            tokens("steel then steel again"),
            vec!["steel", "then", "again"]
        );
        assert_eq!(
            tokens("run python3 and g++ and clang-format"),
            vec!["run", "python3", "and", "g++", "clang-format"]
        );
    }

    #[test]
    fn tokens_trim_surrounding_prose_punctuation() {
        assert_eq!(
            tokens("use steel, then jq."),
            vec!["use", "steel", "then", "jq"]
        );
        assert_eq!(tokens("(steel) [jq]"), vec!["steel", "jq"]);
    }

    #[test]
    fn tokens_reject_anything_that_is_not_a_bare_name() {
        // Paths, separators, redirections, substitutions, globs, and quoting are
        // discarded rather than escaped. Nothing here may survive as a token.
        for hostile in [
            "/usr/bin/steel",
            "./deploy",
            "../deploy",
            "a/b",
            "foo;bar",
            "foo|bar",
            "foo&bar",
            "foo>out",
            "foo<in",
            "$(steel",
            "${steel",
            "steel*",
            "steel?x",
            "a=b",
            "--flag",
            "-flag",
        ] {
            let found = tokens(hostile);
            assert!(
                !found.iter().any(|t| t.contains('/')
                    || t.contains(';')
                    || t.contains('|')
                    || t.contains('&')
                    || t.contains('>')
                    || t.contains('<')
                    || t.contains('$')
                    || t.contains('*')
                    || t.contains('?')
                    || t.contains('=')),
                "{hostile} produced {found:?}"
            );
        }
    }

    #[test]
    fn tokens_drop_oversized_words() {
        let long = "a".repeat(MAX_TOKEN_BYTES + 1);
        assert!(tokens(&long).is_empty());
        let at_limit = "b".repeat(MAX_TOKEN_BYTES);
        assert_eq!(tokens(&at_limit), vec![at_limit]);
    }

    #[test]
    fn help_truncation_never_emits_a_partial_line() {
        let text =
            "usage: steel\n  browser  Browser session management\n  sessions  Cloud sessions\n";
        assert_eq!(truncate_help(text, 4096), text.trim_end());
        let clipped = truncate_help(text, 30);
        assert!(!clipped.is_empty());
        assert!(clipped.len() <= 30);
        assert!(
            text.starts_with(&clipped),
            "truncation must be a prefix: {clipped:?}"
        );
        assert!(!clipped.ends_with('\n'));
        // Every retained line is a whole line from the source.
        for line in clipped.lines() {
            assert!(text.lines().any(|original| original == line), "{line:?}");
        }
    }

    #[test]
    fn help_truncation_handles_a_single_oversized_line() {
        let text = "x".repeat(100);
        let clipped = truncate_help(&text, 10);
        assert_eq!(clipped.len(), 10);
    }

    #[test]
    fn identity_changes_when_the_binary_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool");
        std::fs::write(&path, "one").unwrap();
        let first = Identity::resolve("tool", &path).unwrap();
        std::fs::write(&path, "a longer body").unwrap();
        let second = Identity::resolve("tool", &path).unwrap();
        assert_ne!(first.key(), second.key());
        assert_eq!(first.name, "tool");
    }

    #[test]
    fn identity_rejects_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Identity::resolve("tool", dir.path()).is_none());
    }

    /// Install a probeable fake tool and return its directory plus a search
    /// list, so resolution never depends on the process environment.
    #[cfg(unix)]
    fn fake_tool(name: &str, body: &str) -> (tempfile::TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, path)
    }

    fn deadline() -> std::time::Instant {
        std::time::Instant::now() + std::time::Duration::from_secs(10)
    }

    #[cfg(unix)]
    #[test]
    fn consent_is_asked_once_per_binary_and_then_remembered() {
        let (dir, _) = fake_tool(
            "probeme",
            "#!/bin/sh\ntest \"$1\" = --help && echo 'usage: probeme <command>' || exit 2\n",
        );
        let search = vec![dir.path().to_path_buf()];
        let data = tempfile::tempdir().unwrap();
        let mut asked = 0;
        let first = surface(
            "run probeme now",
            data.path(),
            &search,
            deadline(),
            &mut |_| {
                asked += 1;
                true
            },
        );
        assert_eq!(asked, 1);
        assert_eq!(first.len(), 1, "{first:?}");
        assert_eq!(first[0].name, "probeme");
        assert!(first[0].help.contains("usage: probeme"));

        // A later job asks nothing and reuses the retained observation.
        let second = surface(
            "run probeme now",
            data.path(),
            &search,
            deadline(),
            &mut |_| {
                asked += 1;
                true
            },
        );
        assert_eq!(asked, 1, "an allowed tool must not be asked about again");
        assert_eq!(second, first);
    }

    #[cfg(unix)]
    #[test]
    fn a_declined_tool_is_never_probed_or_asked_about_again() {
        let (dir, _) = fake_tool("nope", "#!/bin/sh\necho 'should never be sent'\n");
        let search = vec![dir.path().to_path_buf()];
        let data = tempfile::tempdir().unwrap();
        let mut asked = 0;
        for _ in 0..2 {
            let observed = surface(
                "use nope please",
                data.path(),
                &search,
                deadline(),
                &mut |_| {
                    asked += 1;
                    false
                },
            );
            assert!(observed.is_empty(), "{observed:?}");
        }
        assert_eq!(asked, 1, "a declined tool must not be re-asked");
        let stored = std::fs::read_to_string(data.path().join(STORE_FILE)).unwrap();
        assert!(
            !stored.contains("should never be sent"),
            "a declined tool's output must never be retained"
        );
    }

    #[cfg(unix)]
    #[test]
    fn changed_bytes_require_consent_again() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, path) = fake_tool("drifty", "#!/bin/sh\necho 'usage: drifty one'\n");
        let search = vec![dir.path().to_path_buf()];
        let data = tempfile::tempdir().unwrap();
        let mut asked = 0;
        let before = surface("call drifty", data.path(), &search, deadline(), &mut |_| {
            asked += 1;
            true
        });
        assert!(before[0].help.contains("drifty one"));
        std::fs::write(&path, "#!/bin/sh\necho 'usage: drifty two, rewritten'\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let after = surface("call drifty", data.path(), &search, deadline(), &mut |_| {
            asked += 1;
            true
        });
        assert_eq!(asked, 2, "a rewritten binary is a different tool");
        assert!(after[0].help.contains("rewritten"), "{after:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_tool_with_no_usable_help_contributes_nothing() {
        let (dir, _) = fake_tool("silent", "#!/bin/sh\nexit 3\n");
        let search = vec![dir.path().to_path_buf()];
        let data = tempfile::tempdir().unwrap();
        let observed = surface("run silent", data.path(), &search, deadline(), &mut |_| {
            true
        });
        assert!(observed.is_empty(), "{observed:?}");
    }

    #[test]
    fn an_intent_naming_nothing_installed_asks_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let mut asked = 0;
        let observed = surface(
            "summarize the quarterly report",
            data.path(),
            &[dir.path().to_path_buf()],
            deadline(),
            &mut |_| {
                asked += 1;
                true
            },
        );
        assert!(observed.is_empty());
        assert_eq!(asked, 0);
        assert!(!data.path().join(STORE_FILE).exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_total_ceiling_bounds_what_leaves_the_device() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        for name in ["bulkone", "bulktwo", "bulkthree"] {
            let path = dir.path().join(name);
            std::fs::write(
                &path,
                "#!/bin/sh\nawk 'BEGIN{for(i=0;i<400;i++) print \"padding line for bulk output\"}'\n",
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let observed = surface(
            "bulkone bulktwo bulkthree",
            data.path(),
            &[dir.path().to_path_buf()],
            deadline(),
            &mut |_| true,
        );
        let total: usize = observed.iter().map(|item| item.help.len()).sum();
        assert!(total <= MAX_TOTAL_BYTES, "{total} exceeded the ceiling");
        for item in &observed {
            assert!(item.help.len() <= MAX_HELP_BYTES);
        }
    }

    #[cfg(unix)]
    #[test]
    fn at_most_the_probe_ceiling_of_tools_is_consulted() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let names = ["alfa", "bravo", "charlie", "delta", "echo9"];
        for name in names {
            let path = dir.path().join(name);
            std::fs::write(&path, format!("#!/bin/sh\necho 'usage: {name}'\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut asked = 0;
        let observed = surface(
            &names.join(" "),
            data.path(),
            &[dir.path().to_path_buf()],
            deadline(),
            &mut |_| {
                asked += 1;
                true
            },
        );
        assert_eq!(observed.len(), MAX_TOOLS);
        assert_eq!(asked, MAX_TOOLS, "no tool beyond the ceiling is even asked");
        assert_eq!(observed[0].name, "alfa", "first mention wins");
    }
}
