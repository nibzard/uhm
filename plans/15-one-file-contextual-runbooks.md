# Plan 15 — Add one-file contextual runbooks

## Purpose and dependency

Make repeated project work teachable without creating a plugin runtime or workflow language. One Markdown file describes one runbook. `uhm` sends only runbook names and short descriptions during normal routing, loads the full selected file for one bounded follow-up model call, and then uses the existing typed action, review, execution, and receipt pipeline.

Use an installed coding agent in non-interactive mode to draft or refine those Markdown files from an explicit task description, selected local-history runs, and read-only project inspection. The generated file remains a draft until a human inspects and finalizes it.

The runtime path depends on Plan 10's outbound-data hardening and the existing canonical action validator. Drafting from retained runs depends on Plan 5's local-history detail controls. This plan does not depend on evidence-based provider routing, but runbook expansion shares the global model-call budget with clarification, replacement, and Plan 13 fallback.

## Product thesis

> A project can remember how its team performs recurring work. The user describes the outcome, `uhm` retrieves the relevant human-reviewed procedure, and the normal UHM execution path completes one bounded job.

The value is not a large plugin ecosystem. AI makes writing procedural Markdown and supporting scripts inexpensive. `uhm` supplies the lightweight retrieval, human review, project context, and repeatable invocation that turn one successful task into reusable team knowledge.

The learning loop is:

```text
one-off task or selected successful run
    -> non-interactive agent drafts one Markdown file
    -> human inspects and edits it
    -> human finalizes it
    -> its short description enters the routing catalog
    -> a later natural-language request selects it
    -> one bounded follow-up call uses the full runbook
    -> the existing UHM action pipeline reviews and executes the result
```

## Settled scope

| Topic | Decision |
| --- | --- |
| Unit | One Markdown file is one runbook |
| Identity | The filename without `.md` is the runbook ID |
| Required metadata | One short `description` in YAML frontmatter |
| Body | Free-form Markdown guidance for the model and human reader |
| Execution | A runbook is context, not directly executable code; the selected body produces one normal UHM action |
| Routing | The initial model call sees IDs and descriptions only and may return `use_runbook` |
| Expansion | At most one selected runbook and one follow-up model call; no nesting or retry loop |
| Draft state | Agent output goes under `.uhm/runbooks/.drafts/` and is excluded from routing |
| Finalization | Human review followed by a local move into the active directory |
| Project sharing | Commit active project runbooks to Git and review them like other project documentation |
| User library | Store personal active runbooks in the existing user configuration root |
| Credentials | Describe expected existing profiles, helpers, or environment names; never store secret values |
| Agent use | Drafting/refinement first; explicit complex-task delegation is a later gate and is never an automatic fallback |
| Non-goals | Plugin ABI, remote marketplace, package manager, workflow DAG, parameter schema, embedded scripts, credential vault, signatures, or per-file capability grants |

## 1. Define the one-file format

Project files live at:

```text
<git-root>/.uhm/runbooks/<id>.md
<git-root>/.uhm/runbooks/.drafts/<id>.md
```

Personal files live at the equivalent paths below the existing user configuration directory:

```text
<uhm-config-root>/runbooks/<id>.md
<uhm-config-root>/runbooks/.drafts/<id>.md
```

An active file has one required frontmatter field:

```markdown
---
description: Deploy the service in the current directory to production.
---

# Deploy to production

Use this when the user wants to deploy an existing service to production.
Do not use it for preview environments or first-time infrastructure creation.

## Procedure

1. Require a clean Git working tree on `main`.
2. Identify the service from the current directory.
3. Deploy the exact current commit with `./ops/deploy production`.
4. Use the already configured `acme-production` AWS profile.
5. Never print credentials or tokens.
6. Verify the current commit with `./ops/deploy-status`.

If authentication is unavailable or the target service is ambiguous, ask the
user for clarification. Success means the deployed commit reports healthy.
```

Keep the parser deliberately small:

- Accept only UTF-8 regular files with a `.md` suffix.
- Accept a conservative runbook-ID character set suitable for CLI use.
- Require a non-empty, single-line description with a fixed length bound.
- Bound individual file size and the total catalog bytes with checked constants.
- Reject duplicate active IDs across project and user scope rather than silently shadowing one.
- Ignore `.drafts`, symlinks, nested directories, and unknown file types.
- Preserve the Markdown body verbatim after frontmatter validation.

Do not add structured inputs, effects, steps, prerequisites, or credential fields in v1. Authors express those in prose. Existing UHM action validation and effect review remain authoritative after expansion.

## 2. Discover a bounded catalog

On an ordinary invocation:

1. Resolve the Git root when the current directory is inside a repository.
2. Read active project runbook metadata if project runbooks are enabled.
3. Read active user runbook metadata.
4. Validate IDs and descriptions locally.
5. Sort the catalog deterministically and add only the bounded catalog to the proposal prompt.

The prompt receives data equivalent to:

```text
Available runbooks:

- deploy-prod: Deploy the service in the current directory to production.
- release: Publish a tagged release using the project's release process.
- investigate-api-errors: Collect the standard API error diagnostics.

If exactly one runbook clearly applies, call use_runbook with its ID.
Otherwise return the appropriate normal UHM action or clarification.
```

Start by sending the complete bounded catalog. Do not build semantic indexing, embeddings, activation rules, or a separate ranking model until real catalogs prove that simple descriptions are insufficient.

Draft files never appear in this list. Under `--context minimal`, do not add an automatic project or user catalog; an explicit runbook option may still load a named file after the normal outbound disclosure.

## 3. Add one bounded retrieval hop

Extend the initial proposal schema with one non-executable routing result:

```text
use_runbook(name)
```

`use_runbook` is not an `Action` and can never reach an executor. The host must validate that `name` exactly identifies one active file from the catalog supplied to that request.

The control flow is:

```text
original intent + normal context + runbook catalog
    -> first model response
       -> normal action/clarification: existing behavior
       -> use_runbook(id): validate and load exactly that file
          -> original intent + normal context + full selected runbook
          -> second model response using only the normal UHM action tools
          -> existing validation, requirements, effect review, execution, and receipt
```

The follow-up prompt must:

- preserve the original user intent verbatim;
- identify the selected runbook by ID and scope;
- delimit the Markdown as task-specific reference material;
- state that it cannot override host policy, tool schemas, or the original intent;
- omit `use_runbook` from the available tools;
- require exactly one existing UHM action or clarification; and
- include no other runbook bodies.

Runbook expansion consumes the current global second-call slot. There is no nested runbook selection, automatic repair, provider fallback, or later revision in the same job after expansion. If the expanded response asks for clarification, render the question and end the job honestly; the user may invoke `uhm` again with the missing detail.

Add an explicit form for inspection, testing, ambiguity, and cases where the first call cannot spend a routing turn:

```sh
uhm --runbook deploy-prod deploy this to production
```

This form skips catalog routing and makes one model request containing the named runbook. It still uses the normal action validator and review/execution path.

If the initial provider attempt has already consumed the second-call slot through configured transport fallback, a natural `use_runbook` result cannot expand. Report the limit and show the explicit `--runbook` invocation rather than silently ignoring the selection.

## 4. Keep project activation simple

Project runbooks add repository-authored descriptions and potentially a full Markdown file to outbound model context. Before first use in a repository, ask once whether to enable that repository's runbook directory. Store one local project-level choice using the canonical repository root; do not build content-addressed per-file grants in this plan.

User-scope files are enabled by default after the existing owner/private-directory checks. Active project files are expected to be reviewed through the repository's normal Git workflow.

Treat catalog descriptions and full bodies as untrusted contextual data in both prompts. Sanitize terminal rendering, apply byte bounds before model submission, and retain the normal effect review for the generated action. A runbook may guide the task but cannot weaken UHM policy.

Update the first-use and privacy documentation to state separately that:

- active runbook IDs and descriptions may be included in normal model requests;
- a full runbook body leaves the device only after selection or explicit invocation;
- drafts are never included in normal requests; and
- secret values must never be written into runbooks.

No runbook content or ID enters telemetry. Metadata history may record `route=runbook`, source scope, and a local hash of the ID/body. Exact IDs and body content follow existing diagnostic/full retention policy.

## 5. Bootstrap drafts with a non-interactive coding agent

Add a small authoring workflow:

```sh
uhm runbook list
uhm runbook show deploy-prod
uhm runbook draft deploy-prod --agent claude --from <run-id>
uhm runbook finalize deploy-prod
uhm runbook refine deploy-prod --agent codex --from <run-id>
```

`draft` may take a plain user description, one or more explicit history IDs, or both. It never searches or uploads all history implicitly. If a selected record was stored at a detail level that lacks the original intent or accepted proposal, fail with an explanation instead of silently inventing the missing evidence.

The fixed authoring prompt asks the agent to produce exactly one Markdown document containing:

- a concise frontmatter description;
- when the procedure should and should not be selected;
- the project's existing commands and conventions;
- required project facts, tools, and authentication expectations;
- conditions that require clarification;
- expected success and verification; and
- prohibitions such as exposing secrets or widening destructive scope.

The selected agent runs non-interactively from the project root with repository reads allowed and workspace writes denied. UHM supplies the selected, redacted history material and requires the candidate Markdown on stdout. UHM, not the agent, writes the one bounded output file under `.drafts` after validating UTF-8, size, frontmatter, and ID.

Start with small built-in adapters for the installed agents the project chooses to support. Each adapter only needs to:

- detect executable/version without invoking a model;
- construct direct argv for non-interactive, read-only execution;
- pass the authoring prompt without shell interpolation;
- capture bounded stdout/stderr with a timeout; and
- distinguish unavailable, failed, malformed, and successful output.

Do not add arbitrary configured shell templates in the first implementation. Do not persist or resume agent sessions. Agent authentication remains owned by the installed CLI and is never copied into runbook content.

`finalize` shows the complete draft or diff, asks for confirmation when interactive, validates it again, and moves it into the active directory. It performs no execution. `refine` always writes a new draft or side-by-side candidate and never overwrites an active file.

## 6. Defer general agent delegation behind a separate gate

After agent-assisted drafting is useful and reliable, add an explicit foreground command for complex one-off work:

```sh
uhm agent fix the failing parser tests
uhm agent --using claude investigate this build failure
```

This command is never selected automatically from an ordinary natural-language request and is not a runbook action. It invokes one installed non-interactive agent, streams bounded progress, returns the agent's result, and exits. It must preserve UHM's no-background-work and no-recursive-delegation boundaries.

A completed agent job may be selected later as input to `uhm runbook draft`, but completion alone does not finalize a runbook or prove the task succeeded. Ship this gate only after cancellation, timeout, output bounds, environment handling, and coarse history receipts have direct integration coverage.

## 7. Implementation seams

Keep the code change narrow:

- Add `src/runbook.rs` for paths, frontmatter parsing, bounded discovery, draft/finalize operations, and catalog rendering.
- Extend the provider-neutral decoded result with `UseRunbook`, or wrap the current action result in a proposal decision type. Do not add it to the executable `Action` enum.
- Let `src/prompt.rs` accept an optional catalog for the first request and one selected Markdown body for the expansion request.
- Let `src/command.rs` own the single retrieval hop and account for it in the existing global call budget.
- Reuse current action validation, cache policy, effect detection, review, execution, output, and receipts after expansion.
- Add `src/agent.rs` only for bounded authoring adapters; keep it separate from provider adapters used to generate UHM actions.

Cache provenance for a catalog-bearing request must include the ordered ID/description catalog hash. An expanded request must additionally include the exact selected-body hash. A cache hit never bypasses current file validation or project enablement.

## 8. Tests and validation

Add offline tests covering:

- project and user discovery, deterministic ordering, and duplicate rejection;
- draft exclusion, symlink rejection, UTF-8 errors, frontmatter errors, and size bounds;
- exact catalog bytes sent to the initial prompt;
- no full body sent until selection;
- rejection of a selected ID absent from the supplied catalog;
- exactly one retrieval hop and absence of `use_runbook` on the expanded call;
- preservation of the original intent and normal context in the expanded prompt;
- second-call budget interaction with clarification, replacement, and provider fallback;
- explicit `--runbook` behavior;
- unchanged validation, effect review, and execution for the final action;
- project enablement and `--context minimal` behavior;
- no runbook contents in telemetry or metadata history;
- agent unavailable/timeout/non-zero/malformed/oversized-output handling;
- agent authoring that can read the repository but cannot write it directly;
- draft validation, finalization, refinement without overwrite, and non-activation before finalize; and
- secret sentinels absent from authoring prompts, rendered errors, history, and generated fixtures.

Create a small checked-in routing fixture set with positive and negative examples for overlapping runbooks. The initial product gate is not a large benchmark: it is enough to demonstrate that common project requests select the right runbook, explanatory requests do not, ambiguous requests clarify, and explicit invocation behaves deterministically.

## Delivery sequence

1. One-file parser, project/user discovery, `list`, `show`, and draft exclusion.
2. Catalog prompt plus `use_runbook` and the one bounded expansion hop.
3. Explicit `--runbook`, project enablement, privacy/history/cache provenance, and end-to-end fixtures.
4. `draft`, `finalize`, and `refine` with one read-only non-interactive agent adapter.
5. A second authoring adapter only after the first adapter contract has integration coverage.
6. Explicit `uhm agent` delegation only after authoring proves the wrapper useful.

## Completion criteria

- A project can commit one reviewed Markdown file and have its description participate in ordinary natural-language routing.
- Selecting that description loads exactly one full file for exactly one bounded follow-up model request.
- The final response enters the existing UHM review and execution pipeline unchanged.
- An explicit runbook invocation uses one model call and bypasses catalog selection.
- A supported installed coding agent can turn selected evidence into one inactive draft without writing elsewhere in the repository.
- A human can inspect, edit, finalize, and later refine the same one-file runbook.
- Project and user runbooks work without a registry, package system, workflow DSL, or secret storage feature.
- Documentation clearly distinguishes runbook guidance, executable UHM actions, and explicit external-agent jobs.
