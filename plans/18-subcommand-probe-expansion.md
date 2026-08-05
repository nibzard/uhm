# Plan 18 — Let the model deepen the tool surface it was given

## Purpose and dependency

Plan 16 Phase 3 shipped the capability surface: when a request names an installed tool, its top-level help output rides in the first model request after one-time per-binary consent. Measured on the motivating intent, it removed both baseline failures — no sample invented a subcommand, and the clarification requests that dominated the baseline disappeared.

It also exposed the next defect, by inspection rather than sampling: `steel --help` lists `browser` but does not contain the string `navigate`. The operation the intent needs is absent from the bytes supplied, so the model composes from a surface that omits the verb it is looking for. The observed results are exactly what that predicts. Before the single-tool guidance, proposals reached for a second utility (`&& open <url>`). After it, they stayed inside the named tool and silently dropped the target instead — `steel browser start --session hacker-news`, with a receipt that says `completed` and nothing that reports the missing half. One sample asked the user to paste `steel describe browser` output, which is the model requesting, by hand, the probe this plan automates.

This plan adds one typed, host-validated expansion: the model may name one subcommand of a tool whose surface it was given, the host reads that subcommand's help under the existing probe machinery, and one follow-up call re-proposes. It depends on Plan 16 Phase 3's `tool_surface` store, consent record, and probe bounds, and on the Plan 15 §3 `use_runbook` pattern for typed non-executable routing results. It amends one settled decision — the conversation boundary — and that amendment is recorded here rather than smuggled in as a side effect.

## Product thesis

> When the model is missing a machine-readable fact, it should get to ask the host for it — never the user.

The clarification loop cannot converge on interface gaps. The measured session that motivated Plan 16 spent both model calls asking the user what a Steel session was; the user answered honestly twice and neither answer could help, because the missing fact was the subcommand surface, which the user would have had to paste by hand. A probe turn is the same shape with opposite economics: the answer is local, bounded, machine-readable, and already consented. More clarification turns multiply useless questions; one probe turn ends them.

The expansion also amortizes to zero. Probe results persist in the Plan 16 store keyed on the binary's path, size, and mtime, so the second call is paid once per (tool, subcommand) per binary version — ever. The next request naming the same tool carries the deepened surface in its first call and is a one-call job again.

## Measured evidence

Live `--dry-run --fresh` proposals for `start steel session and open hacker news`, default provider pair, 2026-08-04. All sample counts are small; they size the problem and are not a benchmark. The structural claim does not rest on them: the absence of `navigate` from `steel --help` is checkable by reading the file this plan exists to supply.

| Condition | Samples | Outcome |
| --- | --- | --- |
| No surface (baseline) | 6 | 5 clarifications, 1 invented interface |
| Top-level surface | 4 | 0 invented subcommands, 0 clarifications; 3 chained `open`, 4 invented a `--session` flag |
| Surface + single-tool guidance | 4 | 0 chained; 2 silently dropped the target, 1 asked the user to paste deeper help |
| Subcommand list named in the request (proxy for this plan) | 2 | 2 identical correct compositions, no invention |

The last row is the existence proof: given the deeper surface, the model composed `steel browser start && steel browser navigate <url> && steel browser live` deterministically. This plan makes that surface reachable without the user writing it into the request.

## The boundary amendment

`plans/README.md` records: at most two model calls and two executions per job, with one global second-call slot spendable by clarification, user-triggered revision/replacement, configured transport fallback, or Plan 15 runbook expansion.

This plan amends that decision as follows, and the amendment ships as its own change to the settled-decisions table:

- A job may spend **at most one probe expansion**, which is an additional model call that does **not** consume the second-call slot. The ceiling becomes: initial call + at most one machine-answered probe expansion + the existing one replacement slot. Three calls, bounded, no loop.
- The expansion is slot-neutral because consuming the replacement slot would recreate the exact pathology v0.3.5 and v0.3.6 repaired: an expansion followed by a failed command would leave no repair turn, and the review prompt would have to hide options the budget could no longer honor. The hard guarantee of Plan 16 §4 — the expansion cannot exhaust the slot the user needs for revise, edit, or repair — holds by construction.
- Nothing else changes: two executions per job, no autonomous retry, no conversation across jobs, and the expansion is never available to ask/explain routes, which still may not propose actions at all.

The rationale for the original ceiling was preventing user-facing turn loops and keeping the product faster than recalling syntax. A probe turn adds no user interaction and is bounded to one; the amortized call count returns to one as the store warms. The ceiling's purpose survives the amendment; only its letter changes.

## Settled scope

| Topic | Decision |
| --- | --- |
| New typed result | `probe_subcommand(tool, subcommand)`, non-executable, modeled on Plan 15's `use_runbook`; never an `Action`, can never reach an executor |
| Who names what | The model names one subcommand token; the host builds the probe argv. The model never chooses argv, flags, or paths |
| Tool validation | `tool` must exactly match a name in the `named_tools` surface supplied in that request |
| Subcommand validation | One token, same character rules as `tool_surface::tokens`, and it must appear verbatim as a word in that tool's retained top-level help — the model can only deepen along a path the tool itself advertised |
| Probe execution | `[path, subcommand, "--help"]` with the existing `HELP_FLAGS` fallback order, existing deadline, existing per-probe and total byte caps |
| Consent | None asked. The binary is already allowed; probing more of its own help gains no authority the first consented probe lacked. A tool absent from the surface cannot be named, so an unconsented binary is unreachable |
| Persistence | Additive `subcommands` map on the Plan 16 store record, serde-defaulted so existing stores parse; no store version bump, no discarded consent answers |
| Follow-up call | Exactly one; carries the original intent verbatim, the deepened surface, and the normal action tools only. `probe_subcommand` is omitted from the expansion call's tools, so nesting is structurally impossible |
| Budget | A dedicated expansion flag on `Budget`, separate from `replacement`; receipts record that an expansion occurred |
| Cache | The expansion call is uncached (follow-up calls already are). The deepened surface changes the snapshot, so the existing context hash keeps cached first-call proposals honest |
| Surface assembly | Top-level help first, then retained subcommand entries whose token appears in the intent, then remaining entries by recency, all within the existing ceilings |
| Non-goals | Multi-level paths, multiple expansions per job, model-chosen argv, eager recursive probing, new consent categories, any change to execution authority or review |

## Rejected alternatives

**Eager recursive probing at consent time.** Probing every listed subcommand when the user first allows a tool would run ~20 executions of a binary the user approved for one, exceed the plain reading of the consent prompt, require heuristic parsing of help text to find subcommand names, and store mostly irrelevant bytes. Selecting the one relevant subcommand is the model's comparative advantage; executing a fixed probe is the host's. The typed result keeps each on its own side.

**Raising the byte ceiling and sending everything.** A full recursive help dump for a tool the size of `steel` is tens of KiB per request, every request, for surface that is almost entirely irrelevant to any single intent. Requests are sent with `store: false`, so nothing is amortized provider-side.

**Tool-specific structured self-description** (`steel describe --json`). Not generalizable across arbitrary tools, and it would mean the model naming argv — the line this plan family never crosses.

**Host-side heuristic subcommand selection.** Matching intent tokens against subcommand names and descriptions would sometimes pick `browser` from "session". It is guessing with extra steps, and guessing from partial evidence is the defect this plan family exists to remove.

## 1. Ship the completeness clause first

Independent of the probe and shippable immediately. The single-tool guidance as written gives the model no legal move when the shown surface cannot reach the target, and the measured result was silent partial success — the worst behavior in the system, because the receipt reads `completed`.

Amend the `src/prompt.rs` shell-action guidance with one rule: never silently satisfy part of a request. If the named tool's shown surface cannot reach the target, chaining a minimal standard utility is better than dropping the target, and saying which part is not covered is better than either. This stays correct after the probe ships: it becomes the fallback for tools whose help genuinely lacks the operation.

## 2. The typed result and its validation

Extend the proposal schema with `probe_subcommand`, a routing result beside `request_clarification`, decoded into the provider-neutral result type but never into the executable `Action` enum. Host validation, in order, each failure falling back to treating the response as a normal proposal failure rather than executing anything:

1. The route allows expansion (run/recover only) and the job's expansion flag is unspent.
2. `tool` exactly matches a `named_tools` entry supplied in the request that produced this response.
3. `subcommand` passes the `tool_surface` token rules and appears verbatim as a whitespace-delimited word in that tool's retained top-level help.
4. The binary's identity record still matches on disk — a tool that changed between calls is re-consented, not silently probed.

A hostile tool controls its own help text and therefore which tokens are probeable — but it is probing itself, with no authority it did not gain when the user consented to its first probe. Help output remains untrusted end to end: sanitized before rendering or submission, bounded, never policy-bearing.

## 3. The expansion call

On a valid probe request: run the probe, persist the observation under the tool's record, rebuild the `named_tools` field with the deepened surface, and make exactly one follow-up call whose input carries the original intent verbatim and a follow-up payload identifying the probe that answered. The expansion call's tool list omits `probe_subcommand`. A probe that produces no usable help (unrecognized subcommand at runtime, empty output, deadline) makes the follow-up call with the surface unchanged and the failure stated in the payload, so the model composes from what exists rather than being asked to guess again.

Render one narration line to stderr while probing (`uhm: reading steel browser --help`), because the alternative is an unexplained second spinner.

## 4. Budget, receipts, and history

Add an expansion flag to `Budget` with the same discipline as `replacement`: spendable once, checked before the probe runs, never resettable. Receipts and metadata history record that an expansion occurred and its coarse outcome (`probed`, `probe_empty`, `invalid_probe`) — never the tool name, subcommand, or help bytes, matching the existing telemetry posture. The interaction summary gains one enum value so the funnel can distinguish one-call jobs, expanded jobs, and expanded-then-replaced jobs.

## 5. Measure before calling it fixed

The plan's own evidence tables are n≤6 per condition and say so. Before the completion criteria are checked off, run the fixed intent set — the motivating intent plus at least one more naming a different uncataloged tool — at 10+ fresh proposals per condition, and record: invented subcommands, invented flags, dropped targets, chained unrelated tools, clarifications, probe requests, and correct complete compositions. The repo's existing benchmark machinery is not required; a checked-in script and a results table in this file are enough. The gate is that correct complete compositions become the majority outcome for the motivating intent and no regression appears in the fixture intents that name no tool.

### §5 results — run 2026-08-04

`scripts/measure-plan-18.sh 12`: 36 live `--dry-run --fresh` proposals against the default provider, isolated HOME and tool-surface store, `steel` consented once and its subcommands cleared before every sample so each proposal faced the same top-level-only surface. `telemetry.enabled: false`; the intent text was sent to the provider as part of each proposal. Raw per-sample output is checked in at `scripts/measure-plan-18-results.json`.

| Condition | Intent | n | Probe fired | Outcome (against confirmed `steel` grammar) |
| --- | --- | ---: | ---: | --- |
| steel-browser | open a steel browser session and navigate to hacker news | 12 | 12 | 12 correct complete: `steel browser start … && steel browser navigate <url>`. `start`, `navigate`, and `--session` are all real (`steel browser --help` advertises `start`/`navigate`; both verbs are absent from `steel --help`), so the depth was genuinely missing and the probe supplied it every time |
| steel-sessions | show my active steel sessions | 12 | 0 | 12 produced `steel browser sessions` (±`--json`). The run's classifier printed `dropped_target`, assuming `steel sessions list` was the only valid target; `steel browser --help` advertises `sessions` as "List active browser sessions", so these are correct complete compositions reached from prior knowledge, with no missing fact to probe |
| no-tool | count the number of lines in /etc/hostname | 12 | 0 | 12 correct (`wc -l …`); zero spurious probes |

Gate check: the motivating intent is a correct complete single-tool composition in 12/12, with zero invented subcommands and zero dropped targets; the no-tool fixture shows no regression (12/12 correct, zero probes). The probe fired deterministically where the model reached for depth (browser) and stayed dormant where the model already held a working verb (sessions) or named no tool — the intended selectivity.

Honest limitation: the probe is model-initiated, so it can only fire when the model recognizes that a machine-readable fact is missing. This run exercised the recognizes-and-probes case, the already-knows-the-verb case, and the no-tool case, and the model was correct in all 36. It did not exercise a model that is confidently wrong about a verb it lacks — the case where probing ought to fire but the model does not reach for it. The host cannot close that case without executing the proposal to learn it failed, which is out of scope here and unchanged from Plan 16. What the browser row measures is the guarantee this plan adds: when the model does ask, the depth is supplied, validated, persisted, and amortized.

## 6. Tests

Offline, red first:

- Decoding accepts `probe_subcommand` and it is unrepresentable as an executable action.
- Each validation rule rejects: unknown tool, token absent from retained help, hostile token characters, spent expansion flag, ask/explain routes, changed binary identity.
- The probe argv is exactly `[path, subcommand, flag]` for the allowlisted flags, no shell, and hostile help content cannot alter it.
- The expansion call's tool list omits `probe_subcommand`; a second probe request in one job is rejected without a call.
- The store roundtrips the subcommand map; existing version-1 records without it still parse and keep their consent answers.
- Surface assembly respects the ceilings and the intent-token priority rule.
- `minimal` mode carries no deepened surface; sanitization holds for subcommand help exactly as for top-level help.
- Budget: expansion does not consume the replacement slot; revise/edit/repair remain offered after an expansion; the review prompt derivation reflects that.
- An ordinary job whose surface suffices still makes exactly one model call.
- Receipts record expansion outcomes as enums only; no tool name or help bytes appear in telemetry or metadata history.

## Delivery sequence

1. Completeness clause in `src/prompt.rs`. One sentence, independent, immediate.
2. The conversation-boundary amendment row in `plans/README.md`, committed as its own change.
3. Store extension and surface assembly in `src/tool_surface.rs`.
4. Typed result, validation, expansion call, budget flag, receipts.
5. The measurement run from §5, results recorded here.
6. Release note stating the new ceiling plainly: one extra machine-answered call, first time a tool's depth is needed, then cached.

## Completion criteria

- The motivating intent produces a correct, complete, single-tool composition as the majority outcome at 10+ samples, with zero invented subcommands and zero silently dropped targets.
- A job that expands still offers revise, edit, and repair afterwards, and no prompt advertises an option the budget cannot honor.
- The probe is unreachable for unconsented binaries, un-nameable tools, and unadvertised subcommands, with tests for each.
- A warmed store returns the tool's jobs to one model call, verified by a request-count assertion.
- The boundary amendment is recorded in the settled-decisions table, and receipts distinguish expanded jobs without leaking what was probed.
- Ask and explain routes are unchanged, and no new consent question, outbound category, or execution authority exists that Plan 16 did not already disclose.
