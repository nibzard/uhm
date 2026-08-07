# uhm

Say what you need. Get the result.

`uhm` is a fast natural-language layer over terminal tools. Ask it to count paragraphs, find a large file, explain a command, or do one local job. It chooses a typed action, runs it, and prints the real output.

It is deliberately smaller than a coding agent. One intent goes in. One bounded job comes out. Then `uhm` exits.

**Documentation site:** <https://nibzard.github.io/uhm/> — install, quickstart, CLI reference, configuration, and guides.

## See it work

[![A terminal demo of uhm turning natural-language requests into results](docs/demo/uhm-demo.svg)](https://nibzard.github.io/uhm/demo/)

Watch the [interactive recording](https://nibzard.github.io/uhm/demo/) or use the [GIF fallback](docs/demo/uhm-demo.gif). The demo uses real OpenAI calls in a disposable repository; its [rebuild and privacy procedure](docs/demo/README.md) is committed with the assets.

## Install

Fast path:

```sh
curl -fsSL https://nibzard.github.io/uhm/install.sh | sh
```

The installer downloads the latest release archive for your platform, verifies it against `SHA256SUMS`, and installs `uhm` to `~/.local/bin` by default. It does not edit shell startup files. Set `UHM_VERSION=v0.6.5` to pin a release or `UHM_INSTALL_DIR=/some/bin` to choose a different install directory.

If you want the manual path instead, download the archive for your machine from the [v0.6.5 release](https://github.com/nibzard/uhm/releases/tag/v0.6.5):

| System | Archive |
|---|---|
| Linux x86-64 | `uhm-v0.6.5-x86_64-unknown-linux-musl.tar.gz` |
| Linux arm64 | `uhm-v0.6.5-aarch64-unknown-linux-musl.tar.gz` |
| macOS Intel | `uhm-v0.6.5-x86_64-apple-darwin.tar.gz` |
| macOS Apple silicon | `uhm-v0.6.5-aarch64-apple-darwin.tar.gz` |

Verify the download before installing it:

```sh
grep 'uhm-v0.6.5-<target>.tar.gz$' SHA256SUMS | sha256sum --check  # Linux
grep 'uhm-v0.6.5-<target>.tar.gz$' SHA256SUMS | shasum -a 256 -c - # macOS
tar --no-same-owner -xzf uhm-v0.6.5-<target>.tar.gz
mkdir -p "$HOME/.local/bin"
install -m 755 uhm-v0.6.5-<target>/uhm "$HOME/.local/bin/uhm"
uhm --version
```

The macOS archives are not notarized yet. If Gatekeeper quarantines the binary, inspect the downloaded file and approve it in System Settings. Never bypass the warning for a mismatched checksum.

Rust users can build the same binary from source:

```sh
cargo install --locked --git https://github.com/nibzard/uhm --tag v0.6.5 uhm-cli
```

The crates.io package is prepared but publication is deferred until ownership is ready.

## First run

The default provider is OpenAI. Give `uhm` an OpenAI API key through the environment or its private secrets file:

```sh
export OPENAI_API_KEY="your-key"
uhm doctor
uhm list the three biggest files
```

`uhm doctor` prints the resolved private secrets path if the key is missing. Put `OPENAI_API_KEY=...` in that file and `chmod 600 <path>` to keep the key out of your shell environment. `uhm doctor network` checks the selected provider. Cerebras and DeepSeek are available as explicit alternatives; set `CEREBRAS_API_KEY` or `DEEPSEEK_API_KEY` and choose with `--provider cerebras|deepseek --model <id>` or persistent configuration.

Managed and corporate networks are supported through the standard upper- or lower-case `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`, and `NO_PROXY` variables. `uhm` loads native certificate roots, honors `SSL_CERT_FILE`/`SSL_CERT_DIR`, and can append a private root with `UHM_CA_BUNDLE`. Certificate verification always remains enabled; see [troubleshooting](docs/troubleshooting.md#proxy-and-tls-certificate-failures).

Before the first outbound request, `uhm` prints a short data notice to stderr. It records that the current notice revision was shown.

## Things to try

Get the answer produced by a local tool:

```sh
uhm 'how many paragraphs are in README.md?'
```

The quotes matter: zsh — the macOS default shell — expands an unquoted `?`, `*`, or `!` before `uhm` runs. Quote any intent that contains `?`, `'`, `*`, or `!`.

Transform files and keep the result pipeable:

```sh
uhm run concatenate the markdown files in docs and write combined.md
```

Use exact piped bytes as request data:

```sh
git diff | uhm ask summarize this for a commit message
```

Ask about a file's content — a question about what a file says needs the file's bytes on stdin:

```sh
cat meeting-notes.md | uhm 'what is this document about'
```

Keep piped content on your machine while letting a generated program process it:

```sh
cat private-report.csv | uhm --local-input --input-format text/csv total the amount column
```

The model receives the intent, byte count, UTF-8 status, and optional format label, but not the piped bytes. If it chooses the bounded Python route, the program receives a private local input path.

Ask for prose without allowing execution:

```sh
uhm explain git log --first-parent --oneline
```

Inspect a proposal without running it:

```sh
uhm run --dry-run count every occurrence of the word world in report.txt
uhm run --review remove old build artifacts
```

Ordinary actions run immediately. `--review` pauses every proposal. `--dry-run` prints exact command bytes and runs nothing. In a non-interactive shell, a proposal that may mutate existing state or file metadata pauses with status 11 because it cannot ask for confirmation; inspect it with `--dry-run`, then rerun with `--force` to authorize the mutation. Metadata-changing utilities such as `touch`, `chmod`, and `chown` are gated conservatively because local classification cannot reliably prove that every target is new. `--force` still shows any warning.

For sensitive environments, `uhm doctor environment` identifies recognized inherited credential names without printing values. `execution.deny_common_env` provides an opt-in common-secret preset, and Linux hosts may explicitly request `execution.containment: bubblewrap` for no-network, read-only-root child execution. See [configuration](docs/configuration.md) for limitations.

If one essential detail is missing, `uhm` can ask one question and revise the proposal. A failed command can get one bounded repair attempt in an interactive terminal. There is no open-ended chat loop.

## How it compares

`uhm` does one bounded job and exits. A chatbot keeps a conversation; an autonomous agent loops over files and repositories until a larger objective is done. `uhm` sits between — one intent in, one bounded job out (a shell action or a generated Python microprogram), then the real result and exit.

Direct alternatives lead with llm-cmd, then hai, cmd-ai, ShellGPT, llm-term, and several more. Broader agents include Claude Code, OpenAI Codex CLI, Warp, and Gemini CLI. When the real problem is remembering a command rather than expressing one, non-AI tools (Atuin, navi, The Fuck, tldr) are usually the better fit.

Full breakdown in [docs/comparison.md](docs/comparison.md).

## Data leaving your machine

The selected provider receives your intent, explicitly piped UTF-8 input unless `--local-input` is used, and the selected context. `standard` context is the default: bounded OS and architecture, target shell, installed-tool booleans, a normalized working directory, bounded Git state, and up to 40 entry names. All modes also disclose the resolved Python 3 path/version and whether `-I -S` works so the model can choose an available route. It does not automatically include file contents, Git remotes or diffs, environment values, API keys, history, or cached results.

Use `uhm context show` to inspect the exact shape. Use `--context minimal` or `context_mode: minimal` to send only the intent and explicitly piped input. OpenAI and DeepSeek requests use the Responses API with `store: false`; explicit Cerebras requests use its fixed Chat Completions endpoint. Provider-side retention is controlled by the selected provider. See the [privacy contract](PRIVACY.md) before opting into any service.

Content-free telemetry is on by default. A summary contains only fixed categories such as platform, shell, route, decision, effect, proposal outcome, process outcome, parent-action acknowledgement, feedback, coarse latency, and cache state. It has no prompt, command, output, path, error text, exact timestamp, or stable ID. Cloudflare processes the HTTPS connection; the Worker does not persist connection metadata in application telemetry or logs. See [PRIVACY.md](PRIVACY.md) for the exact schema and retention.

```sh
uhm telemetry preview       # exact candidate payload for this invocation
uhm telemetry status
uhm telemetry off           # persistent opt-out and clear queued summaries
uhm --no-telemetry do something   # this invocation only
```

`UHM_TELEMETRY=off` and `DO_NOT_TRACK=1` are also honored. Telemetry is best effort and lossy. It runs after result bytes are written, never on local alias or cache hits, and never changes a job's exit status.

## Local records

Private, append-only metadata history is on by default. It contains state transitions, route, effects, process outcome, hashes, and timing categories—never intent, proposal, paths, input, output, or diagnostics. Explicit `diagnostic` and `full` detail levels can retain private per-run artifacts; telemetry remains independently content-free. See [local history](docs/local-history.md).

```sh
uhm history status
uhm history list --limit 20
uhm history show last
uhm history search -- failure
uhm history replay <run-id> --review
uhm history export --output /absolute/path/history.jsonl
uhm history prune --dry-run
uhm feedback good [run-id]
uhm history clear --all
```

The proposal cache holds validated model proposals, not execution results. Runtime directories and files use owner-only permissions on Unix.

## Bounded recovery

Recovery snapshots are off by default: they copy file contents. After its separate disclosure, `uhm recovery on` captures bounded preimages of eligible regular-file outputs that came through the managed Python artifact path. `uhm undo` restores only what it can hash-verify; a later edit is a conflict. `uhm restore --force` reapplies retained evidence when the outcome differs. `uhm recover` asks you for one reviewed, best-effort inverse — and never claims that running it recovered the original. See [bounded recovery](docs/recovery.md).

```sh
uhm recovery on
uhm run --recoverable rewrite report.txt as compact JSON
uhm recovery status
uhm undo <run-id|last>
uhm restore <run-id|last> --force
uhm recover <run-id|last> prefer a local-only inverse
uhm recovery prune --dry-run
```

## Bounded Python microprograms

When a short command or pipeline is clearest, `uhm` still uses the shell. For structured data, statistics, or multifile logic that would become contorted, it may generate one standard-library Python 3 program. The program runs directly as `python3 -I -S <private-source-file>` with a stripped environment, a private workspace, a 10-second wall limit, a 5-second CPU limit, a 16 MiB combined output cap, and best-effort host resource limits. `uhm doctor` reports whether the runtime is available.

This is not a sandbox. The program runs with your user permissions and can read files, use the network, start processes, or cause unmanaged effects if the generated source does so. Isolated/no-site mode and resource limits reduce ambient state and accidents; they do not contain hostile code or protect user-readable secret files. Review shows the exact source, manifest, detected effects, runtime, and limits. `--retain-program` keeps its private temporary workspace only when explicitly requested for debugging.

Artifact programs receive collision-resistant sibling staging paths. After a zero exit, `uhm` verifies regular files, checks sizes, fsyncs, and renames each artifact into place independently. A failed program commits none of its declared staged artifacts, but unrelated side effects cannot be rolled back. Multifile commits are not transactional.

## Shell behavior and limits

A child process cannot change the shell that launched it. Without integration, `uhm` returns the exact typed `cd`, environment, or source action without pretending it was applied. Bash, Zsh, and Fish users can install an optional invocation-only wrapper; see [parent-shell integration](docs/shell-integration.md).

```sh
# Bash (~/.bashrc) or Zsh (~/.zshrc)
eval "$(uhm shell-init bash)"   # use zsh for Zsh

# Fish (~/.config/fish/config.fish)
uhm shell-init fish | source
```

The wrapper uses a private nonce-bound control directory, never application stdout/stderr, and applies only one locally rendered typed action. It does not monitor commands, install prompt hooks, or make sourced code safe.

Warnings for deletion, broad writes, privilege elevation, remote mutation, and process control are convenience signals. They are not a sandbox or a safety guarantee. Model output and the detector can both be wrong. Exit code zero proves only that the process exited zero, not that your intent was satisfied.

The current release has no universal undo, filesystem-wide transaction layer, background agent, native Windows build, shell completion, package installation, JavaScript program runtime, or project-scale code generation. Standalone installs can update themselves from checksum-verified GitHub release assets with `uhm update`.

## CLI reference

```text
uhm [options] <intent>
uhm run|ask|explain [options] <intent>
uhm shell-init bash|zsh|fish
uhm context show [minimal|standard|full]
uhm telemetry [status|preview|on|off]
uhm feedback good|bad [run-id]
uhm repair <run-id|last> [feedback]
uhm recover <run-id|last> [guidance]
uhm undo <run-id|last> [--review]
uhm restore <run-id|last> --force
uhm recovery on|off|status|prune|pin|unpin|resume
uhm update
uhm history [list|show|search|replay|export|prune|clear|status]
uhm config [show|check]
uhm doctor [all] [network|environment]
```

After the first intent word, every argument is opaque user text. A dictated prompt containing `-y`, `--help`, or `--system` cannot change authority. Put `--` before an intent that starts with a hyphen.

`--plain` uses a cooked, ASCII-safe interface with no terminal controls or animation. `--no-motion` keeps color and Unicode but disables animation. `UHM_PLAIN=1`, `NO_COLOR`, `NO_MOTION=1`, and `TERM=dumb` are supported. Requested result data stays on stdout; progress, warnings, and review UI go to stderr. See the [behavior contract](docs/behavior-contract.md) for exit statuses and stream rules.

## Configuration

Copy [config.example.yaml](config.example.yaml) to the path shown by `uhm config show`. Unknown keys and invalid values are errors.

```yaml
provider: openai
model: gpt-5.6-terra
selection:
  mode: fixed
  alternate: null
  fallback_on: []
shell: auto
context_mode: standard

history:
  enabled: true
  detail: metadata

shell_context:
  last_history_entry: false

telemetry:
  enabled: true

program:
  enabled: true
  timeout_secs: 10
  output_max_bytes: 16777216

recovery:
  enabled: false
  max_age_days: 14
  max_total_bytes: 134217728
  max_file_bytes: 8388608
```

## Development

```sh
cargo fmt -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
cargo build --release --locked
(cd telemetry-worker && npm ci && npm test)
```

CI covers stable Rust on Linux and macOS, Rust 1.89, a static Linux build, the packaged crate, and the telemetry gateway. Release tags build and smoke-test four archives, generate SHA-256 checksums, and attach GitHub provenance attestations.

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md), [AI_POLICY.md](AI_POLICY.md), and [SECURITY.md](SECURITY.md).

MIT. See [LICENSE](LICENSE).
