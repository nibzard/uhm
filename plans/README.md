# `uhm` implementation roadmap

This roadmap turns the product research and code audit into a phased implementation plan set. Plans 1–3 are sequential and produce the first public release; Plan 8 is a lightweight, independent demo asset that can ship at any time and feeds Plan 3's README rewrite. After v0.1, Plans 4–6 are evidence-driven tracks that may be prioritized independently; Plan 7 depends on Plans 4 and 5. Plan 9 is the initial model-quality infrastructure. Plan 10 is the corrective v0.2.2 hardening track and takes release priority over further capability work. The first full benchmark then drives three coordinated tracks: make the benchmark decision-grade in Plan 11, simplify the program contract in Plan 12, and add provider adapters plus evidence policy in Plan 13. Plan 14 makes qualification fail closed and release-ready. Plan 15 adds a deliberately small contextual extension mechanism: one Markdown file per runbook, one bounded retrieval hop, and human-reviewed drafts from installed coding agents.

## North star

> Describe a local outcome, let `uhm` complete one bounded job, and receive the result faster than recalling the syntax or opening a coding agent.

The unit of value is a resolved job, not a generated command. The hard product boundary is equally important: one intent receives an initial proposal and may spend one global second turn on clarification or a user-triggered replacement; then the interaction ends.

## Sequence

| Plan | Milestone | Depends on | Ships |
| --- | --- | --- | --- |
| [Plan 1 — Reset the contract and harden the core](./01-contract-and-core.md) | A truthful, testable base | Nothing | Internal foundation |
| [Plan 2 — Build the result-first Responses loop](./02-result-first-responses-loop.md) | Natural language to a completed local result | Plan 1 | Feature-complete v0.1 core |
| [Plan 3 — Prepare and publish the public release](./03-public-release.md) | Installable, measurable, polished v0.1 | Plans 1–2 | First public release |
| [Plan 4 — Add bounded just-in-time microprograms](./04-jit-microprograms.md) | Complex local transformations without an agent | Public v0.1 evidence | Post-release capability |
| [Plan 5 — Add inspectable local history](./05-local-history.md) | Full decision receipts, inspection, replay, and feedback | Plan 2 | Post-release capability |
| [Plan 6 — Add parent-shell integration](./06-parent-shell-integration.md) | Persistent shell-state actions and opt-in last-command context | Plan 3 | Post-release capability |
| [Plan 7 — Add bounded recovery](./07-bounded-recovery.md) | Verified managed restores and clearly labeled inverse proposals | Plans 4–5 | Post-release capability |
| [Plan 8 — Record an asciinema demo and embed it in the README](./08-asciinema-demo.md) | A seconds-long inline demo of `uhm`'s value | Nothing (independent) | Embedded in-README demo |
| [Plan 9 — Add a containerized end-to-end model benchmark](./09-containerized-execution-benchmark.md) | Reproducible provider/model quality evidence | Plans 2, 4, and 6 action contracts | Offline execution benchmark and 120-task corpus |
| [Plan 10 — Harden v0.2 and ship v0.2.2](./10-v0.2.2-hardening.md) | Repair privacy, execution, recovery, history, CLI, and release gates | Shipped Plans 4–7 and the v0.2.1 review | Corrective v0.2.2 patch release |
| [Plan 11 — Make the model benchmark decision-grade](./11-benchmark-integrity-and-validity.md) | Production-parity validation, semantic oracles, family-aware statistics, and resumable evidence | Plan 9; may proceed alongside Plan 10 without delaying it | Trusted corpus v2 benchmark foundation |
| [Plan 12 — Simplify the microprogram contract and measure bounded repair](./12-simplify-program-contract.md) | Host-owned Python helper and one observable, user-triggered contract repair | Plan 10 and Plan 11 integrity gates; cross-provider qualification also needs Plan 13 Gate A | Reliable schema-v4 microprogram candidate |
| [Plan 13 — Add provider adapters and evidence-gated model policy](./13-provider-adapters-and-model-policy.md) | Explicit OpenAI/Cerebras selection, provider-safe provenance, and qualified dispatch | Gate A: Plans 10 and 11 contract seam; Gate B: Plans 11–12 and Gate A | Fixed opt-in provider choice, then measured policy |
| [Plan 14 — Make model qualification release-ready](./14-release-qualification-pipeline.md) | Sealed holdout, exact frozen gates, blinded adjudication, and deterministic manifest generation | Plans 11–13 | Fail-closed qualification pipeline; qualified entries only after the external holdout gate |
| [Plan 15 — Add one-file contextual runbooks](./15-one-file-contextual-runbooks.md) | Natural-language reuse of human-reviewed project procedures and agent-assisted drafts | Plans 5 and 10; integrates with Plan 13's global call budget | Project/user Markdown runbooks, one-hop retrieval, and bounded authoring |

Plans 1 and 3 contain work that can be developed in parallel, especially release automation, telemetry infrastructure, documentation, and terminal test fixtures. Their completion gates remain sequential. After the public release, numbering expresses a recommended reading order, not a strict merge order. Plan 10 remains the immediate release blocker. Plan 11's schemas/statistics can proceed in parallel, but shared execution extraction waits for Plan 10. After Plan 11's canonical validation seam exists, Plan 12 helper development and Plan 13 Gate A adapter work may proceed in parallel. Plan 13 Gate A must complete before Plan 12's cross-provider holdout; Plan 13 Gate B evidence routing/fallback is last.

## Settled product decisions

These are defaults for implementation, not unanswered questions.

| Topic | Decision |
| --- | --- |
| Product name | Keep the existing `uhm` binary and `uhm-cli` crate identity through the first release. |
| Default interaction | `uhm <intent>` executes ordinary generated actions and returns their result. `--review` stops before execution; `--dry-run` emits the proposal. |
| Compound commands | Allowed as one shell action. The advisory detector describes known compound effects and pauses on detected unknown/consequential combinations; `--force` still proceeds. Compound syntax never becomes an autonomous multi-step plan. |
| Consequential actions | Detection is advisory. Show a richer warning and ask for confirmation for detected deletion, privilege elevation, broad writes, remote mutation, or unknown compound effects. `--force` may always proceed; `uhm` never permanently blocks the user. |
| Safety claim | Make none. Replace “safe” and confidence percentages with detected effects, assumptions, and uncertainty. |
| Conversation boundary | At most two model calls and two executions per job. One global second-call slot may be spent by clarification, user-triggered revision/replacement, Plan 13's explicitly configured and pre-disclosed provider transport fallback before any action is accepted, or Plan 15's one selected-runbook expansion; a local edit before first execution remains part of the initial action. No autonomous retry loop or conversation across unrelated jobs. |
| Model provider | OpenAI Responses remains the production default through Plan 12. Plan 13 may add Cerebras Chat Completions as explicit fixed opt-in behind the same canonical action validator; automatic selection requires matching holdout evidence and never qualifies on speed alone. |
| OpenAI storage | Send `store: false` by default. Local history, not provider-side conversation state, owns continuity. |
| Machine context | `standard` is the default and leaves the device after a first-use disclosure. It includes OS/architecture, shell, a presence-only map of a bounded common-tool catalog, a home-redacted working directory, bounded Git state, and a bounded immediate directory inventory. `minimal` contains no general machine fields. Every mode includes the resolved Python 3 path/version and isolated/no-site availability for deliberate program routing. `full` is an explicit opt-in for bounded host, user, shell-version, and tool-version fields. Environment values, file contents, Git remotes/diffs, and shell history remain excluded unless a separately disclosed feature explicitly supplies them. |
| Local history | Use private, locked JSONL rather than SQLite initially. v0.1 keeps bounded metadata-only receipts by default and offers clear/status controls. Plan 5 adds content-rich per-run artifacts under explicit detail/retention settings. Records, IDs, and content fields never enter telemetry; a separate allowlisted coarse outcome projection may be derived for product events. |
| Telemetry | Enabled by default with a first-use notice and simple opt-out. Collect enum- and count-based product events only—never prompts, commands, paths, file contents, output, API keys, or environment values. |
| Telemetry backend | A stable first-party endpoint on a Cloudflare Worker writing to Workers Analytics Engine. Keep the CLI backend-agnostic behind that endpoint. |
| Supported hosts | Linux and macOS, including SSH and tmux. Native Windows is out of scope. |
| Distribution | GitHub Release binaries are primary. crates.io is a useful secondary path for Rust users but does not block the release. |
| Compatibility | Public v0.1 and v0.2 releases exist. Patch releases preserve documented CLI, configuration, history/recovery, privacy, and output contracts; required schema changes need backward-compatible readers or an explicit migration. |
| Generated code | Do not put Cloudflare's hosted runtime or in-process Monty in v0.1. Start post-release with one local Python 3 microprogram route; ordinary commands and compound pipelines stay on the shell route. Add another runtime only if measured jobs justify it. Use explicit limits and make no sandbox promise. Monty remains non-gating research while it is experimental. |
| Undo | Promise receipts and bounded recovery, not universal rollback. Reserve `undo` for hash-verified restoration of a narrow `uhm`-managed file class, subject to retained snapshots and concurrent-writer limitations. |

## The parent-shell question, plainly

A normal CLI runs as a child process of the user's shell. It can create files, invoke network APIs, and launch other programs, because those effects exist outside the process. It cannot permanently change the parent shell's in-memory state. If `uhm` runs `cd /tmp`, `export FOO=bar`, `source .venv/bin/activate`, or defines an alias, that change disappears as soon as `uhm` exits.

The base binary therefore has a typed parent-shell route plus local detection for common forms. Those recognized actions never report false success. Because arbitrary shell syntax cannot be classified completely, obfuscated or undeclared forms remain an explicit default-trust limitation. Plan 6 adds an optional Bash/Zsh/Fish integration that lets a small wrapper apply an already-reviewed state change in the parent shell. Public v0.1 uses the honest fallback: `uhm` returns the exact recognized command and explains that the user must run it in the current shell.

## Why the roadmap is ordered this way

The current code already has a good terminal-native skeleton, but its argument parsing, configuration fallback, prompt boundary, execution authority, and rendering contain contract-level defects. Building code generation or recovery on top of those defects would multiply ambiguity. Plans 1 and 2 first establish a small typed state machine whose output can be trusted mechanically. Plan 3 then gets that narrow promise into users' hands. Only measured failures of normal command generation should justify Python microprograms, detailed history, shell integration, and recovery machinery in Plans 4–7.

## Research references

External API, runtime, pricing, and release facts below were verified on 2026-08-01 and must be rechecked when the relevant plan is implemented.

- OpenAI recommends the Responses API for new text-generation applications and documents typed output items, function calling, strict schemas, and `store: false`: [Responses migration](https://developers.openai.com/api/docs/guides/migrate-to-responses), [function calling](https://developers.openai.com/api/docs/guides/function-calling), and [conversation state](https://developers.openai.com/api/docs/guides/conversation-state).
- Cloudflare Code Mode's useful idea is capability-bound, isolated execution that returns only the needed result—not its hosted implementation: [original Code Mode](https://blog.cloudflare.com/code-mode/) and [server-side Code Mode](https://blog.cloudflare.com/code-mode-mcp/).
- Monty is a promising Rust-native Python subset with controlled host access and resource limits, but its own repository currently calls it experimental and not ready for production: [Pydantic Monty](https://github.com/pydantic/monty).
- Workers Analytics Engine currently includes a substantial free allowance and retains raw data for three months: [pricing](https://developers.cloudflare.com/analytics/analytics-engine/pricing/) and [limits](https://developers.cloudflare.com/analytics/analytics-engine/limits/).
- GitHub supports release assets, immutable releases, checksums, and artifact attestations; Cargo supports crates.io as a secondary binary installation channel: [GitHub releases](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases), [artifact attestations](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations), and [`cargo publish`](https://doc.rust-lang.org/cargo/commands/cargo-publish.html).
