<!-- diataxis: how-to -->

# Add parent-shell integration

A normal child process cannot persistently change the shell that launched it. Install UHM's optional wrapper when you want accepted typed `cd`, environment, or source actions to apply to the current Bash, Zsh, or Fish session.

## 1. Inspect the generated wrapper

```sh
uhm shell-init bash   # or zsh / fish
```

The output is static source. Review it before adding it to a startup file.

## 2. Install it

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

Restart the shell or source its startup file. SSH and tmux need no separate hook; each interactive shell loads its ordinary startup configuration.

### Zsh: wildcards in an intent

Without the wrapper, zsh stops a command that contains a wildcard word with no matching file, for example `need?`. The Zsh wrapper prevents this. It adds `alias uhm='noglob uhm'`, and then expands each word itself:

- A word that matches files expands to the file names, as usual. `*.md` becomes `README.md PRIVACY.md …`.
- A word that matches no file goes to UHM as you typed it. `need?` stays `need?`.

Both of these work without quotes:

```text
uhm What result do you need?
uhm summarize *.md
```

Quotes do not stop the expansion of a single word that matches files, because zsh removes the quotes before the wrapper gets the word. To send a pattern as text, quote the complete intent: `uhm 'summarize *.md'`. An unpaired apostrophe (`what's`) and `!` still need quotes, because zsh reads them before any alias applies.

## 3. Verify it

Request a harmless directory change:

```sh
uhm --review change to the parent directory
pwd
```

Without the wrapper, UHM returns status 11 with `requires_parent_shell=true` and shows a local fallback. With the wrapper, an accepted typed action is applied and acknowledged.

## 4. Enable one-entry history only if needed

```yaml
shell_context:
  last_history_entry: true
```

This is a sensitive opt-in. UHM previews the exact sanitized entry and requires confirmation before sending it.

## Update or uninstall

Regenerate the wrapper after upgrading UHM when the printed protocol version changes. To uninstall it, remove the one startup-file line and restart the shell.

For supported actions, protocol fields, status precedence, and context disclosure, see the [parent-shell integration reference](reference/shell-integration.md).
