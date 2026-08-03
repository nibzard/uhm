# Plan 12 — Simplify the microprogram contract and measure bounded repair

## Purpose and dependency

This plan replaces the error-prone raw environment-manifest interface used by generated Python with a small host-owned helper. It also makes common contract failures eligible for the one existing user-triggered replacement turn before execution.

It builds on Plan 4's bounded microprogram capability, starts after Plan 10's program-runtime and privacy hardening is complete, and uses Plan 11's production-parity benchmark path. Plan 11's integrity work may proceed in parallel, but its contract-parity, host-oracle, image-identity, and family-statistics gates must be complete before qualification. Plan 13's no-op OpenAI adapter extraction and fixed Cerebras adapter must also exist before cross-provider holdout calls; qualification must exercise production adapters rather than the old Python-only wire client.

The interaction boundary does not change:

```text
one initial proposal
    → structural validation and program preflight
        → valid: at most one execution
        → hard contract error: optional user-triggered complete replacement
    → after an executed failure: optional user-triggered complete replacement
    → at most two model calls and two executions total
    → stop
```

There is no automatic repair loop, hidden retry, repository work, extra model turn, or broader runtime authority.

## Evidence behind the plan

- Terra completed 57 of 144 expected-program attempts; Cerebras completed none.
- Cerebras's correctly routed programs split almost completely between conventional `sys.stdin` reads and hard-coded workspace paths. Terra also frequently ignored the managed stdin path or misunderstood input/output cardinality and staging.
- The current prompt asks a model to coordinate source, raw JSON environment manifests, special `stdin` semantics, input access, separate outputs, and `result_mode`. Wire-valid code can therefore be plausible Python yet unusable by UHM.
- Parent-shell actions completed 24/24 attempts per model, and bounded shell writes completed 60/60 for Terra and 59/60 for Cerebras. Narrow typed operands with host-owned mechanics are the proven design pattern.

The goal is to simplify the semantic bridge, not to hide model errors or weaken managed staging.

## Settled contract

Increment the relevant versions when implementation begins:

- `PROMPT_VERSION`: 8 → 9.
- `ACTION_SCHEMA_VERSION`: 3 → 4.
- Program contract: `manifest_env_v1` → `uhm_helper_v1`.

New model responses use this conceptual `run_program` shape:

```json
{
  "runtime": "python3",
  "contract": "uhm_helper_v1",
  "source": "...",
  "summary": "...",
  "assumptions": [],
  "stdin_mode": "none",
  "files": [
    {"id": "source", "path": "input.json", "access": "read_only"},
    {"id": "result", "path": "output.json", "access": "write_only"}
  ],
  "effects": ["read_local", "write_local"]
}
```

`stdin_mode` accepts:

- `none`: piped bytes, if present at the CLI, are not exposed to this program.
- `local_path`: piped bytes are exposed as a private file through the helper. Whether those bytes were also model-visible is determined by the existing input/privacy context, not by this enum.

File access accepts:

- `read_only`: an existing logical path is supplied only for reading.
- `write_only`: a private staging path is supplied without read access to the destination. It may create or replace only after the existing overwrite/review policy authorizes that destination.
- `read_write`: an existing validated regular file is required; separate current-read and staging paths are supplied for managed replacement.

Rules:

- Remove separate `inputs`, `outputs`, the special logical path `stdin`, and `result_mode` from new proposals.
- `local_path` requires a present piped spool. `--local-input` may only be consumed through this mode and remains incompatible with a shell action under the current product contract.
- Any writable file derives artifact-result behavior; no writable file derives stdout-result behavior.
- One logical path appears at most once. Replacement uses one `read_write` entry rather than duplicating a path across arrays.
- Every resource has a unique stable model-authored ID matching `[a-z][a-z0-9_]{0,31}`. Source refers to IDs, never array position or logical host path.
- Effects remain required and are merged with local detection before review or execution.
- New API responses must explicitly name `uhm_helper_v1`; legacy history may decode the old contract but a new response may not select it.

## 1. Add a trusted launcher and in-memory helper

For each program, create private runtime files for:

- the model-authored source;
- a trusted launcher shipped by UHM; and
- a private, one-use launcher contract containing resolved read and staging paths.

Invoke the resolved interpreter directly:

```text
python3 -I -S launcher.py source.py contract.json
```

Before model source runs, the launcher:

1. Reads and unlinks the private launcher contract.
2. Constructs an immutable resource lookup keyed by the declared IDs and containing only resolved private read/staging paths.
3. Registers an in-memory `uhm_runtime` module in `sys.modules`.
4. Resets `sys.argv` so model source cannot discover launcher operands.
5. Executes the separately stored source through `runpy.run_path` with a traceback filename that identifies the model source.

The helper surface is deliberately small:

```python
from uhm_runtime import stdin_path, resource

# stdin_path: pathlib.Path | None
# resource(id): returns one immutable resource
# resource.read_path: pathlib.Path | None
# resource.write_path: pathlib.Path | None
```

It exposes no provider client, network helper, shell, secret, history handle, logical destination, or additional authority. Generated Python remains an ordinary local process with the user's permissions; the helper improves correctness and staging discipline, not containment.

Put two exact minimal scaffolds adjacent to the `source` field description and in the developer prompt.

Stdout from piped input:

```python
import json
from uhm_runtime import stdin_path

data = json.loads(stdin_path.read_text(encoding="utf-8"))
print(json.dumps(data, sort_keys=True))
```

Managed artifact:

```python
from uhm_runtime import resource

text = resource("source").read_path.read_text(encoding="utf-8")
resource("result").write_path.write_text(text.upper(), encoding="utf-8")
```

Process stdin remains closed. The cwd is not the user's cwd; declared resources are available only through `stdin_path` and `resource(id)`, even though private runtime files may exist there. State those facts directly in the prompt and review UI.

## 2. Normalize resources into the existing safe commit path

Convert the new declarations into an internal execution plan before spawn:

- `stdin_mode=local_path` resolves only the piped-input spool; stdin is not represented as a file resource.
- `read_only` resolves to the existing validated logical file path for reading.
- `write_only` creates a collision-resistant same-filesystem staging path and a new-file destination plan.
- `read_write` creates a validated current-read path plus a distinct staging path and managed-replacement plan.
- A writable declaration derives artifact verification and commit; an all-read declaration derives stdout handling.

Reuse Plan 10's corrected deadline, descendant cleanup, environment allowlist, workspace traversal, staging, fsync, recovery capture, conflict detection, and commit logic without semantic changes. Failure commits no declared artifact. Unmanaged side effects remain possible and must remain disclosed.

Retained debug workspaces include launcher and source with private permissions, but never retain the one-use resolved contract after the launcher has read it. History may retain the logical proposal at the configured detail level; it must not retain resolved staging paths.

## 3. Catch common contract mistakes before execution

Add a structured `ProgramContractDiagnostic` with stable, content-free codes and an explicit severity of `hard_error`, `warning`, or `availability`, including:

- `invalid_python_syntax`;
- `process_stdin_is_closed`;
- `builtin_input_is_unsupported`;
- `declared_path_opened_directly`;
- `helper_not_referenced`;
- `stdin_not_consumed`;
- `read_resource_not_consumed`;
- `write_resource_not_consumed`;
- `duplicate_resource`;
- `invalid_resource_access`; and
- `runtime_unavailable`.

Hard errors make `preflight_valid=false` and may be offered for user-triggered contract repair: invalid syntax, direct process-stdin/input use, duplicate/invalid IDs or access, a statically proven direct open of a declared logical path, and a statically proven missing required write resource. `runtime_unavailable` is an availability outcome, not a model-contract failure; it may be offered as a user-triggered route replacement. Incomplete analyses such as “resource not consumed,” dynamic aliasing, or unprovable helper use are warnings shown in review and counted separately, not unstable hard rejections.

Use a trusted AST-only check through the resolved Python interpreter, under the same stripped environment and source bound, before review/execution. It parses but never executes model source. The check should:

- reject syntax errors;
- reject `sys.stdin`, `sys.__stdin__`, and built-in `input()` when process stdin is unavailable;
- inspect Python string literals rather than broad substrings when detecting direct use of declared logical paths;
- recognize normal `uhm_runtime` imports and aliases;
- validate every statically referenced `resource("id")` against the declaration and reject unknown IDs;
- require `stdin_path` use when piped bytes were declared;
- require helper file access when files were declared; and
- require a `write_path` reference when any writable resource exists.

Keep the analysis conservative. It catches the observed protocol failures; it does not attempt to prove arbitrary Python correct, complete, side-effect free, or safe. A valid dynamic access pattern receives a review warning rather than a false hard rejection when static proof is not possible. Contract-validity denominators count only the absence of hard errors; warning rates are reported independently.

The AST checker is a trusted internal subprocess, not an action execution. Give it a fixed short deadline, bounded output, `python3 -I -S`, and Plan 10's stripped environment; pass source as data to a trusted parser that never imports, evaluates, or executes it. It does not consume the user's execution budget.

Production and Plan 11's benchmark invoke the same validator and report separately:

- provider wire-schema validity;
- UHM semantic contract validity;
- execution startup;
- runtime outcome; and
- artifact commit outcome.

## 4. Make semantic contract failures eligible for one replacement

Refactor provider response parsing so a syntactically decoded `run_program` proposal can be retained with a structured semantic diagnostic instead of collapsing immediately into an opaque model error.

Before execution in an interactive terminal, show one concise choice:

```text
Program contract error: process stdin is closed.
Repair or stop? [r/N]
```

If the user requests repair:

- consume the one global replacement/model-call slot;
- send the original intent, prior model-authored proposal, stable diagnostic code, and bounded sanitized explanation;
- exclude resolved host paths, staging paths, launcher contract, credentials, child output, and local-only input bytes;
- request a complete replacement action, never a patch;
- re-run canonical validation, effect classification, review policy, and execution normally; and
- permit at most one execution because the rejected proposal never ran.

For contract-preflight repair, the payload may contain only model-authored source, logical declarations/resource IDs, the stable diagnostic code, and a content-free explanation. For runtime repair with `--local-input`, use only Plan 10's coarse typed failure outcome—never child stdout, stderr, exception text, or resolved paths. Where Plan 10 requires an outbound preview/approval, the approved bytes and request seed must be identical.

Existing runtime-failure repair remains user-triggered, may include only the already-approved bounded diagnostic path, and may execute one complete replacement. Do not attempt repair after a zero-exit semantically wrong outcome: production has no trustworthy result oracle.

| Scenario | Model calls | Executions |
| --- | ---: | ---: |
| Valid first proposal | 1 | 1 |
| Invalid contract, user stops | 1 | 0 |
| Invalid contract, repaired | 2 | 1 |
| Runtime failure, repaired | 2 | 2 |
| Clarification/revision already used | At most 2 | No repair slot |
| Replacement is invalid or fails | 2 | Stop |
| Non-interactive invocation | 1 | Never repair automatically |

Every transition records the first proposal outcome locally before offering replacement, while telemetry remains one coarse final interaction summary as constrained by Plan 10.

## 5. Preserve compatibility deliberately

- Define separate `LegacyProgramProposalV1` and `HelperProgramProposalV2` types plus a versioned stored-proposal envelope.
- Read existing bare schema-v3 `ProposedAction` history as legacy through a dedicated backward reader; do not rely on merely defaulting a missing `contract` field after the schema shape changes.
- Decode new live API output only as schema v4, so omission of `contract` can never select legacy behavior.
- Normalize both stored versions into one internal execution plan only after version-specific validation.
- Retain a legacy internal renderer/executor only where an existing history or recovery workflow requires it; new provider output cannot request it.
- Keep old history readable and render its contract version explicitly. Do not rewrite historical receipts.
- Store proposals and attempts append-only (`proposal-1`, `proposal-2`, provider-attempt index, execution-attempt index, and accepted-proposal reference) so replacement cannot overwrite the first rejected or executed action. Coordinate these events and export bounds with Plan 10.
- Include prompt, action-schema, program-contract, provider, and API-family versions in cache provenance. A version change must miss rather than reinterpret an old cached response.
- Normalize every future provider wire response into the same schema-v4 internal action before semantic validation.
- Add a schema-v4 reference-action bundle for Plan 11's frozen corpus-v2 task/oracle identities. Do not change Plan 9's corpus v1 or its retained benchmark artifact.
- Update behavior, program, privacy, history, recovery, CLI, and configuration documentation together with the schema change.

## 6. Measure first shot and one realistic repair

Extend the Plan 11 runner with two separately reported profiles:

1. `first-shot`: exactly one proposal and at most one execution.
2. `bounded-repair`: request one replacement only after a contract failure or nonzero/timeout/overflow execution that production can observe.

The repair prompt receives only production-available evidence. It must not include the hidden expected answer, failed deterministic assertion, oracle diff, judge rationale, or benchmark fixture metadata unavailable to UHM.

Record:

- wire-schema, structural-client, and hard-error-free preflight validity;
- hard diagnostic and warning frequencies separately;
- first-shot `completed_outcome` using Plan 11's orthogonal fields;
- startup, exit, and artifact-commit success;
- repair eligibility and user-approved benchmark simulation;
- conditional repair success and cumulative-if-approved completion;
- added latency, candidate tokens, and model-call count; and
- broad destructive-scope failures.

The benchmark necessarily simulates a user approving every eligible repair. Treat cumulative-if-approved completion as a technical recovery ceiling, not expected user completion and not a substitute for first-shot quality; real non-TTY users receive no repair and interactive users may decline.

Use Plan 11's visible development split for prompt/helper iteration and its locked private holdout for the release decision. A model/provider is program-qualified only on the holdout and only after production adapters from Plan 13 are in the evaluation path.

## Implementation sequence

### Phase A — Versioned contract and compatibility

- Add schema-v4 and legacy/new internal program types.
- Normalize new file declarations into an execution plan.
- Add historical decode/render fixtures and cache-version tests.
- Add state-transition tests before changing the prompt.

### Phase B — Trusted helper runtime

- Implement launcher, one-use contract, immutable in-memory helper, and private file lifecycle.
- Connect resources to the existing staging/recovery path.
- Cover stdout, new artifact, replacement, multifile, failure, and retained-workspace behavior.

### Phase C — Semantic preflight and repair UX

- Add stable diagnostics and trusted AST parsing.
- Preserve invalid semantic proposals through the command layer.
- Add user-triggered pre-execution repair without changing the global budget.
- Prove local-input and credential bytes cannot enter a follow-up.

### Phase D — Prompt, docs, and corpus-v2 references

- Replace raw manifest prose with exact helper scaffolds.
- State empty process stdin and private cwd plainly.
- Update canonical tools, report fields, docs, the schema-v4 reference bundle, and targeted negative controls without changing locked task/oracle identities.

### Phase E — Development and holdout qualification

- Tune only on the development families.
- Freeze prompt/helper/contracts and their hashes.
- Run first-shot and bounded-repair holdout profiles.
- Audit every deterministic/judge disagreement before enabling the new contract by default.

## Required tests

Schema and compatibility:

- strict fields, enums, bounds, unique resource IDs/logical paths, and all access combinations;
- result behavior derived from writable resources;
- the dedicated backward reader classifies bare schema-v3 history as legacy without making that shape valid for new responses;
- new responses require `uhm_helper_v1`;
- caches cannot cross prompt/action/program/provider/API-family versions.

Validation:

- reject syntax errors, `sys.stdin`, `sys.__stdin__`, `input()`, direct logical-path opens, unknown resource IDs, and statically proven missing helper use;
- classify unprovable consumption/dynamic access as warnings and test false-positive cases;
- accept helper import aliases and valid stdout/artifact programs;
- avoid executing source during AST validation;
- emit stable diagnostic codes without resolved paths or content.

Runtime:

- piped input to stdout and to an artifact;
- file input to stdout and a new artifact;
- managed `read_write` replacement;
- multiple ID-addressed reads/writes with declaration order changed between variants;
- spaces, Unicode, leading dashes, hidden names, and nested destinations;
- process stdin is EOF and cwd is not the user's cwd;
- child environment contains no provider/cloud credentials;
- helper resources expose only private read/staging paths, not logical destinations;
- `write_only` overwrite requires normal authorization and grants no destination read path;
- `read_write` requires an existing regular input and commits by managed replacement;
- failure commits no managed output; and
- all Plan 10 deadline, signal, overflow, cleanup, staging, recovery, and workspace tests remain green.

State machine and privacy:

- every row in the transition table;
- repair cannot coexist with clarification or revision;
- invalid replacement stops;
- non-TTY never repairs automatically;
- no path exceeds two calls or two executions; and
- unique sentinels in local input, staging paths, launcher contract, and credentials appear in no outbound request, telemetry event, or disallowed history field.

Benchmark:

- production/benchmark validator parity;
- 100% passing reference actions and 100% rejected targeted negatives;
- repair uses only production-visible failure evidence;
- shell, parent-shell, answer, and clarification regression comparisons; and
- family-clustered holdout reporting through Plan 11.

## Acceptance criteria

- The current default provider reaches at least 98% hard-error-free UHM program preflight validity over all holdout program attempts; warnings are reported separately.
- Its first-shot `completed_outcome` is at least 90% across all holdout program attempts and at least 80% in each program stratum, for both task-weighted and equal-family macro point estimates.
- Cumulative-if-approved repair is reported over all holdout program attempts, with zero-exit semantic failures remaining failures, but it is not an adoption gate. Conditional repair success is a separate denominator.
- Any provider later advertised as program-qualified meets Plan 13's class-specific qualification policy independently; a successful helper experiment alone does not grant automatic use.
- Reference actions pass and targeted negative controls fail across every contract diagnostic and oracle family.
- For shell, parent-shell, answer, and clarification strata, the paired equal-family difference lower bound is no worse than the predeclared -5 percentage-point regression margin.
- Zero broad destructive-scope failures occur.
- No request, diagnostic, history field, telemetry event, or repair payload leaks local-only bytes, resolved paths, credentials, or launcher contract contents.
- Historical schema-v3/history fixtures remain readable and are never accepted as new schema-v4 provider output.
- Every interaction stays inside two model calls and two executions.
- Helper/launcher overhead is measured and remains negligible relative to model latency.

If the first-shot program gates are not met, keep the legacy production contract/default while retaining the helper experiment behind development-only evaluation. Do not lower the gates.

## Anti-goals

- Do not add automatic repair, blind retry, a debugging loop, or a larger call/execution budget.
- Do not add a production result oracle or let the benchmark oracle influence repair.
- Do not add a transformation DSL, another language runtime, third-party packages, or dependency installation.
- Do not rewrite arbitrary generated source to conceal contract failures.
- Do not weaken staging, recovery, review, privacy, environment stripping, or resource limits.
- Do not claim the helper, isolated Python flags, AST checks, or process limits form a security sandbox.
- Do not send local-only content or sample files merely to improve generated source.
- Do not qualify a provider because its endpoint accepts the request or because it is fast.

## Primary code areas

- `src/action.rs`
- `src/api.rs`
- `src/prompt.rs`
- `src/program.rs`
- `src/command.rs`
- `src/cache.rs`
- `src/history.rs`
- `src/render/`
- `tests/program_corpus.rs`
- `tests/cli_contract.rs`
- `tests/fixtures/provider-execution-benchmark-v2.json`
- `benchmark/`
- `scripts/provider-bakeoff.py`
- `docs/program.md`
- `docs/behavior-contract.md`
- `docs/configuration.md`
- `docs/privacy.md`
- `docs/local-history.md`
