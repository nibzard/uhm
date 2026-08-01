# uhm

Say what you need. Get the result.

`uhm` is a fast natural-language layer over terminal tools. Ask it to count paragraphs, find a large file, explain a command, or do one local job. It chooses a typed action, runs ordinary work, and gives you the real output.

It is deliberately smaller than a coding agent. One intent goes in. One bounded job comes out. Then `uhm` exits.

## Install

Download the archive for your machine from the [v0.1.0 release](https://github.com/nibzard/uhm/releases/tag/v0.1.0):

| System | Archive |
|---|---|
| Linux x86-64 | `uhm-v0.1.0-x86_64-unknown-linux-musl.tar.gz` |
| Linux arm64 | `uhm-v0.1.0-aarch64-unknown-linux-musl.tar.gz` |
| macOS Intel | `uhm-v0.1.0-x86_64-apple-darwin.tar.gz` |
| macOS Apple silicon | `uhm-v0.1.0-aarch64-apple-darwin.tar.gz` |

Verify the download before installing it:

```sh
grep 'uhm-v0.1.0-<target>.tar.gz$' SHA256SUMS | sha256sum --check  # Linux
grep 'uhm-v0.1.0-<target>.tar.gz$' SHA256SUMS | shasum -a 256 -c - # macOS
tar -xzf uhm-v0.1.0-<target>.tar.gz
install -m 755 uhm-v0.1.0-<target>/uhm "$HOME/.local/bin/uhm"
uhm --version
```

The macOS archives are not notarized yet. If Gatekeeper quarantines the binary, inspect the downloaded file and approve it in System Settings. Do not bypass the warning for a file whose checksum does not match.

Rust users can build the same binary from source:

```sh
cargo install --locked --git https://github.com/nibzard/uhm --tag v0.1.0 uhm-cli
```

The crates.io package is prepared but publication is deferred until ownership is ready. GitHub archives are the primary install path.

## First run

Give `uhm` an OpenAI API key through the environment or its private secrets file:

```sh
export OPENAI_API_KEY="your-key"
uhm doctor
uhm -- find the ten biggest files in this directory
```

`uhm doctor` prints the resolved private secrets path if the key is missing. Put `OPENAI_API_KEY=...` in that file and run `chmod 600 <path>` if you do not want the key in your shell environment. `uhm doctor network` makes a separate, explicit OpenAI reachability and authentication check.

Before the first outbound request, `uhm` prints a short data notice to stderr. It records that the current notice revision was shown, then gets out of the way.

## Things to try

Get the answer produced by a local tool:

```sh
uhm -- how many paragraphs are in README.md?
```

Transform files and keep the result pipeable:

```sh
uhm run -- concatenate the markdown files in docs and write combined.md
```

Use exact piped bytes as request data:

```sh
git diff | uhm ask -- summarize this for a commit message
```

Ask for prose without allowing execution:

```sh
uhm explain -- git log --first-parent --oneline
```

Inspect a proposal without running it:

```sh
uhm run --dry-run -- count every occurrence of the word world in report.txt
uhm run --review -- remove old build artifacts
```

Ordinary actions run immediately. `--review` pauses every proposal. `--dry-run` prints exact command bytes and runs nothing. `--force` skips the advisory prompt for a detected consequential action, while still showing the warning.

If one essential detail is missing, `uhm` can ask one question and revise the proposal. A failed command can get one bounded repair attempt in an interactive terminal. There is no open-ended chat loop.

## Data leaving your machine

OpenAI receives your intent, explicitly piped UTF-8 input, and the selected context. `standard` context is the default: bounded OS and architecture, target shell, installed-tool booleans, a normalized working directory, bounded Git state, and up to 40 entry names. It does not automatically include file contents, Git remotes or diffs, environment values, API keys, history, or cached results.

Use `uhm context show` to inspect the exact shape. Use `--context minimal` or `context_mode: minimal` to send only the intent and explicitly piped input. OpenAI requests use the Responses API with `store: false`, which disables Responses application-state storage. OpenAI's default abuse-monitoring logs may still retain API content for up to 30 days unless your organization has approved data-retention controls. See [OpenAI's data controls](https://developers.openai.com/api/docs/guides/your-data#data-retention-controls-for-abuse-monitoring).

Content-free telemetry is on by default. A summary contains only fixed categories such as platform, shell, route, decision, effect, proposal outcome, process outcome, feedback, coarse latency, and cache state. It has no prompt, command, output, path, error text, exact timestamp, or stable ID. Cloudflare processes the HTTPS connection; the Worker does not persist connection metadata in application telemetry or logs. See [PRIVACY.md](PRIVACY.md) for the exact schema and retention.

```sh
uhm telemetry preview       # exact candidate payload for this invocation
uhm telemetry status
uhm telemetry off           # persistent opt-out and clear queued summaries
uhm --no-telemetry -- ...   # this invocation only
```

`UHM_TELEMETRY=off` and `DO_NOT_TRACK=1` are also honored. Telemetry is best effort and lossy. It runs after result bytes are written, never on local alias or cache hits, and never changes a job's exit status.

## Local records

Private metadata receipts are on by default. They contain bounded route, effect, process outcome, and timing categories, never the intent, command, cwd, input, output, or diagnostics. They live in the platform data directory shown by `uhm history status`, with a maximum of 500 records or 30 days.

```sh
uhm history status
uhm feedback good           # attaches one enum to the latest receipt
uhm history clear
```

The proposal cache holds validated model proposals, not execution results. Runtime directories and files use owner-only permissions on Unix.

## Shell behavior and limits

A child process cannot change the shell that launched it. For `cd`, `export`, activation, aliases, and similar requests, `uhm` returns the exact action without pretending it was applied. Copy or evaluate it in your current shell. Automatic parent-shell integration is deferred.

Warnings for deletion, broad writes, privilege elevation, remote mutation, and process control are convenience signals. They are not a sandbox or a safety guarantee. Model output and the detector can both be wrong. Exit code zero proves only that the process exited zero, not that your intent was satisfied.

Version 0.1 has no universal undo, transaction layer, background agent, native Windows build, shell completion, auto-updater, or generated standalone programs.

## CLI reference

```text
uhm [options] -- <intent>
uhm run|ask|explain [options] -- <intent>
uhm context show [minimal|standard|full]
uhm telemetry [status|preview|on|off]
uhm feedback good|bad
uhm history [status|clear]
uhm config [show|check]
uhm doctor [network]
```

After the first intent word, every argument is opaque user text. A dictated prompt containing `-y`, `--help`, or `--system` cannot change authority. Put `--` before an intent that starts with a hyphen.

`--plain` uses a cooked, ASCII-safe interface with no terminal controls or animation. `--no-motion` keeps color and Unicode but disables animation. `UHM_PLAIN=1`, `NO_COLOR`, `NO_MOTION=1`, and `TERM=dumb` are supported. Requested result data stays on stdout; progress, warnings, and review UI go to stderr. See the [behavior contract](docs/behavior-contract.md) for exit statuses and stream rules.

## Configuration

Copy [config.example.yaml](config.example.yaml) to the path shown by `uhm config show`. Unknown keys and invalid values are errors.

```yaml
model: gpt-5.6-terra
shell: auto
context_mode: standard

history:
  enabled: true

telemetry:
  enabled: true
```

## Development

```sh
cargo fmt -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
cargo build --release --locked
(cd telemetry-worker && npm ci && npm test)
```

CI covers stable Rust on Linux and macOS, Rust 1.82, a static Linux build, the packaged crate, and the telemetry gateway. Release tags build and smoke-test four archives, generate SHA-256 checksums, and attach GitHub provenance attestations.

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md), [AI_POLICY.md](AI_POLICY.md), and [SECURITY.md](SECURITY.md).

MIT. See [LICENSE](LICENSE).
