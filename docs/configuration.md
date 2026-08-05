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

`OPENAI_MODEL` and `DEEPSEEK_MODEL` remain compatibility aliases only when the matching provider is selected and `UHM_MODEL` is absent. A model name never selects or infers a provider.

## Top-level keys

| Key | Default | Notes |
|---|---|---|
| `provider` | `openai` | `openai\|cerebras\|deepseek`; fixed built-in endpoints only |
| `model` | provider default (OpenAI: `gpt-5.6-terra`, Cerebras: `gpt-oss-120b`, DeepSeek: `deepseek-v4-flash`) | bare provider-specific ID; does not change provider; when unset, the selected provider's default is used |
| `selection` | fixed, no alternate/fallback | see below |
| `max_completion_tokens` | `8192` | response token budget |
| `reasoning_effort` | `low` | `none\|minimal\|low\|medium\|high\|xhigh` |
| `stream` | `true` | stream the response |
| `shell` | `auto` | `auto\|sh\|bash\|zsh\|fish\|pwsh\|powershell` |
| `context_mode` | `standard` | `minimal\|standard\|full` |
| `context_timeout_ms` | `150` | one shared context-probe deadline |
| `stdin_max_bytes` | `16777216` (16 MiB) | exact stdin spool limit |
| `stdin_first_byte_timeout_ms` | `1000` | first-byte deadline for non-terminal stdin; expiry proceeds without piped input |
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
  deny_common_env: false    # remove the documented common-secret preset
  deny_env: []              # additional names removed from the child env
  containment: off          # off | bubblewrap
```

`OPENAI_API_KEY`, `CEREBRAS_API_KEY`, `DEEPSEEK_API_KEY`, and uhm's private control variables are removed automatically. Set `deny_common_env: true` to remove a conservative preset covering common AWS, Azure, Google, GitHub, GitLab, database, package-registry, Vault, Kubernetes, Docker, and SSH-agent capability names. Add project-specific names to `deny_env`. Run `uhm doctor environment` to list recognized names that would reach shell children; values are never printed. The preset is opt-in because removing credentials would break commands intentionally targeting those services. Generated Python already starts from an empty environment.

`containment: bubblewrap` is an explicit Linux-only mode. It requires `bwrap`, disables the child's network namespace, makes the host filesystem read-only, and permits writes in the invocation working directory and private program workspace. If requested but unavailable, execution fails before the proposed command starts. This is useful defense in depth, not a confidentiality sandbox: readable host files remain readable, the working directory remains writable, and kernel or Bubblewrap vulnerabilities are outside uhm's control.

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

1. Its environment variable: `OPENAI_API_KEY`, `CEREBRAS_API_KEY`, or `DEEPSEEK_API_KEY`.
2. The matching assignment in a private `0600` secrets file, whose path `uhm doctor` prints.

Provider keys are never passed to generated programs and are removed from ordinary child-command environments. Other inherited credentials require explicit `execution.deny_env` entries.

## HTTPS trust and proxies

`uhm` loads the operating system's trusted certificate roots at runtime. Managed networks that intercept TLS can configure their private root through either standard certificate variables or the application-specific extension bundle:

```sh
# Standard trust-source selection. These follow platform/OpenSSL conventions.
export SSL_CERT_FILE=/path/to/managed-ca-bundle.pem
export SSL_CERT_DIR=/path/to/certificate-directory

# Append one or more private roots to the resolved native/standard roots.
export UHM_CA_BUNDLE=/path/to/private-root.pem
```

`UHM_CA_BUNDLE` extends the resolved trust store; it never disables certificate verification. A configured file that is unreadable, malformed, or contains no certificates is a configuration error. `uhm` has no insecure TLS fallback.

For HTTPS destinations, proxy selection uses `HTTPS_PROXY`, then `ALL_PROXY`, then `HTTP_PROXY`, with each uppercase name checked before its lowercase form. HTTP destinations use `HTTP_PROXY`, then `ALL_PROXY`. `NO_PROXY`/`no_proxy` supports `*`, exact hosts and IP addresses, domain suffixes, bracketed IPv6 addresses, and optional port qualifiers.

Keep the proxy configured in managed environments where direct DNS or egress is unavailable. Run `uhm doctor network` to identify trust configuration, proxy configuration, proxy/CONNECT, DNS, TCP, certificate, handshake, HTTP, and authentication failures separately.

## See also

- [CLI reference](cli-reference.md) — `--provider`, `--model`, `--context`, and the rest of the flag surface
- [Configure a provider](how-to/configure-providers.md) — set keys and select a fixed provider/model pair
- [Configure fallback](how-to/configure-fallback.md) — add one alternate for typed failures
- [Provider reference](reference/providers.md) — provider capabilities and selection behavior
- [Privacy & telemetry](privacy.md) — the on-device vs. outbound boundary
