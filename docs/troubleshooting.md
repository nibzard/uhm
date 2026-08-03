# Troubleshooting

Start with `uhm doctor`. It checks the selected provider, local configuration, terminal capability, private secrets path, and Python runtime. Add `network` for an explicit reachability/authentication check, or `all` to inspect both built-in adapters:

```sh
uhm doctor
uhm doctor network
uhm doctor all
uhm doctor all network
```

## Install and PATH

**`uhm: command not found`** — the install directory is not on your `PATH`. If you installed to `~/.local/bin`:

```sh
echo 'export PATH="${HOME}/.local/bin:${PATH}"' >> "${HOME}/.${SHELL##*/}rc"
```

If you built with `cargo install`, the binary is in `~/.cargo/bin`, which `cargo` manages on your `PATH`.

**macOS blocks the binary** — the archive is not notarized in this release. After verifying the checksum, approve it in *System Settings → Privacy & Security*, or clear quarantine on the verified file:

```sh
xattr -d com.apple.quarantine "${HOME}/.local/bin/uhm"
```

Never clear quarantine for a file whose checksum did not match.

## API key and authentication

`uhm` reads the selected provider's environment variable first (`OPENAI_API_KEY` or `CEREBRAS_API_KEY`), then the matching assignment in a private `0600` secrets file whose path `uhm doctor` prints. If the key is missing or rejected:

```sh
export OPENAI_API_KEY="sk-..."
uhm doctor network     # confirms the key works end to end
```

Prefer the secrets file to keep the key out of your shell environment, then `chmod 600 <path>` on the path `uhm doctor` prints.

Exit code **13** means a configuration or credentials problem.

## Model and API errors

Exit code **10** covers model and API failures: rate limits, server errors, bad responses, or a model name your account cannot use.

- Check provider and model together. `--provider` and `--model` override them independently; model names never imply a provider. `OPENAI_MODEL` is ignored for Cerebras. See [Configuration](configuration.md).
- A key that authenticates but lacks access to the selected model still fails here.
- If evidence mode reports unavailable, no exact current reviewed qualification exists. Choose an explicit fixed provider/model or update the checked-in evidence through the qualification workflow.
- A configured fallback happens only for its typed allowlist and uses the second/final model call. A later clarification or repair is then intentionally unavailable.

## Nothing ran (exit 11)

Exit code **11** means no command executed. Common causes:

- `--review` was used and the proposal was cancelled at the prompt.
- `--dry-run` was used — nothing runs by design; it only prints exact command bytes.
- The model declined to propose an action, or asked a question instead.

## A clarification loop (exit 12)

Exit code **12** means `uhm` asked a question and could not settle on a proposal. Re-run with the missing detail stated directly in the intent.

## Terminal display issues

If output looks garbled, animates when you do not want it to, or your terminal reports odd capabilities:

- `--plain` — cooked, ASCII-safe, no terminal controls or animation.
- `--no-motion` — keep color and Unicode, disable animation.
- Environment fallbacks: `UHM_PLAIN=1`, `NO_COLOR`, `NO_MOTION=1`, `TERM=dumb`.

Result data always goes to stdout; progress, warnings, and the review UI go to stderr.

## Recovery conflicts

`uhm undo` restores only what it can hash-verify. If a file changed after the snapshot, that is a conflict and the undo refuses rather than overwrite your newer work.

- `uhm restore <run-id|last> --force` reapplies retained evidence when the current outcome differs.
- `uhm recover <run-id|last>` asks for one reviewed, best-effort inverse and never claims it restored the original.

Recovery is off by default because it copies file contents. See [Bounded recovery](recovery.md).

## Telemetry

Telemetry is best effort and lossy, and never changes a job's exit status. To inspect or disable it:

```sh
uhm telemetry preview     # exact candidate payload for this invocation
uhm telemetry off         # persistent opt-out, clears queued summaries
UHM_TELEMETRY=off         # or DO_NOT_TRACK=1
```

## Exit codes at a glance

| Code | Meaning |
|---|---|
| 0 | success |
| 2 | usage error |
| 10 | model or API error |
| 11 | not executed (review cancelled, dry-run, or declined) |
| 12 | clarification needed |
| 13 | configuration or credentials |
| 14 | unavailable (for example, a feature not ready) |

A child process exit status wins when present; termination by signal is reported as `128 + signal`. See [Behavior & exit codes](behavior-contract.md) for the full contract.

## Still stuck

- [CLI reference](cli-reference.md) — every command and flag
- [Configuration](configuration.md) — config file, precedence, aliases
- Run `uhm help` for the built-in synopsis.
