<!-- diataxis: reference -->

# Configuration reference

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
3. **`UHM_PROVIDER` and `UHM_MODEL`** — override provider/model independently
4. **`--provider` and `--model` / `-m`** — override one invocation

`OPENAI_MODEL` remains a compatibility alias only when the selected provider is OpenAI and `UHM_MODEL` is absent. A model name never selects or infers a provider.

## Top-level keys

| Key | Default | Notes |
|---|---|---|
| `provider` | `openai` | `openai\|cerebras`; fixed built-in endpoints only |
| `model` | `gpt-5.6-terra` | bare provider-specific ID; does not change provider |
| `selection` | fixed, no alternate/fallback | see below |
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

`metadata` is categorical and content-free. Richer levels are explicit and stay local. See the [history reference](reference/history.md).

## `execution` — child-process guardrails

```yaml
execution:
  timeout_secs: 300
  diagnostic_bytes: 65536   # tail per redirected stream
  deny_env: []              # additional names removed from the child env
```

`OPENAI_API_KEY`, `CEREBRAS_API_KEY`, and uhm's private control variables are removed automatically. Add any other credential names (for example cloud-provider tokens) to `deny_env`; arbitrary inherited secrets cannot be identified reliably. These are operational guardrails, not a sandbox.

## `selection` — fixed choice or reviewed evidence

Provider and model are an explicit pair; a model ID never changes the provider implicitly. See [Configure a provider](how-to/configure-providers.md) for task-oriented examples.

```yaml
selection:
  mode: fixed                 # fixed | evidence
  alternate:
    provider: cerebras
    model: gpt-oss-120b
  fallback_on: []             # opt in explicitly; off by default
```

Allowed fallback triggers are `rate_limited`, `transient`, `timeout`, `incomplete`, and `malformed`. Fallback is sequential, occurs only before a proposal is accepted, and consumes the second and final model-call slot. Authentication, missing credentials, and policy rejection fail closed. Enabling a cross-provider alternate changes the first-run disclosure because both fixed endpoints become authorized destinations.

Evidence mode does not score models at runtime. It requires a fresh, reviewed checked-in entry matching the exact provider, endpoint, model fingerprint, request class, and every contract/evaluation hash. The shipped manifest is empty until an untouched holdout clears the frozen policy, so evidence mode currently returns unavailable. Explicit fixed selection remains permitted and reports its qualification status.

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

The only program runtime is `python3`, invoked with `-I -S` through UHM's trusted `uhm_helper_v1` launcher and never through a shell. `input_max_paths` bounds all declared file resources; `output_max_paths` bounds writable resources. CPU, address-space, open-file, and child-process controls depend on host primitives (some are not enforced on macOS) and are operational guardrails, not a sandbox. See [Behavior & exit codes](behavior-contract.md#program-execution).

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

Capture is **separately consented** and off by default. Snapshots contain file bytes; they never leave the device and are excluded from telemetry. See the [recovery reference](reference/recovery.md).

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

## Provider API keys

The selected provider's key is resolved in this order:

1. Its environment variable: `OPENAI_API_KEY` or `CEREBRAS_API_KEY`.
2. The matching assignment in a private `0600` secrets file, whose path `uhm doctor` prints.

Provider keys are never passed to generated programs and are removed from ordinary child-command environments. Other inherited credentials require explicit `execution.deny_env` entries.

## See also

- [CLI reference](cli-reference.md) — `--provider`, `--model`, `--context`, and the rest of the flag surface
- [Configure a provider](how-to/configure-providers.md) — set keys and select a fixed provider/model pair
- [Configure fallback](how-to/configure-fallback.md) — add one alternate for typed failures
- [Provider reference](reference/providers.md) — provider capabilities and selection behavior
- [Privacy & telemetry](privacy.md) — the on-device vs. outbound boundary
