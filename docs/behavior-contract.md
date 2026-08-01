# Invocation and outcome contract

The action proposed by the model and the action passed to the child shell are the same byte string. `stdout` belongs to the requested work. Progress, review UI, and warnings belong to `stderr`.

## Behavior table

| Mode | Environment | Default | `--review` | `--dry-run` | `--force` |
|---|---|---|---|---|---|
| auto | TTY | Answer, clarify, or execute; pause for detected consequential effects | Show exact shell proposal, then ask | Emit exact proposal, never execute | Warn for detected effects, then execute |
| auto | non-TTY | Execute unless an advisory pause is required | Do not execute; status 11 | Emit exact proposal, never execute | Warn on stderr, then execute |
| run | TTY | Require a shell action; otherwise status 11/12 | Show exact shell proposal, then ask | Emit exact shell proposal | Warn for detected effects, then execute |
| run | non-TTY | Execute unless an advisory pause is required | Do not execute; status 11 | Emit exact shell proposal | Warn on stderr, then execute |
| ask | any | Return an answer; never execute | Same as default | Same as default | Same as default |
| explain | any | Return an explanation; never execute | Same as default | Same as default | Same as default |

`--review`, `--dry-run`, and `--force` are mutually exclusive. Ask/explain accept the global rendering and model options, but execution controls do not grant them execution authority.

## Application statuses

| Status | Meaning when no child executed |
|---:|---|
| 0 | Answer or dry-run proposal produced successfully |
| 2 | Invalid invocation |
| 10 | API, transport, or structured-proposal failure |
| 11 | Proposed work was not executed, including review cancellation |
| 12 | Clarification is required |
| 13 | Configuration, credentials, or path resolution failed |

If a child executes, its status wins unchanged; Unix signals use the conventional `128 + signal` form. With `--json`, application outcomes use namespace `uhm`. Executed-child receipts use `uhm.child` on stderr so the child's stdout remains unmodified.

## Current-shell actions

Commands such as `cd`, `export`, `unset`, and `alias` run in a child process by default and therefore cannot alter the shell that launched `uhm`. The typed `parent_shell` proposal makes this limitation explicit. Until shell integration is implemented, uhm returns status 11 and suggests `--dry-run`; it never pretends the parent shell changed.
