<!-- diataxis: explanation -->

# ADR 0002: Strict Responses actions and a bounded result loop

Status: superseded in part by [ADR 0005](../architecture/0005-provider-adapters-and-qualified-selection.md). The bounded result loop and canonical action requirements remain accepted; the OpenAI-only transport decision does not.

## Decision

uhm uses only OpenAI `POST /v1/responses`, with `store: false`, required tool choice, disabled parallel tool calls, and five strict local proposal tools: answer, child-shell action, typed parent-shell action, bounded Python program, and clarification. Parent-shell proposals contain operands rather than generated shell source. Developer instructions are static; intent, context, stdin metadata, feedback, and diagnostics stay in one untrusted JSON input. The client rejects incomplete responses, prose messages/refusals, unknown output items, non-strict resolved tools, and any result other than exactly one completed function call.

The job state is deliberately finite: one initial proposal and one optional user-triggered replacement, with no more than two executions. Clarification, revision, repair, and replacement editing compete for that one slot. Follow-ups are reconstructed stateless requests, not hosted conversations. Failures are never repaired automatically.

Context has explicit `minimal`, `standard`, and `full` policies. Standard is the default. The executor preserves stdin/output bytes, inherits terminal streams, tees redirected streams into bounded diagnostic tails, forwards termination to the child, imposes a wall timeout, and strips provider/private control secrets. This is operational hygiene, not containment.

Execution history is metadata-only JSONL with a dedicated lock, private permissions, atomic bounded rewrites, a 500-record/30-day default, and recovery from an interrupted final line. Content-rich rollback history is deferred.

## Alternatives considered

- Chat Completions or “compatible” provider URLs: rejected because they weaken the single tested contract and structured action guarantees.
- Free-form JSON or prose commands: rejected because executable jobs could be smuggled through an answer field.
- Automatic repair loops: rejected because they turn a command utility into an autonomous agent.
- Capturing terminal streams through a transparent PTY: deferred until evidence justifies its job-control and byte-transformation complexity.
- SQLite receipts: deferred because a bounded single-user append stream does not yet need query machinery.

## Consequences

The client is OpenAI-specific and rejects permissive output. Parent-shell effects are truthful and are applied only by the separately installed optional integration. Native TTY failures may lack captured diagnostics. The default model is `gpt-5.6-terra`, selected by the recorded Plan 2 release bakeoff in `docs/model-selection.md`.
