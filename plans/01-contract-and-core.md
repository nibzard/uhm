# Plan 1 — Reset the contract and harden the core

Status: implemented and locally verified on 2026-08-01. Linux/macOS and musl checks are encoded in CI; the behavior contract is in [`docs/behavior-contract.md`](../docs/behavior-contract.md), and dependency/MSRV decisions are in [ADR 0001](../docs/architecture/0001-core-dependencies-and-msrv.md).

## Purpose and dependency

This plan turns the existing prototype into a truthful foundation for a result-first product. It intentionally makes breaking changes before there are public users. It must complete before the Responses execution loop is allowed to become the default path.

The governing contract is:

```text
intent → one typed proposal → local policy → execute or review → exact outcome
```

The model proposes an action. Local code owns parsing, presentation, execution, exit status, and recording. In default mode, invoking `uhm` grants the proposed local action execution authority; `--review` and `--dry-run` narrow that authority. The local effect detector only decides when to insert an extra advisory pause. It is not an authorization boundary or a safety claim, and a missed effect may therefore run without a warning. That is an explicit trust tradeoff, not an accidental guarantee.

## Full implementation description

### 1. Replace the accidental CLI grammar with an intentional one

Adopt a proven argument parser or implement an equivalently well-tested grammar. The current parser consumes recognized flags anywhere in natural-language input, has no reliable `--` boundary, and can accidentally turn a prompt containing `-y` into execution authority.

The new public grammar should be:

```text
uhm [global options] -- <intent>
uhm run [options] -- <intent>       # require an executable local action
uhm ask [options] -- <question>     # terminal/CLI answer only; never execute
uhm explain [options] -- <command>  # explain only; never execute
uhm history ...
uhm config ...
uhm context ...
uhm doctor
```

For convenience, `--` may be omitted after the first positional word; once prompt collection starts, every remaining token is opaque user text. Options that affect `uhm` must come before that boundary. Remove `-y`; authority must never have a short flag likely to occur in a dictated prompt.

Execution controls:

- Default: execute an ordinary proposed action and return its result.
- `--review`: always stop at the exact proposal before execution.
- `--dry-run`: print a machine-usable proposal and execute nothing.
- `--force`: skip advisory confirmation, including for detected consequential effects. This keeps the user in charge without pretending detection is complete.
- `--plain`: disable raw-mode editing, animation, Unicode ornament, dim styling, color, and terminal-control output.

Define exit-status precedence instead of pretending application and arbitrary child statuses can share one unambiguous 8-bit namespace:

- If a child action executes, return its exit status (or the conventional signal-derived status) unchanged.
- If no child executes, use documented application statuses for usage/configuration, API/structured-response failure, clarification required, and review required.
- In `--json`/automation mode, always emit a namespaced outcome such as `application_error`, `not_executed`, or `child_exit` plus the exact numeric status.

A proposal that was not executed must never exit as though the requested work completed.

### 2. Introduce a typed local action and policy model

Replace the current command envelope and numeric danger/confidence fields with local types that match actual product behavior:

```rust
enum ProposedAction {
    Answer(Answer),
    Shell(ShellAction),
    ParentShell(ParentShellAction),
    Clarification(Clarification),
    // Program is added by Plan 4.
}

struct ShellAction {
    command: String,
    summary: String,
    assumptions: Vec<String>,
    model_effects: Vec<Effect>,
    requirements: Vec<String>,
}

struct ParentShellAction {
    command: String,
    summary: String,
    assumptions: Vec<String>,
    model_effects: Vec<Effect>,
}

enum Effect {
    ReadLocal,
    WriteLocal,
    DeleteLocal,
    NetworkRead,
    RemoteMutation,
    PrivilegeElevation,
    ShellState,
    ProcessControl,
    Unknown,
}
```

The existing classifier becomes an advisory detector that adds `detected_effects` and concrete reasons. It must not emit “safe,” imply that its failure to match authorized execution, or present a single ordinal tier as complete truth. Default authority comes from the user's invocation, not from classifier approval. The display can say “no consequential effect detected,” which is bounded and honest.

Confirmation policy should be simple and user-overridable:

- Ordinary reads and narrowly scoped local commands execute by default.
- Detected deletion, broad writes, privilege elevation, remote mutation, process control, shell-state changes, or unknown compound effects show the exact action, affected targets when known, and a confirmation prompt.
- `--force` always permits progress after showing a concise warning on stderr.
- Editing invalidates the model summary, assumptions, and effect metadata. The edited action is locally redetected and labeled “edited locally.”

Do not keep the current model-supplied confidence percentage. It is not calibrated and does not control behavior.

### 3. Make the reviewed action byte-exact

Rendering may add ANSI spans but must never reconstruct the command. Highlight by byte ranges so that stripping styles returns the original string exactly, including repeated spaces, tabs, quoted whitespace, newlines, heredocs, and Unicode.

Enforce the stream contract:

- The performed action's stdout is the requested result and remains pipeable.
- Product status, warnings, progress, and hints go to stderr.
- Non-TTY output contains no ANSI, OSC, DECSET, cursor motion, spinner frames, or other control sequences unless the user explicitly opts in.
- Model-controlled content is sanitized before TTY rendering. Machine-output modes return exact bytes through a documented channel without also rendering them as terminal controls.
- `NO_COLOR`, `TERM=dumb`, redirected streams, SSH, and tmux capabilities are detected independently rather than treated as one boolean.

Add a cooked/plain path instead of forcing every user through the custom raw line editor. Fix fragmented UTF-8/CSI handling and display-cell width if the raw editor remains.

### 4. Replace silent configuration and path fallbacks

The current hand-rolled YAML/JSON behavior and silent defaulting are incompatible with an execution tool. Use maintained parsers for supported formats, validate every known field, reject or clearly warn on unknown fields, and report file path plus line/field context.

Add:

- `uhm config check` to parse and validate without making a model request.
- `uhm config show` to print every resolved value, its source, and sensitive-value redaction.
- A single platform path resolver used by runtime behavior and all help/error copy.
- Explicit failure when a safe config/data/cache directory cannot be resolved. Never fall back to `./uhm` or `.` for cache clearing, chmod, history, secrets, or receipts.
- Private directory/file creation (`0700`/`0600` on Unix) without recursively changing unrelated paths.

The project may add small, well-maintained Rust dependencies where they remove correctness-critical parsers or terminal approximations. “One direct dependency” is not a user benefit worth preserving at the cost of an ambiguous execution contract.

Before changing the core, record and lock an MSRV plus a dependency decision for: a real CLI grammar; Serde-based JSON and a maintained YAML parser; secure temporary files; cross-process file locking; Unix signal/process primitives; a cryptographic artifact hash; Unicode display width; and PTY test support. Prefer focused crates over a framework, but do not hand-roll these correctness boundaries. Verify licenses, advisories, binary impact, and Linux glibc/musl plus macOS support, update release CI, and delete the obsolete “single direct dependency” claim from `Cargo.toml` and the README.

### 5. Separate trusted instructions from untrusted data

Refactor prompt construction now, even before Plan 2 swaps API endpoints:

- Static application behavior belongs in developer/system instructions.
- Natural-language intent, piped input, filenames, Git metadata, directory entries, previous errors, and every other machine-derived value belong in user/input data.
- Never interpolate untrusted context into a developer/system string.
- Send the user's request once, not both embedded in instructions and repeated as input.
- Version the prompt/action schema in code and include that version in cache provenance.

### 6. Establish execution and test seams

Split the current large command path into testable services:

```text
cli → request builder → model client → proposal validator
    → advisory policy → reviewer → executor → receipt writer
```

Introduce traits or narrow interfaces for model transport, terminal interaction, context probes, clock, filesystem paths, and process execution. This enables deterministic unit, mock HTTP, PTY, and end-to-end tests without a live OpenAI call.

Cache keys must include model, endpoint/API family, prompt/schema version, relevant generation parameters, context-policy version, and request data. Cache model proposals only; never cache execution results.

## Expected outcomes

- Natural language can contain arbitrary flags without changing `uhm`'s mode or authority.
- The exact bytes the user reviews are the bytes the executor receives.
- The UI describes detected effects without claiming that an action is safe.
- Configuration errors stop execution with actionable diagnostics instead of silently changing model, endpoint, context, or autorun behavior.
- The codebase has explicit seams for the Responses API, result capture, receipts, telemetry, and microprograms.
- Ordinary output composes correctly with pipes, files, SSH, tmux, and assistive terminal modes.

## Definition of done

- A checked-in behavior table covers TTY/non-TTY × auto/run/ask/explain × default/review/dry-run/force.
- Exit-code tests prove executed actions preserve child status, non-executed application outcomes use the documented app namespace, and `--json` disambiguates both.
- `uhm ask -- what does -V mean`, `uhm explain -- git log -p`, and a prompt containing `-y`, `--help`, or `--system` reach the intended mode byte-for-byte.
- Property or corpus tests prove `strip_styles(highlight(command)) == command` for spaces, tabs, quotes, Unicode, pipes, chained commands, redirections, and heredocs.
- Edited commands never retain the original summary, assumptions, model effects, or confidence.
- Adversarial classifier fixtures include quoted/chained deletion, shell wrappers such as `bash -lc`, `env` prefixes, `dd`, `rsync --delete`, and remote mutations. False negatives may remain and may skip the advisory pause under the explicit default-trust policy, but none can silently become a claim of safety or classifier-granted authority.
- Non-TTY snapshots contain no terminal control bytes, and “not executed” paths return a distinct nonzero status.
- Invalid config, unknown fields, missing home/XDG directories, and unsafe relative fallbacks fail clearly and touch no unrelated path.
- Prompt request fixtures prove untrusted input appears only in the input/user role.
- Cache tests prove endpoint/API family, schema version, model parameters, and context mode affect the key.
- The selected dependency set and MSRV build on every declared Linux/macOS target, and repository documentation no longer claims a one-dependency posture.
- `cargo fmt --check`, Clippy with warnings denied, all unit/integration tests, and a release build pass on Linux and macOS CI.
- No known P0 issue from the prior architecture and UX audits remains in the affected paths.

## Anti-goals

- Do not migrate to the Responses API in this plan; Plan 2 owns that behavior change after the local contracts are stable.
- Do not add generated Python, JavaScript, or script files.
- Do not add open-ended chat, autonomous retries, repository exploration, planning, background work, or a plugin system.
- Do not attempt to parse every shell dialect into a perfect AST or make the advisory detector complete.
- Do not promise sandboxing, safety, transactional execution, or rollback.
- Do not preserve broken flags, config syntax, or output behavior for backward compatibility.
- Do not redesign the CLI for decoration. Personality work belongs in Plan 3 after exactness and accessibility are secured.

## Primary code areas

`src/args.rs`, `src/command.rs`, `src/safety.rs`, `src/config.rs`, `src/yaml.rs`, `src/json.rs`, `src/dirs.rs`, `src/prompt.rs`, `src/cache.rs`, `src/lineedit.rs`, `src/tty.rs`, and `src/render/*`.
