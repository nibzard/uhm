# Parent-shell integration

A normal program cannot persistently change the shell that launched it. `uhm` therefore treats `cd`, environment changes, and sourcing as typed parent-shell actions. Without integration it shows the exact locally rendered fallback, returns status 11, and reports `requires_parent_shell=true` in JSON.

The optional wrapper supports Bash, Zsh, and Fish. It runs only when you explicitly invoke `uhm`; it installs no prompt, pre-command, debug, tmux, scrollback, daemon, or background hooks.

## Install and remove

First inspect the static source:

```sh
uhm shell-init bash   # or zsh / fish
```

For Bash, add this to `~/.bashrc`:

```sh
eval "$(uhm shell-init bash)"
```

For Zsh, add this to `~/.zshrc`:

```sh
eval "$(uhm shell-init zsh)"
```

For Fish, add this to `~/.config/fish/config.fish`:

```fish
uhm shell-init fish | source
```

Restart the shell or source its startup file. To uninstall, remove that one line and restart. SSH and tmux need no separate hooks; each interactive shell loads its own normal startup configuration.

The generated function resolves a real executable from `PATH` once per invocation, then uses that same path for the job, validation, acknowledgement, and cleanup instead of recursively calling itself. Make sure the intended binary appears on `PATH` before installing the line. Regenerate the wrapper after upgrading `uhm` when the printed protocol version changes.

## What can persist

Version 1 accepts at most one of these typed actions:

- change directory;
- set one environment variable;
- unset one environment variable;
- source one file, including ordinary activation scripts.

Aliases, functions, arbitrary snippets, compound parent actions, `pushd`/`popd`, `umask`, `exit`, `exec`, and traps are not protocol actions. The client never parses model-written shell source into operands.

All persistent actions pass through the ordinary proposal preview and effect policy. Sourcing receives a literal additional warning because sourced code executes with full shell authority and may exit or replace the shell before acknowledgement or cleanup. The protocol is not a sandbox or safety guarantee.

## Private protocol and status

For one invocation, the wrapper asks the binary to create an owner-only directory beneath its validated data runtime root. The fixed request file contains protocol version, a 256-bit system-random nonce, shell family, parent cwd, previous status, and creation time. The child validates ownership, modes, link counts, ancestry, age, and nonce before atomically publishing one fixed-name typed response. Control data never uses application stdout or stderr.

The wrapper invokes a validator that reopens and validates the response and prints one audited shell-builtin template with quoted operands. Only that local template is evaluated. The wrapper then acknowledges `applied` or `failed`; without acknowledgement, local history and queued telemetry remain honestly `unknown`.

Status precedence:

- no parent response: preserve the child application status;
- failed child: apply nothing and preserve its failure;
- validation or application failure: return 15 and print a recovery instruction;
- applied action: return zero.

Cleanup is attempted on every ordinary path. No cleanup or acknowledgement promise is possible if sourced code terminates or replaces the shell.

## Invocation context and shell history

In `standard` mode, integrated invocations may add only protocol version, shell family, normalized parent cwd, and the immediately preceding exit status to the normal untrusted context payload; `full` may include the raw cwd. `minimal` continues to exclude these machine facts. Integration never uploads environment values.

One-entry native shell history is sensitive and off by default. To enable it:

```yaml
shell_context:
  last_history_entry: true
```

When enabled, the wrapper samples exactly one native history entry at invocation time. Before any request, `uhm` prints the exact sanitized entry and requires confirmation. Cancellation sends nothing. It never reads scrollback, tmux panes, or more than that one entry, and it does not install continuous history observation.
