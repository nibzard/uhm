# uhm

Say what you need. Get the result. **The result, not the command.**

You know the result you want. The command, the flag, or the one-liner will not come. `uhm` is for that moment — the name is the sound it starts with.

`uhm` is an AI assistant for the terminal. Say the job in plain words. `uhm` picks one way to do it — a shell command or a short Python program — runs it, and prints the real output. Then it exits.

It is deliberately smaller than a coding agent. One or two model calls per job. Nothing loops, nothing stays running. Fast enough to become a reflex.

Each job sends your intent to a model provider and spends a small amount of API credit, so you need a provider API key. OpenAI is the default provider; Cerebras and DeepSeek are explicit alternatives.

New to the idea? Read [what `uhm` is](docs/concepts.md) before you install. Full documentation site: <https://nibzard.github.io/uhm/>.

## See it work

[![A terminal demo of uhm turning natural-language requests into results](docs/demo/uhm-demo.svg)](https://nibzard.github.io/uhm/demo/)

Six real jobs in under a minute. Watch the [interactive recording](https://nibzard.github.io/uhm/demo/), or open the [GIF fallback](docs/demo/uhm-demo.gif). The demo uses real OpenAI calls in a disposable repository. Its [rebuild and privacy procedure](docs/demo/README.md) is committed with the assets.

## Why uhm

- **You get the result, not a command.** The command or program is an implementation detail. You can inspect it whenever you want.
- **One job per invocation.** `uhm` cannot spiral into an unplanned session. At most one clarifying question, one revision, and one repair attempt.
- **The terminal stays honest.** Exact bytes, real exit codes, result data on stdout, progress on stderr. Pipes keep working.
- **No safety theater.** Warnings are convenience signals. `uhm` never claims a sandbox. Exit code zero only means the process exited zero.

## When uhm is not the tool

- **The job needs many steps, project edits, or autonomous work.** Use a coding agent: Claude Code, OpenAI Codex CLI, Gemini CLI, or Warp.
- **You want a conversation.** `uhm` is not a chatbot. It asks at most one question, then finishes the job.
- **You have run the command before and cannot find it.** History search (Atuin) or cheatsheets (tldr, navi) answer recall faster, without a model or a bill.
- **The job needs no model at all.** A plain command, an alias, or a shell function is cheaper.

## Install and first run

```sh
curl -fsSL https://nibzard.github.io/uhm/install.sh | sh
export OPENAI_API_KEY="your-key"    # https://platform.openai.com/api-keys
uhm doctor
uhm list the three biggest files
```

The installer downloads the release archive for your platform, verifies it against `SHA256SUMS`, and installs `uhm` to `~/.local/bin`. It does not edit your shell startup files. Set `UHM_VERSION=v0.6.6` to pin a release or `UHM_INSTALL_DIR=/some/bin` to choose a different install directory.

For the manual path, download an archive from the [v0.6.6 release](https://github.com/nibzard/uhm/releases/tag/v0.6.6) and follow the checksum steps in the [install guide](docs/install.md). Rust users can build the same binary:

```sh
cargo install --locked --git https://github.com/nibzard/uhm --tag v0.6.6 uhm-cli
```

`uhm doctor` prints the resolved private secrets path if the key is missing. Put `OPENAI_API_KEY=...` in that file and `chmod 600 <path>` to keep the key out of your shell environment. `uhm doctor network` checks the selected provider. Cerebras and DeepSeek keys work the same way; see [configure a provider](docs/how-to/configure-providers.md). Before the first outbound request, `uhm` prints a short data notice on stderr. Corporate proxy and certificate setup lives in [troubleshooting](docs/troubleshooting.md#proxy-and-tls-certificate-failures).

## Things to try

Get the answer produced by a local tool:

```sh
uhm 'how many paragraphs are in README.md?'
```

The quotes matter. zsh — the macOS default shell — expands an unquoted `?`, `*`, or `!` before `uhm` runs. Quote any intent that contains `?`, `'`, `*`, or `!`.

Turn a diff into a commit message:

```sh
git diff | uhm ask summarize this for a commit message
```

Piped bytes are sent to the provider as part of the request. Keep them on your machine with `--local-input`:

```sh
cat private-report.csv | uhm --local-input --input-format text/csv total the amount column
```

The model receives the intent, a byte count, a UTF-8 status, and the format label — not the bytes. A generated program reads the bytes from a private local file.

More recipes: the [quickstart](docs/getting-started.md) and the [cookbook](docs/cookbook.md). To preview or gate any job, use `--dry-run` and `--review`; see the [behavior contract](docs/behavior-contract.md).

## How it compares

Today the command will not come, so you leave the terminal. You search the web or paste the job into a chat tab. You read the answer, adapt it, and run it. `uhm` removes the detour: say the job, receive the result, stay in the shell.

`uhm` is not a chatbot and not a coding agent. One intent goes in. One bounded job comes out — a single shell action or a short generated Python program, run once. Then `uhm` exits. Command-first tools such as llm-cmd hand you a command to inspect and run. `uhm` hands you the result and lets you inspect the implementation when you want it.

The full breakdown, with 60 surveyed tools: [docs/comparison.md](docs/comparison.md).

## Data leaving your machine

- **Sent:** your intent; explicitly piped input, unless you use `--local-input`; and a bounded context. The context holds OS, architecture, shell, installed-tool booleans, a normalized working directory, bounded Git state, and up to 40 entry names. `uhm context show` prints the exact payload.
- **Never sent automatically:** file contents, environment values, API keys, history, Git remotes or diffs, cached results.
- **Telemetry:** content-free categories only — no prompts, commands, paths, or outputs. On by default. Turn it off with `uhm telemetry off`. `uhm telemetry preview` shows the exact candidate payload.

OpenAI and DeepSeek requests use the Responses API with `store: false`; explicit Cerebras requests use its fixed Chat Completions endpoint. The full contract, with provider retention links, is [PRIVACY.md](PRIVACY.md).

## Honest limits

- **Local records.** Every run appends a private metadata receipt: state, route, outcome, and timing categories. Never intent or output. See [local history](docs/local-history.md).
- **Bounded recovery.** Off by default. `uhm recovery on` saves copies of the files a generated program will rewrite. `uhm undo` restores only what it can hash-verify. See [bounded recovery](docs/recovery.md).
- **Short Python programs.** For structured data, `uhm` may generate one standard-library Python program. It runs with a stripped environment, a 10-second limit, and a 16 MiB output cap. This is not a sandbox. See the [program contract](docs/reference/program.md).
- **Parent-shell actions.** A child process cannot change your shell. An optional wrapper applies `cd`, environment, and source actions. See [parent-shell integration](docs/shell-integration.md).
- **Exit statuses and streams.** Review pauses, exit code meanings, and stdout/stderr rules live in the [behavior contract](docs/behavior-contract.md).

## CLI

```text
uhm [options] <intent>
uhm run|ask|explain [options] <intent>
uhm doctor [all] [network|environment]
```

Every command, flag, and exit code: the [CLI reference](docs/cli-reference.md). Put `--` before an intent that starts with a hyphen. After the first intent word, every argument is your text. A dictated `-y` or `--help` cannot change authority.

## Configuration

Copy [config.example.yaml](config.example.yaml) to the path shown by `uhm config show`. Unknown keys and invalid values are errors. Keys, defaults, validation, and provider selection: the [configuration reference](docs/configuration.md).

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
