# Plan 4 — Add bounded just-in-time microprograms

## Purpose and dependency

This plan adds the product's most distinctive post-release capability: when a normal command or compound pipeline is a poor fit, `uhm` may write one small Python 3 program, execute it locally, and return its result. It starts only after public v0.1 feedback and task reports show where command generation fails or becomes too contorted.

This is not a miniature coding agent. The invariant is:

```text
initial turn:          one intent → one program → at most one execution → result
optional second turn: user-triggered complete replacement → at most one execution → stop
```

The same global second-turn budget from Plan 2 applies: clarification, model revision/repair, or a post-failure local replacement may consume it once. A job has at most two model calls, two action candidates, and two executions. There is no repository exploration, dependency installation, test/debug cycle, program project, or autonomous repair.

## Full implementation description

### 1. Validate the jobs before adding a runtime

Create a representative evaluation corpus from real or deliberately authored tasks:

- Count paragraphs, words, repeated terms, headings, links, or line distributions in a large document.
- Extract text between structural markers.
- Concatenate, split, rename, or summarize metadata across multiple files.
- Transform JSON, JSONL, CSV, TSV, and simple logs.
- Group, sort, filter, deduplicate, and compute descriptive statistics.
- Produce one derived artifact while leaving inputs unchanged.

For each task, record whether the best solution is an existing command, a compound shell pipeline, a generated microprogram, a clarification, or an answer. The program route should earn its complexity: ship only if it materially improves first-attempt success or comprehensibility for jobs that users actually attempt.

Do not use raw prompts, commands, paths, or outputs from default telemetry to build this corpus. Collect examples through explicit issue templates, opt-in feedback, and maintainer-authored fixtures.

### 2. Add one strict program proposal tool

Extend the Responses tool set with one strict function:

```text
run_program(
  runtime,
  source,
  summary,
  assumptions,
  inputs,
  outputs,
  effects,
  result_mode
)
```

All fields are required, unknown fields are rejected, sizes and list counts are bounded, and `runtime` initially accepts only the literal versioned enum `python3`. A response may propose `run_shell` or `run_program`, never both and never one nested inside the other.

The manifest must identify:

- Every explicit input path and whether it is read-only or intended for replacement.
- Every expected output path or `stdout` as the only result.
- Declared network, process, write, delete, and privilege effects.
- Runtime assumptions, such as UTF-8 input, CSV headers, or a particular delimiter.
- A short outcome-oriented summary. Do not ask for a model confidence score.

Before the request, collect only the resolved `python3` path/version and whether isolated/no-site mode works. A missing or unsupported Python runtime produces an actionable unavailable result or falls back to a normal command when that genuinely fits; `uhm` never installs a language or package automatically.

### 3. Define an intentional action router

The model chooses the implementation category under these client-enforced guidelines:

1. Prefer `run_shell` when one existing CLI or a short compound pipeline is clear, available on the detected host, and easier to inspect.
2. Choose `run_program(runtime=python3)` for nontrivial text/data processing, standard-library structured formats, statistics, and multifile logic.
3. Return an answer when no local execution is needed.
4. Ask one clarification rather than guessing an input, output, encoding, delimiter, overwrite policy, or ambiguous scope.

Add eval assertions around this router. Route selection should be measured by task success, startup cost, portability, and understandable failure—not aesthetic preference. Bash remains the implementation language of the existing shell route; it is not a second standalone program runtime. JavaScript or another runtime requires later evidence and its own compatibility plan.

### 4. Build a local executor with honest boundaries

The v1 microprogram executor is a constrained process harness, not a security sandbox. That distinction must appear in review copy and documentation. The user has chosen to trust the model, while `uhm` provides practical containment and transparency.

For every execution:

- Create a unique private temporary directory (`0700`) and source file (`0600`).
- Write source directly from the validated model field; never interpolate it into a shell command.
- Spawn the resolved Python interpreter directly with the fixed argument vector `python3 -I -S <source>`. No third-party package is provided or assumed. These flags reduce automatic environment/site imports; they do not prevent generated code from changing `sys.path`, importing user-accessible files, or reaching the host. If a task cannot work with the standard library under this mode, route it elsewhere or report the limitation rather than weakening the default silently.
- Set the working directory deliberately and pass input/output paths as arguments or dedicated environment values; never concatenate user paths into source at execution time.
- Start from an environment allowlist. Strip `OPENAI_API_KEY`, other token/key-looking variables, cloud credentials, SSH/GPG agent sockets, telemetry configuration, and application secrets. Preserve only the locale, a minimal executable path, and declared values. This prevents accidental inheritance; it does not prevent Python from reading credentials or files otherwise accessible to the user.
- Close inherited file descriptors, create a new process group, and make a best effort to terminate that group on timeout or cancellation. Do not claim hostile code cannot escape, detach, or create side effects before termination.
- Apply source/input/output byte caps and a hard wall timeout portably. Apply CPU, address-space, open-file, and process limits only where the host primitives make them meaningful, record which controls were applied, and treat them as operational guardrails rather than a portable containment boundary. A measured temporary-workspace cap is checked before and after execution; it is not advertised as a filesystem quota.
- Enforce a hard total stdout/stderr byte limit in addition to smaller retained diagnostic tails. On overflow, stop the process group, mark the result truncated, and return a distinct failure.
- Remove the temporary directory on success, failure, timeout, signal, or user cancellation unless a debug-retain flag was explicitly selected.

Initial defaults to benchmark and tune:

| Resource | Default |
| --- | ---: |
| Source | 64 KiB |
| Explicit piped input | 16 MiB |
| Declared input paths | 64 |
| Measured temporary workspace | 64 MiB soft cap |
| Wall time | 10 seconds |
| CPU time | 5 seconds |
| Address space | 256 MiB where supported and compatible |
| Child processes | Best-effort host limit; no portable hard guarantee |
| Total stdout + stderr | 16 MiB hard cap |
| Retained diagnostic tail | 1 MiB per stream |

These limits are initial hypotheses to benchmark and tune, not safety guarantees. Local Python can access host files, processes, and networks available to the user unless the OS independently prevents it. `uhm` must say this plainly.

### 5. Make local data processing the privacy advantage

For file-based tasks, the LLM usually needs the requested transformation and bounded metadata—not the document body. Send file path/type/size metadata only when required by the selected context policy. The generated program reads and processes the actual data locally.

Explicitly piped stdin is user-supplied model input under the current product contract, but add `--local-input` so a user can make large stdin available only to the generated program while the model receives metadata such as byte count and an optional user-declared format.

Do not silently sample file contents to improve code generation. If a schema/header/sample is genuinely necessary, ask once and show exactly what excerpt would leave the device.

### 6. Preserve the result-first experience

For a read-only program whose result is stdout, ordinary mode runs it and returns stdout just like a normal command. `--review` shows:

- Runtime and fixed interpreter command.
- Exact source.
- Declared input/output paths.
- Assumptions and declared/detected effects.
- Resource limits and the explicit “local process, not sandboxed” boundary.

Detected writes, deletes, network access, child-process use, privilege elevation, undeclared path references, or effect/source disagreement trigger the consequential-action warning. `--force` remains available; warnings never permanently block the user.

For generated artifacts, prefer a cooperative two-stage contract:

1. For each regular-file destination, `uhm` supplies a private sibling staging path on the same filesystem and asks the program to write only there.
2. After successful execution, `uhm` verifies declared paths and size, rejects symlinks and unsupported destination types, shows consequential overwrites, fsyncs the staged regular file, and renames each path into place independently.

This makes each supported file replacement atomic relative to the observed destination filesystem; it does not make a multifile job transactional, stop concurrent writers, or constrain a program that ignores the supplied path. If the program fails, `uhm` commits no staged artifact, but unmanaged effects may already have happened. In-place mutation is allowed only when the user explicitly requests it and receives the corresponding warning. Plan 7, not this plan, adds retained preimages and verified restoration.

The final result is stdout or a concise list of committed artifacts. Source and runtime details stay on stderr or behind review/history commands so pipes remain useful.

### 7. Permit one user-triggered program correction within the global budget

On compile/runtime failure, offer one repair only if clarification or revision has not already spent the global second turn and the job has not reached its two-execution limit. The follow-up request contains the original intent, previous manifest/source, runtime version, and available sanitized diagnostic tail. It returns a complete replacement program; patches are not applied to partially trusted source.

The user triggers this request. There is no automatic compile-run-debug loop. The replacement is revalidated, reclassified, and handled under the normal default-run/review policy, and it may execute once. A second failure—or any failure after the turn was already spent—ends the interaction with diagnostics and the local receipt ID. A local edit before first execution remains part of the initial action; a post-failure local edit consumes the same replacement slot without making a model call and never raises the two-execution ceiling.

### 8. Track Monty separately; do not gate this feature on it

Cloudflare Code Mode demonstrates the valuable shape—code as a compact plan, a narrow capability surface, credentials held by the host, controlled egress, and only final results returned—but its Dynamic Workers are a hosted Cloudflare facility designed around large MCP/API catalogs, not a local CLI runtime. Do not add that platform dependency. See [Cloudflare Code Mode](https://blog.cloudflare.com/code-mode/) and [server-side Code Mode](https://blog.cloudflare.com/code-mode-mcp/).

Pydantic Monty is more directly relevant: it is a Rust Python-subset interpreter with deny-by-default filesystem/environment/network access, resource tracking, captured output, and host-supplied functions. However, its repository currently labels it experimental and “not ready for prime time,” its language/stdlib is partial, and its runtime is changing rapidly. See [Pydantic Monty](https://github.com/pydantic/monty).

A separate, non-gating research issue may be opened after the normal microprogram corpus exists. If undertaken, it should:

- Test corpus compatibility against a pinned Monty release.
- Use the crash-isolated worker/runtime subprocess, never link an untrusted program into the main `uhm` process.
- Start a fresh worker per program with no state reuse, network, environment, shell, LLM callback, or ambient filesystem.
- Expose only explicit read-only/copy-on-write mounts and pure host functions.
- Compare success, binary size, compile time, startup, diagnostics, and repair rate with local Python.

Monty becomes an optional runtime only when its project no longer disclaims production readiness, its pinned API passes audit/fuzzing and the task corpus, and the dependency/toolchain cost is acceptable. The spike and Monty adoption are not part of this plan's definition of done, and its presence must never be required for the ordinary command path.

Do not build a custom data-transformation DSL preemptively. If post-release tasks cluster around a small pure operation set, a versioned IR can later provide a more controllable fast path, but it should be justified by evidence rather than used to avoid the product's explicit small-program goal.

## Expected outcomes

- Users can solve bounded document, data, and multifile tasks that are awkward or unreliable as one remembered command.
- Large local inputs normally remain on the device; the model writes the operation while the machine processes the data.
- The model selects among an existing shell command/pipeline, one Python 3 microprogram, an answer, or a clarification using current runtime context.
- Generated code remains a one-shot artifact with explicit inputs, outputs, effects, limits, and one optional correction.
- The feature is powerful without pretending that ordinary host interpreters are sandboxed or drifting into project-scale coding-agent behavior.

## Definition of done

- A versioned eval corpus contains at least 50 representative tasks, expected route, fixtures, expected result, allowed effects, and failure cases.
- The strict `run_program` schema rejects extra fields, a runtime other than `python3`, unavailable Python, oversized source/manifests, undeclared outputs, multiple actions, and malformed paths.
- The router selects an existing command or compound pipeline for simple native jobs and Python for the core structured/text corpus; it never invents a Bash-program or JavaScript-program route.
- Program source is never shell-interpolated; interpreter and arguments are independently asserted in process-spawn tests.
- Child programs do not inherit the OpenAI key or other stripped test credentials in their environment; documentation and review copy state that user-readable secret files remain accessible.
- Hard byte/wall limits and best-effort host-specific memory/process controls, cancellation, interpreter crash, signal, output overflow, and cleanup paths are covered on Linux and macOS; UI and receipts distinguish applied from unavailable controls.
- Read-only examples for paragraph count, repeated word count, text statistics, extraction, concatenation, JSONL filtering, CSV aggregation, and deduplication return correct result stdout.
- Artifact examples use same-filesystem sibling staging, validate supported regular-file destinations, and commit each file independently; on failure `uhm` commits no staged output and reports that unmanaged side effects cannot be ruled out.
- Source/effect disagreement produces a warning; `--force` can still proceed.
- `--local-input` integration tests prove large stdin is not placed in the OpenAI request.
- Transition tests enforce the shared limit of two model calls, two action candidates, and two executions. Clarification, model revision/repair, and post-failure local replacement are mutually exclusive second-turn uses; every over-budget path makes no API call or execution.
- v0.1-style metadata receipts add only the coarse program route/runtime/outcome. When Plan 5 detailed history is enabled, it may additionally retain exact source, runtime version, manifest, effects, limits, exit, and artifact hashes locally; telemetry receives only coarse route/outcome enums.

## Anti-goals

- Do not create or edit a software project, inspect a repository recursively, write tests, choose dependencies, or maintain multiple source files.
- Do not install Python, packages, system tools, or libraries on the user's behalf.
- Do not automatically repair, retry, plan, call the LLM from generated code, or keep a persistent interpreter/REPL.
- Do not intentionally schedule or detach programs, add background-job UX, or claim the process harness can prevent hostile code from escaping it.
- Do not claim a local Python process is sandboxed, safe, hermetic, unable to access the host, unable to detach, or fully constrained by portable resource limits.
- Do not send local file contents to the model without an explicit user action and preview.
- Do not intentionally pass the OpenAI API key, telemetry endpoint internals, cloud credentials, SSH/GPG agents, or full environment to generated code.
- Do not adopt Cloudflare Dynamic Workers as a required local runtime or link experimental Monty in-process.
- Do not add standalone Bash or JavaScript program modes in this phase; ordinary compound shell actions remain supported through `run_shell`.
- Do not make a custom language or general code-execution framework the product. The feature exists only to finish bounded user jobs.

## Primary code areas

New program proposal, runtime inventory, process-limit, staging, and artifact modules; extensions to `src/api.rs`, `src/prompt.rs`, `src/command.rs`, `src/context.rs`, `src/shell.rs`, `src/history.rs`, `src/render/*`, configuration/docs, and the mock Responses/eval harness.
