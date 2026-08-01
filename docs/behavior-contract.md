# Invocation and outcome contract

The action proposed by the model and the action passed to the child shell are the same byte string. `stdout` belongs to the requested work. Progress, review UI, and warnings belong to `stderr`.

## Behavior table

| Mode | Environment | Default | `--review` | `--dry-run` | `--force` |
|---|---|---|---|---|---|
| auto | TTY | Answer, clarify, or execute; pause for detected consequential effects | Show exact shell/program proposal, then ask | Emit exact proposal, never execute | Warn for detected effects, then execute |
| auto | non-TTY | Execute unless an advisory pause is required | Do not execute; status 11 | Emit exact proposal, never execute | Warn on stderr, then execute |
| run | TTY | Require an executable shell/program action; otherwise status 11/12 | Show exact shell/program proposal, then ask | Emit exact proposal | Warn for detected effects, then execute |
| run | non-TTY | Execute unless an advisory pause is required | Do not execute; status 11 | Emit exact shell proposal | Warn on stderr, then execute |
| ask | any | Return a typed answer for prose-valued terminal/CLI work | Same as default | Answers remain answers | Same as default |
| explain | any | Return a typed explanation | Same as default | Answers remain answers | Same as default |

`--review`, `--dry-run`, and `--force` are mutually exclusive. Ask/explain accept the global rendering and model options, but execution controls do not grant them execution authority.

The no-argument TTY path collects one intent, completes one bounded interaction, and exits. It is not a REPL. A clarification or failed-command repair may consume at most one second turn; they cannot both occur in the same interaction.

## Application statuses

| Status | Meaning when no child executed |
|---:|---|
| 0 | Answer or dry-run proposal produced successfully |
| 2 | Invalid invocation |
| 10 | API, transport, or structured-proposal failure |
| 11 | Proposed work was not executed, including review cancellation |
| 12 | Clarification is required |
| 13 | Configuration, credentials, or path resolution failed |
| 14 | A model-declared executable requirement is unavailable |

If a child executes, its status wins unchanged; Unix signals use the conventional `128 + signal` form. With `--json`, application outcomes use namespace `uhm`. Executed-child receipts use `uhm.child` on stderr so the child's stdout remains unmodified.

## Current-shell actions

Commands such as `cd`, `export`, `unset`, `source`, activation, and `alias` cannot alter the shell that launched `uhm`. Typed parent-shell proposals and common locally recognized forms return status 11 with the exact not-applied command. Until shell integration is implemented, uhm never runs these in a child and pretends the state persisted. Obfuscated shell syntax remains outside the advisory detector's completeness claim.

## First use and outbound work

Before any OpenAI request or telemetry send, a fresh installation writes a versioned disclosure to stderr and flushes it. Only then does it atomically persist an owner-only notice marker. A changed outbound-data contract increments the revision and makes the notice appear once more.

OpenAI receives the prompt, explicitly supplied UTF-8 stdin, and the selected context. `standard` context is the default. Aggregate telemetry is enabled by default but has only fixed, content-free categories. Its opt-outs are evaluated before an event or queue entry exists. [PRIVACY.md](../PRIVACY.md) is the normative field and retention description.

`--local-input` changes explicitly piped stdin to local-only program data. The request carries only presence, byte count, UTF-8 status, and the optional `--input-format` label. It never carries the input body. Python runtime path/version/isolated-mode inventory is disclosed in every context mode for intentional routing.

Telemetry runs after useful output has been written. Its current-send budget is 100 ms. A model-bound invocation can spend another 200 ms flushing up to ten old entries. A telemetry error, timeout, quota response, or opt-out never changes result bytes or exit status. Local aliases and proposal-cache hits do not create telemetry.

## Terminal modes

`--plain`, `UHM_PLAIN=1`, and `TERM=dumb` select cooked ASCII-safe presentation. Plain output has no ANSI, OSC, cursor, alternate-screen, or animation sequences. `NO_COLOR` disables styling without forcing cooked input. `--no-motion` and `NO_MOTION=1` disable animation without disabling color or Unicode.

Important state has a text label and never depends on color. Layout measures display cells rather than bytes, including wide CJK and emoji glyphs and zero-width combining marks. Ctrl-C cancels the current interaction; Ctrl-D exits an input session.

## One replacement slot

A job makes at most two model calls. The second slot is consumed by one clarification, requested revision, or user-triggered failure repair; these are mutually exclusive. A repair can produce at most a second execution and never happens automatically. Each follow-up is a fresh stateless Responses request reconstructed from bounded inputs; no provider conversation ID or hidden reasoning is retained.

Redirected child streams are teed byte-for-byte while bounded diagnostic tails are retained independently. Terminal-attached streams are inherited and have no automatic diagnostic promise. Child exit status wins, signals map to `128 + signal`, execution has a configurable timeout, and provider/private control secrets are removed from the child environment.

## Program execution

The only standalone program runtime is the literal `python3`. Source is a validated field written to a private file and passed to the resolved interpreter with fixed arguments `-I -S`; it is never evaluated through a shell. The environment is cleared and rebuilt from locale, minimal PATH, private temp path, and JSON input/output manifests. `OPENAI_API_KEY`, cloud credentials, agent sockets, telemetry controls, and other inherited values are not intentionally passed.

The harness is not a sandbox. Generated Python retains the operating-system access of the user, including access to readable secret files. Wall time and combined stdout/stderr are hard harness limits. CPU, address-space, open-file, and child-process controls depend on host primitives and are operational guardrails. Workspace size is measured, not a filesystem quota. Timeout, signal, and overflow terminate the program process group on a best-effort basis.

Stdout programs publish their result bytes only after successful bounded execution. Artifact programs write declared sibling staging paths; after success, each regular file is checked, fsynced, and renamed independently. Failure commits no declared staging path. This does not prevent unmanaged effects or make multiple outputs transactional.
