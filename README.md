# uhm

Say what you need. Get the result.

`uhm` is a fast natural-language layer over terminal tools. It turns one intent into one typed proposal, runs ordinary work, and returns the command's actual output. It is not a coding agent, a chat session, or a background worker.

## Quick start

You need Rust 1.82 or newer and an OpenAI API key:

```sh
cargo install --path .
export OPENAI_API_KEY=sk-...
uhm -- find the ten biggest files in this directory
```

Ordinary actions run immediately. When uhm detects deletion, broad writes, elevated privileges, remote mutation, process control, or another hard-to-classify effect, it shows the exact command and asks first. Detection is a convenience warning, not a safety guarantee. `--force` remains the user's override.

## Examples

```sh
# Require an executable action
uhm run -- count the paragraphs in README.md

# Ask without executing
uhm ask -- what does git log -p mean

# Explain command text without executing it
uhm explain -- git log -p

# Use piped input as request data
git diff | uhm ask -- write a concise summary

# Review every proposal, or print exact command bytes
uhm run --review -- remove build artifacts
uhm run --dry-run -- concatenate the markdown files
```

After the first intent word, every argument is opaque user text. A dictated prompt containing `-y`, `--help`, or `--system` cannot change uhm's authority. Use the explicit `--` boundary when intent starts with a hyphen.

## Commands

| Command | Behavior |
|---|---|
| `uhm -- <intent>` | Answer, clarify, or perform one local action |
| `uhm run -- <intent>` | Require an executable local action |
| `uhm ask -- <question>` | Request a prose-valued terminal/CLI result |
| `uhm explain -- <command>` | Request an explanation through the same typed action contract |
| `uhm history status|clear` | Inspect or clear bounded metadata receipts |
| `uhm config show` | Show resolved values and sources |
| `uhm config check` | Strictly parse and validate configuration |
| `uhm context show [mode]` | Show the exact structured context shape used for proposals |
| `uhm doctor` | Check paths, configuration, and credentials |

Execution options:

```text
--review    always show the exact proposal and ask before execution
--dry-run   emit exact command bytes and execute nothing
--force     skip advisory confirmation after showing warnings
--plain     disable styling, animation, and terminal controls
--json      emit namespaced machine-readable outcomes
```

`--review`, `--dry-run`, and `--force` are mutually exclusive. See the full [behavior and exit-status contract](docs/behavior-contract.md).

## Output contract

The requested result stays on stdout and composes with pipes. Progress, review UI, warnings, and executed-child JSON receipts go to stderr. If a child executes, uhm returns its exit status unchanged. A proposal that was not executed returns a distinct nonzero application status.

Styling never reconstructs a command. The bytes shown in review and supplied to the child shell are the model's validated command bytes. Redirected output, `--plain`, `NO_COLOR`, and `TERM=dumb` do not emit terminal control sequences.

## Configuration

Copy [config.example.yaml](config.example.yaml) to the platform path shown by `uhm config show`. Every key is optional, but unknown keys, bad types, invalid values, unreadable files, and unsafe relative XDG roots are errors.

```yaml
model: gpt-5.6-terra
shell: auto
stream: true
context_mode: standard

history:
  enabled: true

aliases:
  gst: git status -sb
```

Environment overrides:

| Variable | Purpose |
|---|---|
| `OPENAI_API_KEY` | API credential |
| `OPENAI_MODEL` | Model ID |
| `UHM_PLAIN` | Disable terminal presentation features |
| `NO_COLOR` | Disable color |

The optional private secrets file is `<data-dir>/uhm/secrets` and contains `OPENAI_API_KEY=...`. On Unix, uhm requires private file permissions. Runtime directories and files are created as `0700` and `0600` respectively.

## Data and trust

All model calls go to OpenAI's official `POST /v1/responses` endpoint with provider storage disabled. `standard` context is sent by default: bounded OS/architecture, target shell, common-tool presence booleans, a normalized working directory, bounded Git state, and at most 40 entry names. `minimal` sends only the intent and explicitly piped stdin; `full` adds identifying machine fields and bounded versions. Inspect the exact shape with `uhm context show`, or select a mode with `--context minimal` / `context_mode: minimal`. Environment values, API keys, file contents, Git remotes/diffs, history, and secret files are never added automatically.

Metadata receipts are on by default and bounded to 500 records/30 days. They contain coarse route/effect/outcome categories and timing buckets—not intent, command, cwd, context, answers, feedback, stdout, stderr, or diagnostics. Disable them with `history.enabled: false`. The proposal cache contains validated model proposals, not execution results.

Piped stdin is spooled as bounded exact bytes. Valid UTF-8 may be sent as explicit request data; non-UTF-8 sends only presence and byte-count metadata, then is replayed unchanged when the action requests original stdin. Child processes never receive the OpenAI key or private `uhm` control variables. This is not a sandbox.

uhm deliberately does not promise sandboxing, command safety, transactions, or rollback. You remain responsible for commands you authorize.

## Build and test

```sh
cargo fmt -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
cargo build --release --locked
```

Stable CI covers Linux and macOS; a separate job checks Rust 1.82. Dependency rationale lives in [ADR 0001](docs/architecture/0001-core-dependencies-and-msrv.md).

## Contributing and license

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) and [AI_POLICY.md](AI_POLICY.md). Security reports follow [SECURITY.md](SECURITY.md).

MIT. See [LICENSE](LICENSE).
