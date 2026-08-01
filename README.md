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
| `uhm ask -- <question>` | Answer only; never execute |
| `uhm explain -- <command>` | Explain only; never execute |
| `uhm history [n]` | Show optional local execution history |
| `uhm config show` | Show resolved values and sources |
| `uhm config check` | Strictly parse and validate configuration |
| `uhm context` | Show the machine context used for proposals |
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
model: gpt-5.6-luna
shell: auto
stream: true
include_history: false

aliases:
  gst: git status -sb
```

Environment overrides:

| Variable | Purpose |
|---|---|
| `OPENAI_API_KEY` | API credential |
| `OPENAI_MODEL` | Model ID |
| `OPENAI_BASE_URL` | API base URL |
| `UHM_PLAIN` | Disable terminal presentation features |
| `NO_COLOR` | Disable color |

The optional private secrets file is `<data-dir>/uhm/secrets` and contains `OPENAI_API_KEY=...`. On Unix, uhm requires private file permissions. Runtime directories and files are created as `0700` and `0600` respectively.

## Data and trust

By default, the request and a bounded terminal context are sent to the configured OpenAI endpoint because platform and project state materially improve commands. Context includes platform, shell, working directory, Git state, and optionally a short directory listing (`include_ls: false` disables the listing). Set `context_mode: request_only` to send only the request. Static application rules remain in the trusted system message; requests, filenames, Git metadata, piped input, and context are sent once as untrusted JSON input.

Command history is off by default because commands can contain secrets. The proposal cache contains model proposals, not execution results, and is keyed by model, endpoint/API family, schema/policy versions, generation parameters, context, and request.

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
