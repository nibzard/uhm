# Privacy

This document describes the current `uhm` data contract. The short version: terminal work goes to OpenAI only when it is part of the model request; product telemetry contains fixed categories and no content or stable identity.

## OpenAI requests

`uhm` uses OpenAI's Responses API and sends `store: false`. OpenAI documents this as disabling Responses application-state storage; it is not the same as Zero Data Retention. By default, OpenAI may retain API customer content in abuse-monitoring logs for up to 30 days unless longer retention is legally or operationally required. Organizations approved for Modified Abuse Monitoring or Zero Data Retention have different controls. Read [OpenAI's current data-control documentation](https://developers.openai.com/api/docs/guides/your-data#data-retention-controls-for-abuse-monitoring) for the provider-side policy.

A request contains:

- your natural-language intent;
- explicitly piped UTF-8 input, when present;
- the selected bounded context object;
- fixed instructions and strict tool schemas needed to return a typed action.

With `--local-input`, the piped body is omitted and replaced by presence, byte count, UTF-8 status, and an optional user-declared format label. The generated local program can read the private spooled bytes. The flag requires piped input.

`standard` context is the default. It contains OS and architecture, the target shell, common-tool presence booleans, a normalized working directory, bounded Git state, and up to 40 directory entry names. `minimal` contains no general machine fields. All modes contain resolved Python 3 path/version and isolated/no-site availability so the model does not propose an unavailable runtime. `full` adds bounded host, user, shell-version, and tool-version fields.

Use `uhm context show minimal|standard|full` before a request to inspect the exact shape. Select `minimal` with `--context minimal` or in `config.yaml`.

`uhm` does not automatically add environment values, secrets, file contents, Git remotes, Git diffs, local receipts, cached proposals, stdout, stderr, or clipboard data. The generated command may read files or contact services when executed; that behavior belongs to the command and is described by its declared and detected effects.

`uhm recover` is the one explicit exception for local receipts. It prints the exact bounded subset before sending, requires terminal approval, and sends only the retained original intent, typed proposal, coarse outcome, optional guidance, and a fixed best-effort label. The selected current context is also sent under the normal context policy. It never sends the full journal, unrelated runs, recovery manifests, snapshot paths, or snapshot bytes.

OpenAI's terms and retention controls can change. The linked provider documentation, not this repository, is authoritative for provider-side handling.

## Aggregate telemetry

Telemetry is enabled by default after the first-use notice. The CLI sends HTTPS requests to `https://uhm-telemetry.nikola-balic.workers.dev/v1/events`, a Cloudflare Worker operated for this project.

An interaction summary has exactly these fields:

```json
{
  "v": 2,
  "event": "interaction_summary",
  "release": "0.1",
  "os": "linux",
  "arch": "x86_64",
  "shell": "bash",
  "mode": "auto",
  "route": "shell",
  "decision": "ran",
  "effects": "read_local",
  "proposal_outcome": "valid",
  "execution_outcome": "exit_zero",
  "user_feedback": "unknown",
  "latency": "1s_2s",
  "cache": "miss",
  "parent_action": "not_applicable",
  "interactive": true,
  "notice_revision": 3
}
```

Every string after `release` is selected from a short server-maintained enum. `parent_action` is only `not_applicable`, `unknown`, `applied`, or `failed`; an integrated action remains `unknown` until the wrapper acknowledges it. `release` is major/minor only. `interactive` is a boolean. `uhm telemetry preview` prints the candidate schema without sending it.

The Worker rejects unknown keys, unknown enum values, unsupported versions, non-JSON requests, and bodies of 2 KiB or more. It writes accepted categories to Workers Analytics Engine. Raw Analytics Engine data is retained for three months. No raw event is copied to a durable identity store.

Cloudflare necessarily processes the connection IP and other connection metadata to serve HTTPS. Analytics Engine adds its own ingestion timestamp. The CLI does not send an IP, User-Agent, exact client timestamp, or identifier in its JSON. The Worker does not read or persist request headers, IP, geolocation, User-Agent, or request body in application telemetry. Worker observability and invocation logs are disabled.

The project uses aggregate telemetry to understand route, effect, proposal, execution, latency, cache, and explicit-feedback distributions. It cannot calculate daily active users, retention, or cross-invocation funnels because there is no user, installation, device, session, repository, or pseudonymous ID.

Telemetry never contains:

- prompts, questions, clarification text, feedback text, commands, or hashes of them;
- answers, stdin, stdout, stderr, clipboard contents, panic text, stack traces, or raw errors;
- cwd, paths, filenames, directory entries, repositories, branches, remotes, diffs, aliases, receipts, or cached responses;
- username, hostname, locale, timezone, exact client time, environment or config values, model ID, API keys, or tokens.

## Feedback

`uhm feedback good|bad` stores that one enum on the latest local metadata receipt. If its interaction summary is still queued, the enum is added there. Otherwise `uhm` may send a separate `feedback_summary` containing the same permitted coarse categories and no join key. Aggregate queries exclude feedback summaries from interaction counts.

No free-form feedback is accepted or sent.

## Delivery and opt-out

Telemetry is best effort and deliberately lossy. The current summary gets one post-result attempt with a 100 ms hard deadline. On the next model-bound invocation, up to ten older summaries may flush within a separate 200 ms budget. There is no daemon. Definite failures before a request is sent may be queued; ambiguous outcomes are dropped. A crash can produce a rare duplicate, and network or quota failures can lose events.

Queued summaries are separate owner-only files, capped at 20 files and seven days. Local alias and proposal-cache hits do not create or send telemetry.

Opt out in any of these ways:

```sh
uhm telemetry off
uhm --no-telemetry -- <intent>
UHM_TELEMETRY=off uhm -- <intent>
DO_NOT_TRACK=1 uhm -- <intent>
```

You can also set `telemetry.enabled: false` in `config.yaml`. Every opt-out is checked before an event is created or a queued event is claimed. `uhm telemetry off` writes the persistent opt-out first, waits for any send already in flight, clears queued summaries, and returns with no local send in flight. It cannot retract an event that the server already accepted. Use `uhm telemetry on` to remove the persistent opt-out; environment and config opt-outs still take precedence.

## Local storage

Metadata receipts are enabled by default and stored under the platform data directory reported by `uhm history status`. They are capped at 500 records and 30 days. Receipts contain categorical execution metadata, including only coarse program route/runtime/outcome, never intent, commands, program source, manifests, paths, terminal content, or diagnostics. `uhm history clear` removes them. Set `history.enabled: false` to stop recording new receipts.

Recovery snapshot capture is separately consented and off by default. When enabled globally with `uhm recovery on` or once with `--recoverable`, eligible managed file preimages are copied below private `runs/<run-id>/snapshots/` files. The default limits are 8 MiB per file, 128 MiB total, and 14 days. `uhm recovery status` reports usage; `uhm recovery off` stops new capture; `uhm recovery prune` removes validated owned snapshots and leaves expiry tombstones. Normal history export excludes snapshots. Snapshot bytes, paths, manifests, and hashes are excluded from telemetry.

The same private data directory contains the disclosure revision, optional secrets file, telemetry opt-out marker, and short-lived telemetry queue. The cache directory contains validated model proposals. Unix directories and files created by `uhm` use modes `0700` and `0600`.

`uhm` has no account, cloud history, or cross-device sync.
