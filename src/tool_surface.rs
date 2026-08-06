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
    // The caller's limit is a byte budget, not a char boundary, and help output
    // routinely contains multi-byte characters.
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let clipped = &text[..end];
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

/// Standard utilities whose interfaces the model already knows, beyond the
/// common tools in `context::TOOL_CATALOG`. Naming one never triggers a help
/// probe, so there is no consent prompt, no local execution, and no help bytes
/// leave the device for it.
const UBIQUITOUS_TOOLS: &[&str] = &[
    "mv", "cp", "rm", "ls", "ln", "cat", "mkdir", "rmdir", "touch", "chmod", "chown", "pwd",
    "echo", "printf", "head", "tail", "wc", "sort", "uniq", "cut", "tr", "grep", "sed", "awk",
    "find", "xargs", "which", "env", "date", "diff", "cmp", "tee", "du", "df", "ps", "kill",
    "sleep", "dirname", "basename", "stat", "file", "less", "more",
];

/// A tool the model needs no self-description for: probing it would add
/// nothing, so its name is skipped before resolution or consent.
fn is_ubiquitous(name: &str) -> bool {
    UBIQUITOUS_TOOLS.contains(&name) || crate::context::TOOL_CATALOG.contains(&name)
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
    /// Plan 18: help observed for named subcommands of this tool, in the order
    /// first probed (recency). Additive and serde-defaulted, so a version-1
    /// store without it still parses and keeps its consent answers. A changed
    /// binary is a different identity key, so these never carry across versions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    subcommands: Vec<SubcommandHelp>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct SubcommandHelp {
    subcommand: String,
    help: String,
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
/// Ubiquitous standard tools are skipped before resolution: they are never
/// probed, never prompted about, and never occupy one of the `MAX_TOOLS` slots.
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
        .filter(|name| !is_ubiquitous(name))
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
                        subcommands: Vec::new(),
                    },
                );
                dirty = true;
                answer
            }
        };
        if !allowed {
            continue;
        }
        // Ensure the top-level help is retained, probing once per identity.
        // Mutate in place so any retained subcommands survive a fresh top-level
        // probe instead of being replaced by a bare Record.
        if store
            .tools
            .get(&key)
            .and_then(|record| record.help.clone())
            .is_none()
        {
            match probe(&identity, deadline) {
                Some(fresh) => {
                    if let Some(record) = store.tools.get_mut(&key) {
                        record.help = Some(fresh);
                    }
                    dirty = true;
                }
                None => continue,
            }
        }
        let Some(record) = store.tools.get(&key) else {
            continue;
        };
        // Plan 18: a warm store carries retained subcommand help into the first
        // call, so a tool whose depth was needed once is a one-call job again.
        let help = assemble_help(
            record.help.as_deref().unwrap_or_default(),
            &record.subcommands,
            intent,
        );
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

/// Coarse outcome of one subcommand probe. Recorded on the receipt as an enum
/// only; the tool name, subcommand, and help bytes never leave the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeResult {
    /// Validation failed (token, unadvertised subcommand, unknown tool, or a
    /// binary whose identity no longer matches). Nothing was executed.
    Invalid,
    /// The probe ran but produced no usable help; the surface is unchanged.
    Empty,
    /// The probe succeeded and the observation was persisted for reuse.
    Probed,
}

/// Build one tool's sent surface: top-level help first, then retained
/// subcommand entries whose token the intent names, then the rest by recency,
/// all within the existing ceilings. Called for both the first call (warm store
/// deepens it for free) and the rebuilt surface after a probe.
fn assemble_help(top: &str, subs: &[SubcommandHelp], intent: &str) -> String {
    let intent_tokens = tokens(intent);
    let matched: Vec<&SubcommandHelp> = subs
        .iter()
        .filter(|s| intent_tokens.iter().any(|t| t == &s.subcommand))
        .collect();
    // Remaining entries by recency: the most recently probed first. New entries
    // are appended, so the most recent is last; unmatched entries iterate in
    // reverse so the freshest surfaces when the ceiling forces a choice.
    let remaining: Vec<&SubcommandHelp> = subs
        .iter()
        .filter(|s| !intent_tokens.iter().any(|t| t == &s.subcommand))
        .rev()
        .collect();
    let mut out = String::new();
    let mut budget = MAX_TOTAL_BYTES;
    if !top.is_empty() {
        out.push_str(top);
        budget = budget.saturating_sub(top.len());
    }
    for piece in matched.iter().chain(remaining.iter()) {
        if budget == 0 {
            break;
        }
        let limited = truncate_help(&piece.help, budget.min(MAX_HELP_BYTES));
        if limited.is_empty() {
            continue;
        }
        out.push('\n');
        out.push_str(&limited);
        budget = budget.saturating_sub(limited.len() + 1);
    }
    out
}

/// Read one subcommand's help under the same inert argv as the top-level probe:
/// `[path, subcommand, flag]`, no shell, stdin closed, stdout capped. Hostile
/// help content cannot alter the argv, which is built from constants.
fn probe_subcommand_help(
    identity: &Identity,
    subcommand: &str,
    deadline: std::time::Instant,
) -> Option<String> {
    let path = identity.path.to_string_lossy().into_owned();
    HELP_FLAGS.iter().find_map(|flag| {
        crate::context::run(&[path.as_str(), subcommand, flag], deadline)
            .map(|text| truncate_help(&text, MAX_HELP_BYTES))
            .filter(|text| !text.is_empty())
    })
}

/// Validate and run one host-answered subcommand probe (Plan 18).
///
/// `tool` must name a binary already allowed in the store for its current
/// identity, and `subcommand` must be one bare token that appears verbatim as a
/// whitespace-delimited word in that tool's retained top-level help. The model
/// can only deepen along a path the tool itself advertised. `narrate` is called
/// once, with a single stderr line, immediately before the probe runs — and only
/// after validation has passed, so an invalid probe narrates nothing.
pub fn probe_subcommand(
    data_dir: &Path,
    search: &[PathBuf],
    tool: &str,
    subcommand: &str,
    deadline: std::time::Instant,
    narrate: &mut dyn FnMut(&str),
) -> ProbeResult {
    // Rule: the subcommand is one bare token (same rules as an executable name).
    if !is_candidate_token(subcommand) {
        return ProbeResult::Invalid;
    }
    // Resolve the named tool to its current identity on disk.
    let Some(path) = crate::context::resolve_in(search, tool) else {
        return ProbeResult::Invalid;
    };
    let Some(identity) = Identity::resolve(tool, &path) else {
        return ProbeResult::Invalid;
    };
    let mut store = load(data_dir);
    // Rule: the binary's identity must still match a retained record — a tool
    // that changed between calls is re-consented, never silently probed.
    let Some(record) = store.tools.get(&identity.key()) else {
        return ProbeResult::Invalid;
    };
    if !record.allowed {
        return ProbeResult::Invalid;
    }
    // Rule: the subcommand must appear verbatim as a word in the retained
    // top-level help. A hostile tool controls its own help text and therefore
    // which tokens are probeable, but it is probing itself, with no authority
    // the first consented probe lacked.
    let Some(top_help) = record.help.as_deref() else {
        return ProbeResult::Invalid;
    };
    if !top_help.split_whitespace().any(|word| word == subcommand) {
        return ProbeResult::Invalid;
    }
    narrate(&format!("uhm: reading {tool} {subcommand} --help"));
    let Some(fresh) = probe_subcommand_help(&identity, subcommand, deadline) else {
        return ProbeResult::Empty;
    };
    if let Some(record) = store.tools.get_mut(&identity.key()) {
        if let Some(existing) = record
            .subcommands
            .iter_mut()
            .find(|entry| entry.subcommand == subcommand)
        {
            existing.help = fresh;
        } else {
            record.subcommands.push(SubcommandHelp {
                subcommand: subcommand.to_owned(),
                help: fresh,
            });
        }
    }
    if let Err(error) = save(data_dir, &store) {
        crate::history::warn(&error);
    }
    ProbeResult::Probed
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
    fn help_truncation_never_splits_a_multi_byte_character() {
        // Modern help output routinely contains arrows and box drawing, and
        // assemble_help's remaining budget is arithmetic, not a char boundary.
        let text = format!(
            "{}\u{2192} run the thing\n{}",
            "a".repeat(10),
            "b".repeat(200)
        );
        for limit in 1..40 {
            let clipped = truncate_help(&text, limit);
            assert!(clipped.len() <= limit, "limit {limit}");
        }
    }

    #[test]
    fn assembled_help_survives_a_multi_byte_budget_boundary() {
        let subs = vec![
            SubcommandHelp {
                subcommand: "alfa".into(),
                help: format!("usage: tool alfa\n{}", "a".repeat(MAX_HELP_BYTES)),
            },
            SubcommandHelp {
                subcommand: "bravo".into(),
                help: format!("{}\u{2192}\u{2192}\u{2192} bravo", "b".repeat(64)),
            },
        ];
        let out = assemble_help("usage: tool\n", &subs, "run alfa then bravo");
        assert!(!out.is_empty());
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

    #[cfg(unix)]
    #[test]
    fn a_ubiquitous_tool_is_never_asked_about_or_probed() {
        // `mv` here is a fake that would leak if probed; the point is that a
        // ubiquitous name is skipped before resolution, so nothing runs, nothing
        // is asked, and nothing is persisted.
        let (dir, _) = fake_tool("mv", "#!/bin/sh\necho 'should never be probed'\n");
        let search = vec![dir.path().to_path_buf()];
        let data = tempfile::tempdir().unwrap();
        let observed = surface(
            "mv drive to blaxel-drive",
            data.path(),
            &search,
            deadline(),
            &mut |identity| panic!("a ubiquitous tool must not prompt: {}", identity.name),
        );
        assert!(observed.is_empty(), "{observed:?}");
        assert!(
            !data.path().join(STORE_FILE).exists(),
            "a skipped tool must leave no record"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_catalog_tool_is_never_asked_about_or_probed() {
        let (dir, _) = fake_tool("git", "#!/bin/sh\necho 'should never be probed'\n");
        let search = vec![dir.path().to_path_buf()];
        let data = tempfile::tempdir().unwrap();
        let observed = surface(
            "git status please",
            data.path(),
            &search,
            deadline(),
            &mut |identity| panic!("a catalog tool must not prompt: {}", identity.name),
        );
        assert!(observed.is_empty(), "{observed:?}");
    }

    #[cfg(unix)]
    #[test]
    fn ubiquitous_tools_do_not_consume_probe_slots() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        for name in ["mv", "grep", "sed", "alfa", "bravo", "charlie"] {
            let path = dir.path().join(name);
            std::fs::write(&path, format!("#!/bin/sh\necho 'usage: {name}'\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let observed = surface(
            "mv grep sed alfa bravo charlie",
            data.path(),
            &[dir.path().to_path_buf()],
            deadline(),
            &mut |_| true,
        );
        let names: Vec<&str> = observed.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["alfa", "bravo", "charlie"],
            "skipped names must not crowd out probeable tools"
        );
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

    #[test]
    fn assemble_help_leads_with_top_then_intent_matched_subcommands() {
        let top = "usage: tool\n  sessions  Cloud sessions\n  config    Settings\n";
        let subs = vec![
            SubcommandHelp {
                subcommand: "config".into(),
                help: "usage: tool config <key>".into(),
            },
            SubcommandHelp {
                subcommand: "sessions".into(),
                help: "usage: tool sessions <verb>".into(),
            },
        ];
        // Top-level help always leads; an intent-named subcommand precedes the
        // rest so the freshest relevant depth surfaces within the ceiling.
        let out = assemble_help(top, &subs, "manage tool sessions please");
        assert!(
            out.starts_with("usage: tool"),
            "top-level help leads: {out:?}"
        );
        let sessions = out.find("sessions <verb>").unwrap();
        let config = out.find("config <key>").unwrap();
        assert!(
            sessions < config,
            "intent-named subcommand precedes others: {out:?}"
        );
        // When the intent names neither subcommand, both still appear after top.
        let neutral = assemble_help(top, &subs, "do something unrelated here");
        assert!(neutral.contains("sessions <verb>"));
        assert!(neutral.contains("config <key>"));
        // A single oversized subcommand entry is bounded by the total ceiling.
        let big = SubcommandHelp {
            subcommand: "big".into(),
            help: "x".repeat(MAX_HELP_BYTES * 4),
        };
        let oversized = assemble_help("", &[big], "big");
        assert!(oversized.len() <= MAX_TOTAL_BYTES, "{}", oversized.len());
    }

    /// Consent one tool (answer yes) and retain its top-level help, returning the
    /// dir that owns the fake binary so the caller keeps it alive on disk.
    #[cfg(unix)]
    fn consented_tool(name: &str, body: &str) -> (tempfile::TempDir, tempfile::TempDir) {
        let (dir, _) = fake_tool(name, body);
        let data = tempfile::tempdir().unwrap();
        surface(
            &format!("run {name} now"),
            data.path(),
            &[dir.path().to_path_buf()],
            deadline(),
            &mut |_| true,
        );
        (dir, data)
    }

    #[cfg(unix)]
    #[test]
    fn probe_rejects_a_non_token_subcommand_without_narrating() {
        let (dir, data) = consented_tool("probeme", "#!/bin/sh\necho 'usage: probeme sessions'\n");
        let mut narrated = 0;
        let outcome = probe_subcommand(
            data.path(),
            &[dir.path().to_path_buf()],
            "probeme",
            "--help",
            deadline(),
            &mut |_| narrated += 1,
        );
        assert_eq!(outcome, ProbeResult::Invalid);
        assert_eq!(narrated, 0, "an invalid probe must not narrate");
    }

    #[cfg(unix)]
    #[test]
    fn probe_rejects_a_tool_that_does_not_resolve() {
        let (_dir, data) = consented_tool("probeme", "#!/bin/sh\necho 'usage: probeme sessions'\n");
        let outcome = probe_subcommand(
            data.path(),
            &[],
            "ghost-not-installed",
            "sessions",
            deadline(),
            &mut |_| panic!("unknown tool must not narrate"),
        );
        assert_eq!(outcome, ProbeResult::Invalid);
    }

    #[cfg(unix)]
    #[test]
    fn probe_rejects_a_resolvable_but_unrecorded_tool() {
        // The binary resolves but was never consented, so the store has no record
        // for its identity: probing would deepen a tool the user never allowed.
        let (dir, _data) = fake_tool("stranger", "#!/bin/sh\necho 'usage: stranger sessions'\n");
        let data = tempfile::tempdir().unwrap();
        let outcome = probe_subcommand(
            data.path(),
            &[dir.path().to_path_buf()],
            "stranger",
            "sessions",
            deadline(),
            &mut |_| panic!("unrecorded tool must not narrate"),
        );
        assert_eq!(outcome, ProbeResult::Invalid);
    }

    #[cfg(unix)]
    #[test]
    fn probe_rejects_a_tool_the_user_declined() {
        let (dir, _) = fake_tool("nope", "#!/bin/sh\necho 'usage: nope sessions'\n");
        let data = tempfile::tempdir().unwrap();
        // Consent is asked and refused; a declined record is retained but unused.
        surface(
            "run nope now",
            data.path(),
            &[dir.path().to_path_buf()],
            deadline(),
            &mut |_| false,
        );
        let outcome = probe_subcommand(
            data.path(),
            &[dir.path().to_path_buf()],
            "nope",
            "sessions",
            deadline(),
            &mut |_| panic!("a declined tool must not narrate"),
        );
        assert_eq!(outcome, ProbeResult::Invalid);
    }

    #[cfg(unix)]
    #[test]
    fn probe_rejects_a_subcommand_the_top_help_did_not_advertise() {
        let (dir, data) = consented_tool(
            "probeme",
            "#!/bin/sh\necho 'usage: probeme sessions status'\n",
        );
        let outcome = probe_subcommand(
            data.path(),
            &[dir.path().to_path_buf()],
            "probeme",
            "network",
            deadline(),
            &mut |_| panic!("an unadvertised subcommand must not narrate"),
        );
        assert_eq!(outcome, ProbeResult::Invalid);
    }

    #[cfg(unix)]
    #[test]
    fn probe_rejects_a_consented_tool_with_no_retained_help() {
        // Consent succeeds but the top-level probe produced nothing usable, so the
        // record has no help to validate the subcommand token against.
        let (dir, data) = consented_tool("silent", "#!/bin/sh\nexit 3\n");
        // Ensure consent landed even though help is absent.
        assert!(load(data.path())
            .tools
            .values()
            .any(|record| record.allowed && record.help.is_none()));
        let outcome = probe_subcommand(
            data.path(),
            &[dir.path().to_path_buf()],
            "silent",
            "anything",
            deadline(),
            &mut |_| panic!("a tool with no help must not narrate"),
        );
        assert_eq!(outcome, ProbeResult::Invalid);
    }

    #[cfg(unix)]
    #[test]
    fn probe_rejects_a_tool_whose_bytes_changed_since_consent() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, path) = fake_tool("drifty", "#!/bin/sh\necho 'usage: drifty sessions'\n");
        let data = tempfile::tempdir().unwrap();
        surface(
            "run drifty now",
            data.path(),
            &[dir.path().to_path_buf()],
            deadline(),
            &mut |_| true,
        );
        // Rewrite the binary: a different identity key, so the old consent no
        // longer matches and the probe is refused rather than run silently.
        std::fs::write(
            &path,
            "#!/bin/sh\necho 'usage: drifty rewritten sessions'\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let outcome = probe_subcommand(
            data.path(),
            &[dir.path().to_path_buf()],
            "drifty",
            "sessions",
            deadline(),
            &mut |_| panic!("a changed binary must not narrate"),
        );
        assert_eq!(outcome, ProbeResult::Invalid);
    }

    #[cfg(unix)]
    #[test]
    fn probe_persists_depth_and_then_amortizes_for_free() {
        let (dir, data) = consented_tool(
            "probeme",
            "#!/bin/sh\n\
             if [ \"$1\" = sessions ]; then\n\
             \tprintf 'usage: probeme sessions <verb>\\n  list  List sessions\\n  new   Create a session\\n'\n\
             \texit 0\n\
             fi\n\
             printf 'usage: probeme <command>\\n  sessions  Cloud sessions\\n  status    Show status\\n'",
        );
        let mut narrated = Vec::new();
        let outcome = probe_subcommand(
            data.path(),
            &[dir.path().to_path_buf()],
            "probeme",
            "sessions",
            deadline(),
            &mut |line| narrated.push(line.to_owned()),
        );
        assert_eq!(outcome, ProbeResult::Probed);
        assert_eq!(
            narrated,
            vec!["uhm: reading probeme sessions --help"],
            "narration fires once, only after validation"
        );
        // The deeper help is retained for the subcommand exactly once.
        let store = load(data.path());
        let record = store.tools.values().find(|record| record.allowed).unwrap();
        assert_eq!(record.subcommands.len(), 1);
        assert_eq!(record.subcommands[0].subcommand, "sessions");
        assert!(record.subcommands[0].help.contains("list"));

        // A later job warms from the store: the surfaced help now carries the
        // deeper verb set, so the same intent is a one-call job again.
        let observed = surface(
            "run probeme sessions",
            data.path(),
            &[dir.path().to_path_buf()],
            deadline(),
            &mut |_| panic!("a warm store must not ask for consent again"),
        );
        assert_eq!(observed.len(), 1);
        assert!(observed[0].help.contains("list"));
        assert!(observed[0].help.contains("usage: probeme"));

        // Re-probing the same subcommand upserts in place instead of duplicating.
        let again = probe_subcommand(
            data.path(),
            &[dir.path().to_path_buf()],
            "probeme",
            "sessions",
            deadline(),
            &mut |_| {},
        );
        assert_eq!(again, ProbeResult::Probed);
        let store = load(data.path());
        let record = store.tools.values().find(|record| record.allowed).unwrap();
        assert_eq!(record.subcommands.len(), 1, "upsert must not duplicate");
    }

    #[cfg(unix)]
    #[test]
    fn probe_reports_empty_when_the_subcommand_has_no_help() {
        let (dir, data) = consented_tool(
            "probeme",
            "#!/bin/sh\nif [ \"$1\" = sessions ]; then exit 0; fi\necho 'usage: probeme sessions'\n",
        );
        let mut narrated = 0;
        let outcome = probe_subcommand(
            data.path(),
            &[dir.path().to_path_buf()],
            "probeme",
            "sessions",
            deadline(),
            &mut |_| narrated += 1,
        );
        assert_eq!(outcome, ProbeResult::Empty);
        assert_eq!(narrated, 1, "an empty probe still narrated before running");
        // Nothing was retained: a subsequent surface carries top-level help only.
        let observed = surface(
            "run probeme sessions",
            data.path(),
            &[dir.path().to_path_buf()],
            deadline(),
            &mut |_| true,
        );
        assert_eq!(observed.len(), 1);
        let store = load(data.path());
        let record = store.tools.values().find(|record| record.allowed).unwrap();
        assert!(
            record.subcommands.is_empty(),
            "an empty probe must not persist"
        );
    }
}
