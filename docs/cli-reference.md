<!-- diataxis: reference -->

# CLI reference

The complete command surface, transcribed from the parser. Flags are shown in their long form; all value flags also accept `--flag=value`.

## How parsing works

The **first intent word** is a deliberate boundary. Everything after it is opaque user text, even when it starts with `-`:

```sh
uhm find --force -y --model=x     # prompt is the whole string "find --force -y --model=x"
```

Use the `--` separator only when the intent itself begins with `-`:

```sh
uhm --plain -- "--find" this
```

The removed `-y` / `--yes` flags are rejected with an error. Unknown options are errors; if your intent genuinely starts with `-`, put `--` first.

## Global options

These may appear before or after a subcommand, up to the first intent word.

| Flag | Value | Effect |
|---|---|---|
| `-h, --help` | — | Print help |
| `-V, --version` | — | Print version |
| `--provider` | `openai\|cerebras\|deepseek` | Provider for this invocation; never inferred from the model |
| `-m, --model` | `<id>` | Model id for this invocation (highest precedence) |
| `--shell` | `auto\|sh\|bash\|zsh\|fish\|pwsh\|powershell` | Target shell |
| `--context` | `minimal\|standard\|full` | Outbound context mode |
| `--review` | — | Review every proposal (`run`/`revise`/`edit`/`copy`/`cancel`) |
| `--dry-run` | — | Emit the exact proposal, never execute |
| `--force` | — | Authorize non-interactive mutation and proceed past warnings without confirmation |
| `--plain` | — | Cooked ASCII-safe UI, no styling or animation |
| `--no-motion` | — | Disable animation, keep color and Unicode |
| `--no-stream` | — | Buffer the response instead of streaming |
| `--no-telemetry` | — | Disable telemetry for this invocation |
| `--json` | — | Machine-readable product outcomes on stdout |
| `--local-input` | — | Keep piped bytes on-device for a generated program |
| `--input-format` | `<label>` | Describe local-only input without sending its content (1–64 chars) |
| `--retain-program` | — | Keep the private program workspace for debugging |
| `--recoverable` | — | Capture bounded managed-file preimages for this one job |
| `--fresh, --no-cache` | — | Bypass the response cache for this invocation |
| `-v, --verbose` | — | Verbose diagnostics |

`--review`, `--dry-run`, and `--force` are **pairwise mutually exclusive**. See the [behavior table](behavior-contract.md) for how they interact across TTY/non-TTY and route.

In a non-TTY environment, work that may mutate existing state or file metadata cannot prompt for confirmation and returns status 11 without executing. This conservatively includes `touch`, `chmod`, and `chown`. Use `--dry-run` to inspect the exact proposal, then repeat the invocation with `--force` if the mutation is intended. The JSON `message` gives the same next step. EOF, terminal hangup, or a read error at an interactive review prompt also returns status 11 without execution.

Generated Python uses the `uhm_helper_v1` resource contract. Its process stdin is closed and its cwd is private; `--local-input` exposes piped bytes only when the proposal selects `stdin_mode=local_path`. A preflight contract error may offer one user-triggered complete replacement in an interactive terminal. That replacement shares the global second-model-call slot, never runs automatically in non-TTY or JSON mode, and leaves room for only one execution because the rejected proposal did not run.

## Commands

### Natural-language routes

```
uhm [options] <intent>
uhm run|ask|explain [options] <intent>
```

- **`uhm <intent>`** — the primary form. Turn one intent into one bounded local job and return the result.
- **`run`** — the explicit form of the bare invocation; requires an executable shell/program action.
- **`ask`** — return a typed answer for prose-valued terminal/CLI work; does not execute.
- **`explain`** — return a typed explanation of a command; does not execute.

### Recovering and undoing prior runs

```
uhm repair  <run-id|last> [feedback]
uhm recover <run-id|last> [guidance]
uhm undo    <run-id|last> [--review]
uhm restore <run-id|last> --force
```

`<run-id>` identifies a prior run; `last` means the most recent. `undo` is a local, hash-verified restore that never calls the model. `restore` uses the same retained evidence but records a forced outcome (`--force` is required). See the [recovery reference](reference/recovery.md) for the full contract.

### recovery — snapshot management

```
uhm recovery on|off [--prune]|status [<run-id|last>]|prune [--dry-run] [--all]|pin|unpin <run-id|last>|resume <run-id>
```

- **`on`** — enable recovery snapshot capture (separately consented; writes the marker).
- **`off [--prune]`** — stop new capture. With `--prune`, validated owned snapshots are removed now; otherwise they remain until expiry.
- **`status [<run-id|last>]`** — report enabled state, manifest/snapshot counts, bytes, pinned count, and limits.
- **`prune [--dry-run] [--all]`** — enforce expiry and byte limits; `--dry-run` previews and `--all` also removes current inactive, unpinned evidence.
- **`pin|unpin <run-id|last>`** — pin evidence before its deadline so ordinary prune and expiry skip it; unpinning does not reset the deadline.
- **`resume <run-id>`** — resume a partial managed commit after an interruption (requires a terminal to review).

### history — local decision journal

```
uhm history [list|show|search|replay|export|prune|clear|status]
uhm history list [--limit N] [--failed] [--route ROUTE]
```

Receipts are local and metadata-only by default. See the [history reference](reference/history.md) for detail levels, output capture, path redaction, and retention.

### config — resolved configuration

```
uhm config [show|check]
```

`show` (the default) prints every key with its source. `check` validates the config file and exits non-zero on error. See [Configuration](configuration.md).

`uhm doctor environment` reports recognized inherited credential and capability names that would reach shell children, never their values, and reports whether requested containment is available.

### context — inspect outbound context

```
uhm context show [minimal|standard|full]
```

Prints the exact context object that would be sent with a request, so you can verify the boundary before any call.

### telemetry — aggregate telemetry control

```
uhm telemetry [status|preview|on|off]
```

`preview` prints the candidate summary without sending it. Opt-outs: `uhm telemetry off`, `--no-telemetry`, `UHM_TELEMETRY=off`, `DO_NOT_TRACK=1`. See [Privacy & telemetry](privacy.md).

### feedback — one categorical label

```
uhm feedback good|bad [run-id]
```

Stores one enum on the latest local metadata receipt (or the named run). No free-form text is accepted or sent.

### shell-init — optional parent-shell integration

```
uhm shell-init bash|zsh|fish
```

Emit the optional wrapper that can apply one accepted typed parent-shell action (`cd`, `export`, …). Installs no prompt, pre-command, daemon, or background hooks. See [parent-shell integration](shell-integration.md).

### doctor — local and network checks

```
uhm doctor [all] [network|environment]
```

Local configuration and terminal checks. `network` checks reachability and authentication for the selected provider. `all` inspects all built-in adapters; `all network` also checks every configured credential and endpoint. The command exits 13 if any selected non-optional check fails, and 0 only when all selected checks pass.

### help and version

```
uhm help
uhm version
```

Equivalent to `uhm --help` and `uhm --version`.

## Environment variables

| Variable | Effect |
|---|---|
| `OPENAI_API_KEY` | API key (also readable from a `0600` secrets file) |
| `CEREBRAS_API_KEY` | Cerebras API key (same private secrets-file support) |
| `DEEPSEEK_API_KEY` | DeepSeek API key (same private secrets-file support) |
| `UHM_PROVIDER` | Override provider without inferring it from the model |
| `UHM_MODEL` | Override model for the selected provider |
| `OPENAI_MODEL` | Compatibility model alias only for OpenAI when `UHM_MODEL` is absent |
| `DEEPSEEK_MODEL` | Compatibility model alias only for DeepSeek when `UHM_MODEL` is absent |
| `UHM_TELEMETRY` | `off` disables telemetry |
| `DO_NOT_TRACK` | `1` disables telemetry |
| `UHM_PLAIN` | `1` selects cooked ASCII-safe output |
| `TERM` | `dumb` selects cooked ASCII-safe output |
| `NO_COLOR` | Disable styling without forcing cooked input |
| `NO_MOTION` | `1` disables animation, keeps color and Unicode |

## Exit codes

When no child executes, the application status is:

| Code | Meaning |
|---:|---|
| 0 | Answer or dry-run proposal produced successfully |
| 2 | Invalid invocation |
| 10 | API, transport, or structured-proposal failure |
| 11 | Proposed work was not executed, including review cancellation |
| 12 | Clarification is required |
| 13 | Configuration, credentials, or path resolution failed |
| 14 | A model-declared executable requirement is unavailable |

When a child executes, its status wins unchanged; Unix signals use the conventional `128 + signal`. See [Behavior & exit codes](behavior-contract.md) for the full contract, including parent-shell status precedence.
