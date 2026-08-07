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

The most sensitive areas are command/program classification, confirmation and auto-run
logic, terminal escape handling, program staging and temporary files, local secret
storage, API response parsing, the telemetry schema and queue, and the self-update
transport and release-signature verification
(see [docs/reference/release-signing.md](docs/reference/release-signing.md)).

`uhm` executes model-generated shell commands. Its warnings are advisory, not a
sandbox or safety boundary. A report that shows an unexpected command can run,
a control sequence can reach a terminal, a secret can leave the device, or an
opt-out can be bypassed is in scope. Model quality disputes without a boundary
failure are not security vulnerabilities.

`uhm` can also execute model-generated Python with the user's permissions. Its
isolated/no-site flags, cleared environment, time/output/resource limits, and
artifact staging are operational guardrails, not a sandbox. Python can still
read user-accessible files, reach the network, spawn or detach work, and cause
effects before termination. Reports about a concrete boundary bypass or secret
inheritance are in scope; reports premised on Python being a hostile-code sandbox
are not, because the product makes no such promise.

The telemetry gateway accepts only the enum-only schema documented in
[PRIVACY.md](PRIVACY.md). Do not put credentials, private prompts, commands,
outputs, paths, or identifying data in a vulnerability report.

## Disclosure

Please allow time for a fix and release before publishing exploit details. The
maintainers will credit reporters who want to be named.
