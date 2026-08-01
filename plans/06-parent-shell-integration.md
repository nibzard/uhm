# Plan 6 — Add optional parent-shell integration

## Purpose and dependency

This post-release plan lets an explicitly installed wrapper apply selected actions to the user's current Bash, Zsh, or Fish process. It depends on Plan 3's public binary, terminal behavior, configuration, notices, and packaging. It does not depend on Plan 5 and must not grow a history engine or recovery system.

The problem is a process boundary, not an LLM limitation. A normal `uhm` binary runs as a child: it can create files and launch programs, but its `cd`, `export`, `unset`, `source`, activation, or alias changes disappear when the child exits. Without integration, `uhm` must continue to return the exact command and explain that it was not applied. With integration, a small static wrapper may apply one already-reviewed, typed parent-shell action and acknowledge what it observed.

## Full implementation description

### 1. Generate a static, auditable wrapper

Add:

```text
uhm shell-init bash
uhm shell-init zsh
uhm shell-init fish
```

Each invocation emits versioned static shell source for the selected shell. Generation must not contact OpenAI, inspect the working directory, load a prompt, or depend on user intent. Documentation shows the normal installation patterns and how to remove the integration. The generated function delegates ordinary execution to the real `uhm` binary, preserves its stdout/stderr, applies a parent action only through the private protocol below, cleans up, and returns a documented status.

Keep shell templates as reviewed repository assets with golden tests. Include a recursion guard so the wrapper invokes the binary rather than calling itself. Resolve the binary consistently without baking an unverified writable path into persistent shell configuration.

### 2. Use a private control-file protocol, never stdout

The wrapper creates a unique `0700` directory beneath a validated, current-user-owned runtime root using `mktemp`-equivalent facilities and a restrictive umask. It creates a fixed-name `0600` request file containing a protocol version, cryptographic nonce, selected shell, parent cwd, and immediately preceding exit status, then invokes the binary with the private directory through a reserved integration argument. Before writing anything, the binary opens the directory without following links and verifies owner, mode, link/type properties, ancestry under the resolved runtime root, request ownership/mode, and nonce. It creates only a fixed-name response through descriptor-relative exclusive creation and atomic replacement; it never accepts a caller-selected output filename.

Replace Plan 2's v0.1 opaque `require_parent_shell(command, ...)` payload with a strict typed variant whose required fields include a `kind` enum plus nullable `path`, `name`, and `value` operands. Client validation enforces the legal field matrix for each kind and locally renders the exact fallback command; it does not parse a model-written shell string into operands.

If the completed job proposes a persistent parent-shell effect and the user accepts it under the current review/warning policy, the binary atomically writes one typed response to that directory. The response echoes the protocol version and nonce and contains at most one parent action:

```text
change_directory(path)
set_environment(name, value)
unset_environment(name)
source_file(path)
```

Do not place generated shell source in the control schema. A separate internal helper reopens the response descriptor-relatively, validates owner/mode/link count, nonce, schema, action count, field lengths, environment names, and path representation, then renders one audited shell-specific builtin template with encoded operands. The wrapper evaluates only this validator's fixed template output, never response text or model output directly. Unknown versions/actions, multiple/compound actions, direct `exit`/`exec`/`trap`, malformed files, ownership or permission mismatches, and nonce failures result in no parent change.

The protocol is not a sandbox or a safety guarantee. `source_file` can execute arbitrary code with the user's authority, including code that exits or replaces the shell before acknowledgement or cleanup. It is separately classified as consequential and receives a literal warning, confirmation, and `--force` escape hatch. `cd` and set/unset operations still pass through ordinary effect review. Alias/function definition and arbitrary shell snippets remain out of the initial protocol.

### 3. Preserve a clean result and exit-status contract

Application results remain on stdout and UI/progress/warnings remain on stderr. Control data never shares either stream, so pipelines and command substitution cannot accidentally consume it.

After the child exits, the wrapper validates and applies any response, then invokes `uhm shell-ack --run-id <id> --nonce <nonce> --status applied|failed`. The primary child must defer its telemetry candidate into Plan 3's private queue with `parent_action=unknown`; it cannot know the parent outcome yet. The acknowledgement atomically updates the local receipt and still-unsent candidate, makes no model request, and only then may perform Plan 3's bounded post-result telemetry handoff. If acknowledgement never runs, the next flush retains the honest `unknown` outcome. The wrapper removes its private directory through normal flow and traps where the shell permits, but documentation must not promise cleanup or acknowledgement if sourced code terminates/replaces the shell. Ordinary child exit status alone is never interpreted as control data. Define final wrapper status precedence explicitly:

- If no parent action was requested, return the child application's status.
- If the child job failed, do not apply a parent action and return that failure.
- If the child succeeded but validation or application failed, return a distinct integration failure status and a literal recovery instruction.
- If application succeeded, return success without masking warnings already emitted by the child.

Non-integrated invocation never claims that parent state changed. In TTY mode it gives a concise instruction; in structured mode it reports `requires_parent_shell=true` and the reviewed command/action.

### 4. Send only bounded invocation-time shell context

The wrapper may provide these facts for the current invocation:

- Selected shell family and integration protocol version.
- Parent working directory.
- Exit status of the command immediately preceding `uhm`.
- Existing SSH/tmux/TTY capability markers already allowed by the context policy.

These values enter the normal `minimal|standard|full` context selection and first-use disclosure; integration is not permission to send additional machine state. The binary treats wrapper-supplied strings as untrusted data, validates types and bounds, and never inserts them into system instructions.

An optional `shell_context.last_history_entry` setting may capture exactly one shell-native history entry at invocation time for jobs such as “fix the last command.” It is off by default because history entries often contain secrets. Before that entry is sent, `uhm` previews the exact text and requires the user to continue. Cancellation discards it. This feature reads neither scrollback nor a tmux pane and retains no shell history of its own.

### 5. Avoid continuous observation

Do not install `preexec`, `precmd`, `DEBUG`, prompt, or equivalent hooks to watch all commands. Do not run a daemon, background recorder, or persistent shell subprocess. The wrapper exists only as a function around an explicit `uhm` invocation; the immediately preceding exit status and optional one-entry history lookup are sampled then and discarded according to the current receipt policy.

No feature in this plan requires Plan 5's content journal. If local history is present, the ordinary outcome recorder may store the typed acknowledgement under its configured detail level; shell integration itself remains functional when history is disabled.

### 6. Isolate implementation areas and test real shells

Add a focused shell-integration module and template assets rather than extending command execution with shell-specific string concatenation. Expected code areas include `src/args.rs`, `src/main.rs`, `src/shell.rs`, `src/context.rs`, `src/command.rs`, `src/config.rs`, `src/dirs.rs`, structured result types, and new `src/shell_integration/` modules or equivalent.

Build PTY integration fixtures using actual supported Bash, Zsh, and Fish versions on Linux and macOS. Cover local shells, SSH PTYs, tmux, nested shells, no TTY, signals, stale files, concurrent invocations, and installation/uninstallation documentation. A missing optional shell in CI may skip only its platform matrix leg, not unit/golden protocol coverage.

## Expected outcomes

- `uhm` can apply selected `cd`, environment, sourcing, and activation effects to the current supported shell when the user installs the integration, then distinguish acknowledged success, failure, and unknown application state.
- Users without the wrapper receive an honest command/instruction instead of a false success.
- Ordinary command results remain pipe-friendly because parent-control data never travels over stdout or stderr.
- The model gains useful current cwd and last-exit context without continuous command monitoring.
- Users may explicitly provide one previewed history entry for a relevant job without turning `uhm` into a shell-history collector.
- The integration remains removable, auditable, optional, and independent of cloud or content-rich local history.

## Definition of done

- `uhm shell-init bash|zsh|fish` produces stable, versioned output from golden-tested repository templates and performs no network or model request.
- Installation and removal instructions work on supported Linux/macOS shell startup layouts and clearly describe the child/parent process boundary.
- End-to-end PTY tests prove `cd`, set/unset environment, and normal source/activation actions persist in the parent shell only after accepted typed proposals.
- The same jobs without integration report `requires_parent_shell` and never claim the state was applied.
- Protocol tests reject unknown versions/actions, multiple/compound actions, direct exit/exec/trap actions, caller-selected output files, traversal, symlinks, stale or replayed nonces, loose permissions, wrong owner/link count, malformed fields, oversized content, and control paths outside the validated private runtime root.
- Shell renderers round-trip adversarial spaces, quotes, newlines, Unicode, leading dashes, and shell metacharacters without adding an unintended action.
- stdout byte-for-byte contains only the job result; progress, warnings, and integration diagnostics remain on stderr; control payloads appear on neither.
- Exit-status and acknowledgement tests enforce child-failure, validation-failure, application-failure, cancellation, success, and missing-ack precedence without masking the original error or treating a dedicated child code as control data.
- Telemetry tests prove wrapper invocations are not sent as applied before acknowledgement, successful/failed acknowledgement updates only the matching unsent candidate, missing acknowledgement remains `unknown`, and the normal privacy/latency/opt-out contract still holds.
- Concurrent invocations cannot consume or overwrite each other's control files. Cleanup covers normal exit, model/API failure, signal, and ordinary wrapper interruption; a source fixture that terminates the shell is recorded as potentially unacknowledged rather than given an impossible cleanup guarantee.
- Context tests prove only allowed bounded fields reach an OpenAI request and wrapper values remain untrusted user-context data.
- Last-history-entry mode is off by default, previews exactly one entry, sends nothing after cancellation, and never reads scrollback, tmux panes, or more than the requested entry.
- Bash, Zsh, and Fish tests pass locally and through representative tmux and SSH PTY fixtures; history-disabled operation remains fully functional.

## Anti-goals

- Do not include shell integration in the public v0.1 gate or make it mandatory for ordinary `uhm` use.
- Do not install pre-command hooks, scrape scrollback or tmux panes, monitor all commands, or run a daemon/background recorder.
- Do not create a local history engine, replay feature, receipt search, snapshot store, undo, or recovery behavior in this plan.
- Do not use stdout, stderr, a predictable shared file, or an unvalidated `eval` string as the parent-control transport.
- Do not claim that the control protocol sandboxes sourced code or makes consequential shell changes safe.
- Do not support arbitrary shell dialects, PowerShell, native Windows, or shells without explicit templates and integration tests.
- Do not allow wrapper presence to broaden the standard machine-context policy or upload environment values and shell history by default.
- Do not silently apply more than one persistent parent-shell action per job or continue after an integration validation failure.
