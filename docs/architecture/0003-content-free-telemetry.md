# ADR 0003: Content-free aggregate telemetry

- Status: accepted
- Date: 2026-08-01

## Context

The first public release needs enough evidence to tell whether people receive executable proposals, run them, hit process failures, or provide explicit feedback. Terminal content is unusually sensitive. Prompts, commands, paths, output, repositories, and errors would make richer analysis possible, but collecting them would violate the product's local-tool boundary.

A stable installation identifier would enable retention analysis. It would also create a durable identity surface that the product does not need for v0.1.

## Decision

The CLI sends at most one enum-only `interaction_summary` after a completed interaction. It contains coarse platform, mode, route, decision, effect, proposal, process outcome, parent-action acknowledgement, feedback, latency, cache, and interactivity categories. It has no exact client timestamp or stable identifier. `feedback_summary` uses the same categories and no join key.

The gateway is a Cloudflare Worker with exact versioned schemas, a 2 KiB body limit, coarse rate limiting, a kill switch, and disabled application logging. It accepts the released v1 event and the v2 event that adds only the parent-action enum. Accepted events go to Workers Analytics Engine dataset `uhm_cli_v1`. Aggregate queries use `SUM(_sample_interval)` and keep interaction and feedback counts separate.

Telemetry is on after a versioned first-use notice. The CLI honors a persistent command, configuration, invocation flag, `UHM_TELEMETRY=off`, and `DO_NOT_TRACK=1`. Delivery happens after result bytes with fixed 100 ms current-event and 200 ms old-queue budgets. The private queue is capped at 20 events and seven days. Ambiguous sends prefer loss over a retry.

## Consequences

Maintainers can compare coarse proposal and process outcomes without receiving terminal work. There is no DAU, retention, or user funnel analysis. The system tolerates loss and rare duplicates, so it is suitable for directional aggregates, not billing or correctness claims.

Cloudflare still processes network connection metadata and Analytics Engine records an ingestion timestamp. The product documentation says this directly. A future provider or schema change requires a new disclosure revision and a new review of the outbound-data boundary.
