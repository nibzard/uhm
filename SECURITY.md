# Security policy

## Reporting a vulnerability

Please do not open a public issue for a vulnerability that could expose API
keys, alter terminal output, bypass command confirmation, overwrite files, or
execute a command unexpectedly.

Use the repository's private vulnerability reporting feature. If private
reporting is unavailable, open a minimal issue asking for a private contact
channel without including exploit details.

Include what you can safely provide:

- the affected version or commit;
- the operating system and shell;
- a minimal reproduction;
- the impact you observed;
- any suggested mitigation.

Do not include real credentials or private user data. Use temporary files and
harmless commands in reproductions.

## Scope

The latest release and the current default branch receive security fixes.
Older releases may be asked to upgrade rather than receive a backport.

The most sensitive areas are command classification, confirmation and auto-run
logic, terminal escape handling, temporary files, local secret storage, and API
response parsing.

## Disclosure

Please allow time for a fix and release before publishing exploit details. The
maintainers will credit reporters who want to be named.
