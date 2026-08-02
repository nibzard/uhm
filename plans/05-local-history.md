# Plan 5 — Add inspectable local history

## Purpose and dependency

This post-release plan expands Plan 2's bounded metadata receipts into an optional, content-rich local record of what `uhm` understood, proposed, decided, executed, and observed. It depends on Plan 2's typed job state machine and receipt IDs; rollout begins only after Plan 3's public release has validated the core workflow.

The history is for the individual at the terminal. It is local, inspectable, bounded, and never treated as provider-side memory. This plan adds evidence for inspection, replay, feedback, and explicit repair. It does not add rollback, snapshots, parent-shell integration, or autonomous continuation.

## Full implementation description

### 1. Use an append-only journal with per-run artifacts

Store schema-versioned JSONL events and bounded artifacts under the resolved platform data directory:

```text
<data-dir>/uhm/
  history.v1.jsonl
  runs/
    <run-id>/
      manifest.json
      stdout.tail
      stderr.tail
      proposal.txt
      program.py
```

The artifact names are illustrative and created only when the selected detail level permits them. The journal is authoritative; a manifest indexes the artifacts associated with one run. Extend the existing `src/history.rs` and `src/dirs.rs` rather than creating a second receipt system.

Use private Unix permissions (`0700` directories and `0600` files), exclusive creation for run directories, an append lock for concurrent invocations, monotonically increasing per-run event sequences, and schema versions on both events and manifests. Flush a complete event before reporting that it was recorded. Ignore and report a truncated final JSONL line; earlier corruption must produce a diagnostic and a documented repair/export path rather than silent data loss. A missing or invalid platform data directory is an error, never a reason to fall back to the current directory.

JSONL is the initial store because this is a single-user, append-oriented CLI and its data should remain understandable with ordinary tools. Do not move to SQLite until measurements demonstrate material query latency, writer contention, migration risk, or relational requirements. Preserve a stable redacted JSON export if a later migration occurs.

### 2. Record a typed decision timeline

Represent a run as immutable events rather than repeatedly rewriting one row. Initial event kinds include:

```text
request_created
context_selected
proposal_received
clarification_requested
user_feedback_received
warning_shown
user_decision
execution_started
execution_finished
artifact_recorded
job_finished
```

Every event includes `schema_version`, `run_id`, a sequence, local timestamp, app/model/prompt versions, route, interaction mode, and optional `related_run_id`. Structured outcome fields distinguish proposal validity, whether execution occurred, child exit/signal, and explicit user feedback; exit code zero is not treated as proof that the user's job succeeded.

Content capture is controlled independently from telemetry:

- `metadata`: state transitions, versions, route/effect enums, timings, exit data, truncation flags, and hashes only.
- `diagnostic`: metadata plus the exact proposal and bounded error/result tails.
- `full`: diagnostic data plus original intent, clarification/revision text, selected context values, and complete local proposal/program manifests within configured size limits.

Add configuration such as `history.detail`, `history.capture_output`, `history.redact_paths`, `history.max_records`, `history.max_age`, and `history.max_bytes`. Do not silently broaden an upgraded installation from Plan 2's metadata default. A user must explicitly choose `diagnostic` or `full`, with a preview of what those levels retain. `uhm history status` reports the effective level, paths, current size, retention, and whether the most recent write succeeded.

Never copy the journal wholesale into a model request. Telemetry serializers must have no dependency on content-bearing history types; they may accept only an allowlisted coarse projection type with no run ID or content. History remains independently controllable when telemetry is enabled.

### 3. Make retention explicit and evidence-driven

Implement record-count, age, and total-byte limits, with artifact pruning tied to their owning run. Pruning happens in a bounded maintenance slice after a successful journal append or through an explicit command; it never requires a daemon and never makes ordinary startup scan the entire artifact tree.

Candidate dogfood defaults are 5,000 metadata events, 90 days for diagnostic tails, and 256 MiB for all content-bearing artifacts. These are hypotheses, not settled product constants. Benchmark them with real local usage, document the measurements, and tune them before making them release defaults. A limit of `none` must be explicit, and the status screen must make unbounded retention unmistakable.

`prune` and `clear` first resolve and validate exact descendants of the history root. Interactive use previews counts and bytes; noninteractive destructive use requires an explicit scope flag. Clearing history cannot target cache, configuration, working-directory files, or unrelated application data.

### 4. Add local inspection and lifecycle commands

Provide this initial command surface:

```text
uhm history list [--limit N] [--failed] [--route ROUTE]
uhm history show <run-id|last>
uhm history search -- <substring>
uhm history replay <run-id> --review
uhm history export [--redacted] [--output PATH]
uhm history prune [--dry-run]
uhm history clear [--before DATE|--all]
uhm history status
```

`list` and `show` reconstruct a human-readable job timeline from typed events. They describe proposals and observed outcomes, not hidden model reasoning. `search` is local substring search over fields permitted by the current detail level. It must not trigger an OpenAI request.

`replay` is available only when the configured detail level retained an exact proposal and enough manifest data. Otherwise it explains which evidence is missing. When available, it creates a new linked job, re-resolves current context, paths, effects, warnings, and execution policy, starts in review mode, and never blindly executes a stored command. `export` defaults to redaction and excludes captured output/program source unless the user explicitly includes those classes. It writes atomically and lists the included data classes before an interactive export.

Keep command parsing in `src/args.rs`, orchestration in `src/command.rs`, storage/query logic in focused history modules, and terminal presentation in `src/render.rs` without contaminating stdout intended for pipelines.

### 5. Treat repair as a new bounded job

Add:

```text
uhm repair <run-id|last> [feedback]
```

Repair is available only when the selected receipt level retained enough evidence. It builds the smallest explicit request from the prior intent, failed proposal, current machine facts, runtime version, exit/signal, and bounded sanitized diagnostic tail. Before sending, review mode shows which receipt fields will leave the device. The full journal and unrelated runs are never attached.

A repair creates a new run linked by `related_run_id`; it does not reopen or mutate the old run. It receives the same global job budget as any other invocation: at most two model calls and two executions, with one second-turn slot spent by clarification, model revision/repair, or a post-failure local replacement. Those paths are mutually exclusive. There is no compile-run-debug loop, automatic retry, or inherited budget from the earlier failed job.

The replacement proposal passes through current schema validation, effect detection, warnings, review/default-run policy, `--force`, execution, and history recording. A second execution failure ends the job with diagnostics and its new receipt ID.

### 6. Link explicit outcome feedback without collecting prose remotely

Store `uhm feedback good|bad [run-id]` as a typed event linked to the selected run. If telemetry is enabled, enqueue only the feedback enum and permitted coarse route/effect categories. Intent, command/program, path, output, error text, and free-form repair feedback remain local.

Feedback is an explicit user judgment and stays distinct from proposal validation and process execution outcome. This distinction should carry through local summaries and aggregate telemetry so maintainers do not label every zero exit status a successful job.

### 7. Add migration, integrity, and operability

Migrate Plan 2 metadata receipts without inventing absent content. Migrations write a new journal, validate event counts and checksums, then atomically swap it into place; retain a timestamped backup until a later successful invocation. Add `uhm history audit` only if it can reliably flag likely secret patterns before export without claiming complete secret detection.

Use checksums to detect accidental corruption, not to claim tamper-proof auditing. Document manual backup, export, pruning, and corruption recovery. Instrument local query and append timings as coarse telemetry only when telemetry is enabled; do not send record contents, exact record counts if uniquely identifying, or local paths.

Primary code areas are `src/history.rs`, `src/dirs.rs`, `src/args.rs`, `src/command.rs`, `src/config.rs`, `src/context.rs`, `src/render.rs`, and new narrowly scoped journal, query, migration, and retention modules.

## Expected outcomes

- A user can answer what `uhm` understood, proposed, warned about, ran, and observed from an inspectable private timeline.
- Users can choose how much sensitive local detail is retained and can see, export, prune, or clear it without affecting telemetry or configuration.
- Replay re-evaluates an old job under current conditions instead of treating stale text as executable truth.
- A failed run can seed one explicit, bounded repair job without opening an agent loop or sending unrelated history to OpenAI.
- Maintainers can distinguish proposal quality, execution outcome, and user-reported outcome without collecting terminal content.
- History remains responsive and understandable at the measured retention envelope.

## Definition of done

- All required event and manifest schemas are versioned, documented, round-trip tested, and reject unknown required semantics safely.
- Fixtures cover concurrent append, process interruption, truncated final line, earlier corruption, lock failure, disk-full behavior, migration, and private permission enforcement on Linux and macOS.
- `history list`, `show`, `search`, `replay`, `export`, `prune`, `clear`, and `status` work in TTY and non-TTY modes with stdout/stderr contracts suitable for piping.
- `show` accurately reconstructs initial proposal, clarification or revision, warning/force choice, execution, failure, and feedback timelines without claiming access to model reasoning.
- Upgrades preserve metadata-only capture until the user explicitly selects a richer level; disabling history creates no new journal events or artifacts and does not disable ordinary execution.
- Redacted exports contain none of the configured user, host, path, prompt, proposal, program, output, or diagnostic fields, and exports never include unrelated files.
- Retention tests enforce record, age, and byte limits without an unbounded startup scan; candidate defaults have a recorded dogfood benchmark and are labeled as hypotheses until validated.
- Clear/prune path tests reject symlinks, traversal, broad roots, configuration paths, cache paths, and unresolved targets.
- When sufficient detail exists, replay creates a linked new run in review mode and re-runs current validation, effect classification, and warnings; metadata-only receipts return a precise unavailable result without a model call or execution.
- `repair <id>` sends only the documented receipt subset, creates a new run, and cannot exceed two total model calls or two executions for that new job; clarification, model revision/repair, and post-failure local replacement compete for the one second-turn slot.
- Local and telemetry feedback schemas distinguish proposal outcome, execution outcome, and user feedback; prohibited history content cannot enter telemetry serialization.
- Benchmarks keep append, recent listing, ID lookup, and bounded search responsive at the validated retention envelope.

## Anti-goals

- Do not add undo, inverse generation, snapshots, filesystem restoration, or any recovery promise in this plan.
- Do not add parent-shell wrappers, `shell-init`, shell-history capture, pre-execution hooks, or terminal surveillance.
- Do not upload prompts, commands, programs, paths, output, errors, artifacts, or the local journal to telemetry or provider-side memory.
- Do not automatically use unrelated history as context for a new request or create a persistent model conversation.
- Do not blindly replay stored commands, automatically repair failures, or allow more than the global two-call job budget.
- Do not create accounts, cloud sync, collaboration, shared audit logs, or compliance-grade tamper evidence.
- Do not switch to SQLite without the documented performance or concurrency trigger.
- Do not present local timestamps, checksums, or stored output as proof that the user's intended outcome was achieved.
