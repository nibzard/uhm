# Plan 2 — Build the result-first Responses loop

## Purpose and dependency

This plan implements the product's primary job: turn one natural-language intent into a useful local result with the least possible ceremony. It depends on Plan 1's exact rendering, explicit authority, validated configuration, and test seams.

The public state machine is intentionally small:

```text
request
  ├─ answer ───────────────────────────────→ print result
  ├─ clarification → user answer → action ─→ execute result
  └─ local action ─────────────────────────→ execute result
                               failure ─────→ optional one repair if the
                                             global second turn is unused
```

No path permits an unbounded model loop. Each job has at most two model calls and two executions. Clarification, model revision, or a post-execution replacement spends the same global second-turn slot. A local edit before the first execution does not call the model and remains part of the initial action; editing after a failure is a replacement and is unavailable if the slot was already spent.

## Full implementation description

### 1. Migrate to OpenAI's Responses API

Remove the generic “OpenAI-compatible” product surface and make the official OpenAI `POST /v1/responses` endpoint the only supported model API. Do not retain an arbitrary `base_url` escape hatch in the public configuration. Keep the transport small, but model the Responses API's typed output items rather than assuming the first output is text.

Choose the release default model through the checked-in result-first evaluation corpus: it must support Responses streaming and strict function tools, meet the agreed route/validity threshold, and minimize median/time-to-first-tool latency among qualifying OpenAI models. Pin that model ID per release, keep an explicit OpenAI model override for advanced users, and re-run the gate before changing the default rather than treating a model name as permanent product architecture.

Every request must set:

- `store: false`, so provider-side response retention is not the product's conversation/history mechanism.
- `parallel_tool_calls: false`, because one user intent may produce only one proposed action.
- `tool_choice: "required"`, because answers, actions, and clarifications all use the typed tool protocol and zero tool calls is not a valid product result.
- A fixed, code-versioned developer instruction string with no interpolated user or machine data.
- Explicit maximum output limits and a stable schema version.

Define four strict function tools for v0.1:

```text
return_answer(text)
run_shell(command, summary, assumptions, effects, requirements, stdin_mode)
require_parent_shell(command, summary, assumptions, effects)
request_clarification(question)
```

Use `strict: true`; every object has `additionalProperties: false`, all fields are required, and optional values use nullable types. `stdin_mode` is a client-controlled enum of `none` or `original`. Accept only after `response.completed`, allowing documented reasoning items plus exactly one completed function call. Reject plain-text action attempts, refusals as actions, unknown tools, zero/multiple calls, missing fields, oversized strings/arrays, or a tool schema marked non-strict in the returned metadata.

The client executes tools locally; the API never executes the command. `return_answer` is valid only when explanatory prose is itself the requested terminal/CLI result and no local action or local-data read is needed. It must not satisfy an executable job by merely describing a command. `run_shell` passes through requirement checks, advisory policy, and the executor. `require_parent_shell` returns an exact not-yet-applied action through the parent-state fallback below. `request_clarification` may be used only once before the model must return a final answer or action.

Implement semantic streaming events rather than reusing the Chat Completions SSE assumptions. Enforce byte/item limits while deltas arrive. Handle at minimum `response.created`, function-argument deltas/done, output-item done, `response.completed`, `response.incomplete`, `response.failed`, refusal content, `error`, and early EOF. A complete non-streamed path remains available for tests and plain mode.

Relevant source contracts: [Responses migration](https://developers.openai.com/api/docs/guides/migrate-to-responses), [function calling and strict mode](https://developers.openai.com/api/docs/guides/function-calling#strict-mode), [streaming Responses](https://developers.openai.com/api/docs/guides/streaming-responses), and [conversation state](https://developers.openai.com/api/docs/guides/conversation-state).

### 2. Make context useful, bounded, and visible

Resolve the contradictory “prompt only” versus “send everything” ideas with three explicit modes:

| Mode | Data sent |
| --- | --- |
| `minimal` | The user's intent and any stdin they explicitly piped to `uhm`; no automatic machine metadata. |
| `standard` (default) | Minimal plus OS family/version, architecture, target shell, presence booleans for a versioned catalog of common CLIs, working directory normalized to `$HOME/...` (or a non-identifying basename outside home), bounded Git branch/dirty summary, and at most 40 immediate entry names under a shared byte limit. |
| `full` | Standard plus raw cwd, username, hostname, kernel, bounded installed-tool/version inventory, and shell-integration error context when available. Still never includes environment values, API keys, secret files, file contents, shell history, Git remotes, or repository diffs automatically. |

The common-tool catalog should be small and product-versioned—for example core POSIX tools plus `git`, `rg`, `fd`, `jq`, `yq`, `fzf`, `gh`, `python3`, package managers, and common container/cloud CLIs. `standard` sends only catalog name → present boolean, never discovered paths, versions, aliases, or the full `PATH`; `full` may add bounded versions. Resolve it locally from the actual execution environment under the shared context deadline.

Expose a versioned disclosure payload that says `standard` context leaves the device, names the field groups, and points to `uhm context show`, `--context minimal`, and the config key. The request builder requires an injected rendered-disclosure marker before the first outbound request. Plan 2 tests that gate through a seam; Plan 3 owns the user-visible combined notice and its end-to-end persistence, so Plan 2 must not create a competing second notice.

`uhm context show [mode]` must render the exact structured context that the next request would send, with the prompt represented by a placeholder. Context probes run concurrently under one aggregate deadline, start only after local aliases and purely local paths are resolved, and degrade field-by-field when a probe fails. Never allow a context timeout to become several seconds of silent startup.

The required `run_shell.requirements` list names the executables the proposal expects. Resolve those names again immediately before execution. Missing requirements produce an actionable unavailable result and may consume the one user-triggered revision slot if the user requests an alternative; they are never installed automatically. This validation improves availability but is not a complete parser or containment boundary—compound shell text may invoke undeclared tools, so the model instruction and eval corpus must penalize missing declarations.

Do not send document contents merely because a filename appears in a request. In Plan 4, generated programs operate on those files locally; the model usually needs only the requested operation, path metadata, and available runtime inventory.

### 3. Implement result-first execution

`uhm <intent>` should normally finish by returning the action's output, not by leaving the user at a command card.

Execution requirements:

- Spawn the user's configured shell deliberately and record which shell/dialect is used.
- Preserve piped stdin in a bounded byte spool before request construction; never trim, normalize, or round-trip it through UTF-8. Valid UTF-8 may be included in model input under the selected context policy. For non-UTF-8 input, send only `present`, byte count, and `utf8=false`; replay the exact bytes only when the proposal declares `stdin_mode=original`.
- When a child stream is attached to a terminal, inherit the foreground terminal so buffering, color, interaction, and job control remain native; no diagnostic tail is promised for that stream. When redirected, tee stdout/stderr as bytes concurrently to their destinations and bounded diagnostic rings. Do not claim a total ordering between the two streams.
- Preserve the child exit status, including signal termination, as `uhm`'s outcome status.
- Product progress and policy copy remain on stderr; child stdout remains the pipeable result.
- Cap captured stdout/stderr, request/stdin size, response size, and execution wall time through documented configuration. Truncation affects the retained diagnostic copy, not the live child stream.
- Sanitize captured diagnostics before showing them inside `uhm` UI; never reinterpret command output as a control protocol.
- Start child commands with a deliberate environment that removes `OPENAI_API_KEY`, any separately stored `uhm` provider secret, and private `uhm` control variables. Preserve the user's ordinary operational environment by default so tools such as Git and cloud CLIs still work, without ever sending those values to the model; add configurable execution-only deny entries for users who want a narrower child environment. This reduces accidental provider-key forwarding, but it is not a sandbox and does not stop a command from reading user-accessible files.
- Put the child in a foreground process group, forward SIGINT/SIGTERM, escalate cancellation after a documented grace period, map signal termination to `128 + signal` while recording the signal separately, and treat downstream SIGPIPE/BrokenPipe as normal pipeline termination rather than UI failure.

This is the honest v0.1 tradeoff for error-aware repair: redirected errors can be captured automatically, while a native TTY stream cannot. When diagnostics were inherited, the repair UI says they are unavailable and lets the user supply a short observation. Evaluate a transparent PTY proxy later only if failed-job evidence justifies its job-control and byte-transformation complexity.

`--review` shows the byte-exact action, summary, assumptions, model-declared effects, locally detected effects, shell, and cwd. The available controls are run, revise, edit, copy, and cancel. Any revised or edited action returns to the beginning of local policy evaluation.

`--dry-run` returns a stable structured envelope when stdout is not a TTY and a concise exact proposal on a TTY. It must never execute. Answers in dry-run mode are still answers; forced `run` mode exits nonzero if the model refuses or returns only prose.

Treat voice dictation as ordinary natural-language input rather than a separate product surface. Prompts must not require shell-style quoting once the prompt boundary begins, and evaluation fixtures should include dictated punctuation, filler words, casing, and common command-name homophones. Do not silently “correct” paths, flags, or quoted literals during normalization.

### 4. Add one clarification or one repair—not a conversation

Support two mutually exclusive uses of the single second turn:

1. **Before execution:** the model calls `request_clarification`. Read one answer and make the one allowed follow-up Responses request containing the original intent, structured context, and that answer. No later model revision or repair is permitted in this job.
2. **After proposal or failure:** if no clarification/revision has used the budget, the user selects model revision/repair and optionally supplies short feedback, or edits a failed action locally. A model follow-up receives the original intent, exact prior action, feedback, and—after a failed execution—the exit status plus available bounded sanitized stderr, and must return one replacement answer or action. A post-failure local edit consumes the same replacement slot even though it makes no API call.

Every follow-up is a fresh stateless Responses request reconstructed from the bounded inputs above. Do not send `previous_response_id`, rely on provider storage, or claim to preserve hidden reasoning. Do not repair automatically. Capturing an error and offering `repair?` is feasible; sending it back to the model without user action would turn a command failure into an autonomous loop. The replacement action follows the same warning/review policy; detected consequential effects are never hidden because it is a retry. Cross-invocation receipt-driven repair belongs to Plan 5 and starts a new explicit job with its own budget.

### 5. Handle parent-shell actions truthfully

Instruct the model to use `require_parent_shell` whenever the requested outcome is a persistent `cd`, `pushd/popd`, export/assignment, unset, source/activation, alias/function definition, `umask`, or similar shell-state effect. Independently detect common recognized forms if the model incorrectly returns `run_shell`, and convert them to the not-applied path rather than executing them in a child.

Until the optional post-release integration in Plan 6 is installed:

- Do not execute these in a child shell and report success; the effect would disappear when `uhm` exits.
- Show the exact action with a one-line explanation and an instruction to run/copy it in the current shell.
- Return a distinct outcome indicating that the action was generated but not applied.

This answers the earlier question: filesystem/network effects survive a child process, while the parent shell's cwd, variables, aliases, functions, and activation state do not. Shell syntax is too broad for a complete local detector, so the guarantee is deliberately scoped: model-declared and locally recognized parent-state actions are never reported as persisted. Unknown or obfuscated syntax may evade the advisory detector under the product's default-trust policy; do not claim otherwise.

### 6. Create a minimal local execution receipt

Keep clarification/revision/repair state in memory for the current job. For public v0.1, persist only a bounded metadata receipt:

- Schema version, opaque run ID, local timestamp, app major/minor version, mode, context mode, route, and prompt-schema version.
- Coarse declared/detected effect categories and user decision.
- Execution-attempted boolean, exit/signal category, latency bucket, cache state, and whether the second turn was used.

Do not persist the intent, command/program, cwd, context values, clarification/feedback text, answers, stdout, stderr, or diagnostic strings in v0.1 metadata history.

Store append-only JSONL under the platform data directory with a private parent directory and file permissions. Use a dedicated lock file: serialize one complete line, acquire the exclusive lock, append and sync that line, then release it. Append, retention pruning, clear, and later migration must share the same lock; readers take a locked snapshot and tolerate only an interrupted final line. Receipt write failure warns on stderr but cannot rewrite an already-returned child status. Enable metadata receipts by default; bound them to 500 records or 30 days, provide `uhm history status|clear` in v0.1, and allow `history.enabled: false`. Never serialize a record, run ID, timestamp, or content-bearing field into a model request or telemetry. Plan 3 may independently build an allowlisted enum-only outcome projection from the in-memory job state—or, for explicit later feedback, from the latest metadata receipt—without uploading the receipt itself.

Use JSONL now because this is a single-user append-only event stream that should remain inspectable and recoverable after a partial final line. Defer SQLite until concurrency or query volume demonstrates a need.

### 7. Meet a terminal-speed budget

Measure the pipeline in separate spans: argument/config load, local alias, context, API connect, first tool delta, proposal validation, policy, process start, execution, and telemetry handoff.

Initial budgets:

- Local alias/result path: p95 under 25 ms on a warm machine.
- Non-network local overhead before the API: p95 under 100 ms; context has one shared 150 ms budget.
- Visible progress begins within 100 ms when an API call is needed.
- Telemetry begins only after result delivery and is measured separately under Plan 3's hard return-to-shell budget.

Do not promise an end-to-end model latency that the application cannot control. Report model/API latency separately from local overhead.

Create one reusable HTTP agent rather than rebuilding it per request. Proposal cache entries must obey Plan 1 provenance rules and a short configurable TTL; a cache hit still runs current local policy and never reuses an execution result.

## Expected outcomes

- A typical invocation produces the requested local result in one step.
- Users can force review/dry-run without maintaining a separate product mode.
- The model's output is schema-constrained and cannot silently smuggle action text through a prose field.
- Useful terminal context improves action selection, while users can inspect or minimize exactly what leaves the machine.
- A wrong proposal or failed command can be corrected once without launching a coding agent or starting an open-ended chat.
- Piped results, child exit codes, and parent-shell limitations behave predictably.

## Definition of done

- Mock API fixtures cover every accepted/rejected Responses output shape, strict-schema failure, multiple tool calls, refusal, incomplete response, stream interruption, timeout, and oversized arguments.
- All requests use `/v1/responses`, `store: false`, strict tools, `tool_choice: "required"`, and `parallel_tool_calls: false`; no Chat Completions endpoint, provider conversation ID, or “compatible provider” path remains.
- The release pins one eval-qualified default OpenAI model; the benchmark records structured-action validity, route correctness, correction rate, time to first tool output, and total latency for every candidate and rejects models without strict-tool support.
- Developer instructions are byte-stable and contain no user request, stdin, path, Git data, filenames, error text, or other untrusted context.
- Context snapshot tests prove each mode includes exactly its documented fields and never includes environment values or secrets.
- Standard-context tests prove the versioned common-tool map contains only names and presence booleans, obeys the shared deadline, and reflects the execution `PATH`; requirement tests catch declared unavailable tools without installing anything or claiming complete shell analysis.
- The versioned context disclosure payload and injected marker interface are complete; Plan 2 tests both blocked and pre-rendered paths without depending on Plan 3 UI. Plan 3 owns the end-to-end proof that a real first request follows the displayed notice.
- End-to-end PTY tests cover successful run, answer, clarification, review/edit/run, failure/repair/run, cancel, and `--force` on a detected consequential action.
- A voice-dictation prompt corpus reaches the request builder without option collisions or path/flag rewriting and produces the same routes as equivalent typed intents.
- Non-TTY tests prove stdout contains only the action result or requested dry-run envelope and the process returns the child outcome status.
- Byte-level I/O tests cover exact piped stdin replay, non-UTF-8 stdin metadata, independent redirected streams, inherited TTY behavior, signal forwarding/escalation, SIGPIPE, and removal of secrets from the child environment.
- Transition tests enforce at most two model calls, two model proposals, and—only when the second action is a user-triggered post-failure replacement—two executions. Clarification, model revision, model repair, and post-failure local edit are mutually exclusive second-turn consumers; every over-budget transition makes no API call or execution.
- Typed `require_parent_shell` fixtures plus recognized `cd`, export, source, and activation forms are never falsely reported as persisted without shell integration; adversarial unknown syntax is documented as outside the detector's completeness claim.
- Concurrent-process tests prove metadata receipt append/prune/clear use one lock, files are private and bounded, an interrupted final write is recoverable, prohibited content is absent, and history can be disabled without affecting execution.
- A result-first acceptance corpus covers common read/search/inspect jobs, one bounded write, one answer, piped context, a consequential action, an ambiguity, and a correctable failure. Unambiguous ordinary jobs complete on the initial invocation without a review card or confirmation.
- Routing fixtures prove executable/local-data jobs cannot end as prose that merely recommends a command; `return_answer` is limited to requested terminal/CLI explanation or a genuinely prose-valued result.
- TTY and supported non-TTY tests prove ordinary actions run directly, only detected consequential actions pause, `--review` always pauses, `--dry-run` never executes, and `--force` warns without prompting.
- Local overhead and context deadlines are measured in CI benchmarks or a repeatable benchmark command; regressions beyond the agreed budget fail a release gate.
- Under one documented benchmark network setup, the model bakeoff records p50/p95 time to first complete valid proposal alongside corpus task success and correction rate. The default is the fastest model that clears the quality threshold; these comparative measurements are not advertised as a universal network SLO.
- The existing unit suite plus mock HTTP, process, and PTY suites pass on Linux and macOS.

## Anti-goals

- Do not add OpenAI Conversations, provider-side durable threads, or cross-job chat memory.
- Do not let the Responses API call hosted web search, file search, code interpreter, MCP, computer use, or any tool other than the four local proposal functions.
- Do not automatically retry a failed command, explore the filesystem, install missing tools, or keep repairing until success.
- Do not generate standalone Python/JavaScript program artifacts yet; Plan 4 owns that scope.
- Do not attempt to persist parent-shell state from the unintegrated base binary.
- Do not add speech recognition, microphone capture, or an audio dependency; operating-system dictation already supplies text to the same input path.
- Do not implement telemetry, personality, or package distribution in Plan 2; they remain mandatory Plan 3 gates for public v0.1. Content-rich history remains Plan 5 scope.
- Do not treat exit code zero as proof that the user's intent was satisfied. It is an outcome signal, not ground truth.

## Primary code areas

`src/api.rs`, `src/http.rs`, `src/sse.rs`, `src/prompt.rs`, `src/command.rs`, `src/context.rs`, `src/shell.rs`, `src/cache.rs`, `src/history.rs`, `src/main.rs`, and new typed request/action/executor modules extracted from them.
