<!-- diataxis: reference -->

# Parent-shell integration reference

The optional integration supports Bash, Zsh, and Fish. It runs only for an explicit `uhm` invocation and installs no prompt, pre-command, debug, tmux, scrollback, daemon, or background hook.

## Supported actions

Protocol version 1 accepts at most one typed action:

- change directory;
- set one environment variable;
- unset one environment variable;
- source one file, including an ordinary activation script.

Aliases, functions, arbitrary snippets, compound actions, `pushd`, `popd`, `umask`, `exit`, `exec`, and traps are not protocol actions. Model-written shell source is never parsed into operands.

## Request and response

For one invocation, the wrapper creates an owner-only control directory beneath the validated runtime root. The request contains protocol version, a 256-bit system-random nonce, shell family, parent cwd, previous status, and creation time.

The child validates ownership, permissions, link counts, ancestry, age, and nonce before atomically publishing one fixed-name response. A local validator reopens that response and prints one audited shell-builtin template with quoted operands. Control data never uses application stdout or stderr.

The wrapper acknowledges `applied` or `failed`. Until matching acknowledgement, history and queued telemetry report the parent outcome as `unknown`.

## Status precedence

| Condition | Result |
|---|---|
| No parent response | Preserve child application status |
| Failed child | Apply nothing and preserve failure |
| Validation or application failure | Return 15 and print recovery guidance |
| Applied action | Return 0 |

Cleanup is attempted on ordinary paths. Sourced code that terminates or replaces the shell can prevent acknowledgement and cleanup.

## Context disclosure

In `standard` mode, integrated invocations may add protocol version, shell family, normalized parent cwd, and previous status. `full` may include raw cwd. `minimal` excludes these machine facts. Integration never uploads environment values.

One-entry native shell history is off by default. When enabled with `shell_context.last_history_entry`, the wrapper samples exactly one entry, previews the sanitized value, and requires confirmation before a provider request. It never reads scrollback, tmux panes, or continuous history.
