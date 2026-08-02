# Configuration

`uhm` works with no configuration file: every key has a built-in default. To override one, copy [`config.example.yaml`](https://github.com/nibzard/uhm/blob/main/config.example.yaml) to the path printed by `uhm config show` and uncomment the lines you need.

```sh
uhm config show    # prints each key, its value, and the source that set it
uhm config check   # validates the file and exits non-zero on error
```

The config is **strict**: unknown keys are rejected. Set only the keys you intend to change.

## Resolution precedence

For every key, the winner is the highest step on this list:

1. **Built-in defaults**
2. **`config.yaml`** — overrides defaults for any key you set
3. **`OPENAI_MODEL`** environment variable — overrides `model` only
4. **`--model` / `-m`** flag — overrides `model` for one invocation

> The `OPENAI_MODEL` environment variable overrides the `model` key in `config.yaml`. This is useful for trying a different model without editing the file, but it means a stray `OPENAI_MODEL` in your shell can silently change which model answers. `uhm config show` reports the active source as `OPENAI_MODEL` when the env var is in effect.

## Top-level keys

| Key | Default | Notes |
|---|---|---|
| `model` | `gpt-5.6-terra` | overridden by `OPENAI_MODEL`, then `--model` |
| `max_completion_tokens` | `8192` | response token budget |
| `reasoning_effort` | `low` | `none\|minimal\|low\|medium\|high\|xhigh` |
| `stream` | `true` | stream the response |
| `shell` | `auto` | `auto\|sh\|bash\|zsh\|fish\|pwsh\|powershell` |
| `context_mode` | `standard` | `minimal\|standard\|full` |
| `context_timeout_ms` | `150` | one shared context-probe deadline |
| `stdin_max_bytes` | `16777216` (16 MiB) | exact stdin spool limit |
| `request_max_bytes` | `262144` (256 KiB) | complete untrusted model-input JSON |
| `response_max_bytes` | `2097152` (2 MiB) | streamed or buffered API response |
| `cache_enabled` | `true` | caches the **response** only, never the prompt |
| `cache_ttl_secs` | `86400` (1 day) | response-cache TTL |

## `history` — local decision journal

```yaml
history:
  enabled: true
  detail: metadata          # metadata | diagnostic | full
  capture_output: false     # applies only to diagnostic/full
  redact_paths: true
  max_records: 500
  max_age_days: 30
  max_bytes: 268435456      # 256 MiB
  artifact_max_bytes: 1048576
```

`metadata` is categorical and content-free. Richer levels are explicit and stay local. See [local history](local-history.md).

## `execution` — child-process guardrails

```yaml
execution:
  timeout_secs: 300
  diagnostic_bytes: 65536   # tail per redirected stream
  deny_env: []              # additional names removed from the child env
```

`OPENAI_API_KEY` and uhm's private control variables are removed automatically. Add any other credential names (for example cloud-provider tokens) to `deny_env`; arbitrary inherited secrets cannot be identified reliably. These are operational guardrails, not a sandbox.

## `program` — Python 3 microprogram limits

```yaml
program:
  enabled: true
  source_max_bytes: 65536
  input_max_paths: 64
  output_max_paths: 16
  workspace_max_bytes: 67108864   # 64 MiB
  timeout_secs: 10
  cpu_secs: 5
  address_space_bytes: 268435456  # 256 MiB
  open_files: 64
  child_processes: 16
  output_max_bytes: 16777216      # 16 MiB
  diagnostic_bytes: 1048576       # 1 MiB
```

The only program runtime is `python3`, invoked with `-I -S` and never through a shell. CPU, address-space, open-file, and child-process controls depend on host primitives (some are not enforced on macOS) and are operational guardrails, not a sandbox. See [Behavior & exit codes](behavior-contract.md#program-execution).

## `recovery` — bounded snapshot capture

```yaml
recovery:
  enabled: false           # run `uhm recovery on` to record consent
  max_age_days: 14
  max_total_bytes: 134217728   # 128 MiB
  max_file_bytes: 8388608      # 8 MiB
  scan_limit: 1000
  prune_batch: 100
```

Capture is **separately consented** and off by default. Snapshots contain file bytes; they never leave the device and are excluded from telemetry. See [bounded recovery](recovery.md).

## `shell_context` — sensitive opt-in

```yaml
shell_context:
  last_history_entry: false    # previews one shell-history entry before sending
```

## `telemetry`

```yaml
telemetry:
  enabled: true          # content-free aggregate outcomes; see `uhm telemetry preview`
```

Opt-outs (`uhm telemetry off`, `--no-telemetry`, `UHM_TELEMETRY=off`, `DO_NOT_TRACK=1`) take precedence over this key. See [Privacy & telemetry](privacy.md).

## `aliases` — local shortcuts

```yaml
aliases:
  gst: git status -sb
  ll: ls -lAhF
  ports: ss -tulpn
```

Aliases are short triggers expanded **locally** — no API call, no API key. The expansion still passes through local effect detection and the invocation policy, so consequential effects are still flagged. Aliases are empty by default.

## API key

The key is resolved in this order:

1. `OPENAI_API_KEY` environment variable
2. A private `0600` secrets file (a line of the form `OPENAI_API_KEY=...`), whose path `uhm doctor` prints

Create the file with restricted permissions and add the line with a private editor:

```sh
install -m 600 /dev/null "$(uhm doctor 2>/dev/null | grep -o '/[^ ]*secrets[^ ]*')"
# then edit that file to contain: OPENAI_API_KEY=sk-...
```

The OpenAI key is never passed to a child command's environment. Other inherited credentials require explicit `execution.deny_env` entries.

## See also

- [CLI reference](cli-reference.md) — `--model`, `--context`, and the rest of the flag surface
- [Model selection](model-selection.md) — choosing and overriding the model
- [Privacy & telemetry](privacy.md) — the on-device vs. outbound boundary
