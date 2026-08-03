# Plan 11 — Make the model benchmark decision-grade

## Purpose and dependency

This plan corrects the integrity, fidelity, and statistical weaknesses found after the first full Plan 9 benchmark. It depends on Plan 9's existing corpus, runner, worker image, and retained JSONL evidence. Its schema, statistics, corpus-structure, and persistence work may proceed alongside Plan 10, but Plan 10 remains the release-critical v0.2.2 track. Extraction from `program.rs` or `shell.rs` waits until Plan 10 lands so the benchmark does not fork execution code that is actively being corrected.

No new provider-selection claim should be made from another paid full run until this plan's preflight, contract-parity, oracle, and statistics gates are complete. The original corpus v1, raw run artifact, and published report remain immutable historical evidence; corrections create corpus v2 and a clearly labeled erratum rather than rewriting the first result.

The benchmark invariant becomes:

```text
provider response
    → wire decoding and schema validation
    → production UHM semantic action validation
    → allowed/preferred route classification
    → fresh keyless offline worker using production execution primitives
    → deterministic outcome oracle
    → secondary blinded judgment
    → append-only resumable evidence and generated report
```

## Evidence behind the plan

The first 120-task, three-trial run was useful precisely because it exposed defects in the measuring system as well as the models:

- Both candidates reached 100% transport and wire-schema validity, but the benchmark's Python validator omitted Rust invariants. At least 50 recorded program proposals would have failed production's embedded-path validation.
- Twelve of 28 Terra and 23 of 55 Cerebras executable route mismatches passed the task oracle when replayed. Shell-versus-program preference was being counted as user-outcome failure.
- Several exact-text oracles rejected valid conventional output, including `uniq -c` padding and a separate branch-plus-short-status representation.
- The 120 task IDs are variants from a much smaller number of semantic families. A family-clustered check widened the paired interval materially while preserving Terra's lead.
- The summary mixed actual judge API calls with synthetic failures, and repeated agreement measured consistency rather than judge neutrality.
- `benchmark/schemas/worker-result.schema.json` is malformed and unenforced, while a stale existing image can silently contain a different baked corpus from the host artifact.
- The model context claimed Python 3.12.3 while the recorded worker executed Python 3.11.

These findings do not invalidate the conclusion that neither candidate qualified under the tested contract. They do mean the next run must measure the production contract more faithfully and label each layer honestly.

## Settled corrections

| Topic | Decision |
| --- | --- |
| Historical evidence | Freeze corpus v1 and its result; publish limitations as an erratum, never edit old scores in place |
| Action acceptance | Report wire-schema validity and production UHM contract validity separately |
| Contract authority | Rust production decoding/validation is authoritative; Python must not duplicate semantic invariants |
| Route scoring | Distinguish an allowed route from UHM's preferred route; execute any policy-allowed shell/program alternative |
| Primary outcome | Deterministic completed outcome over all attempts |
| Statistical unit | Semantic family, with every variant and trial kept inside its family during resampling |
| Judge role | Secondary semantic/safety filter; it never promotes a deterministic failure |
| Judge coverage | Judge every deterministic pass and a fixed diagnostic sample of failures |
| Worker identity | Content-addressed image manifest must match host corpus, schemas, worker, and contracts before any API call |
| Run persistence | One append-only resumable event log, atomically finalized to private JSONL |
| Next full run | Defer until Plan 12's program-contract candidate is ready, so paid evidence measures the intended replacement |

## 1. Make the production contract authoritative

### 1.1 Export and validate one canonical action contract

Factor tool-call decoding, canonical tool definitions, and `ProposedAction::validate` behind a reusable internal Rust interface. Add a benchmark-only helper binary, not a public user workflow, with two bounded JSON operations:

- `describe` emits prompt, action, context-policy, and program-contract versions plus the canonical function-tool definitions.
- `validate` accepts one decoded `{tool, arguments}` envelope and emits either the canonical validated action or a structured rejection code.

Production API parsing and the benchmark helper must call the same Rust functions. Remove `proposal_tools()` and `validate_action()` in `scripts/provider-bakeoff.py` as independent authorities. A provider adapter may transform canonical schemas into a provider's supported wire dialect, but the response must always normalize back through Rust validation before it can be called client-valid or executed.

Keep validation layers explicit:

```text
wire decode/schema check
    → pure canonical Rust action validation
    → runtime-dependent program preflight, when applicable
    → route/policy/review classification
    → execution
```

Adapters return bounded decoded tool calls rather than provider-specific “validated actions.” The canonical layer produces a structurally validated action or rejection. Plan 12's AST/runtime preflight is a later shared layer and preserves the model-authored proposal plus a content-free diagnostic for eligible repair.

Maintain shared accepted/rejected fixtures for:

- unknown fields, missing fields, bounds, enums, and unsafe control bytes;
- invalid executable requirements and parent-shell operand combinations;
- duplicate program resources;
- embedded logical paths under the legacy contract;
- replacement/output mismatches and result-mode mismatches; and
- unsupported runtime or action-schema versions.

Acceptance gate: every fixture produces the same normalized action or stable rejection through production parsing and the benchmark helper.

### 1.2 Reuse production execution primitives

After Plan 10 is complete, build a small Rust benchmark worker from the same shell-spawn, program-manifest, staging, parent-shell rendering, timeout, output-bound, and environment-scrubbing modules used by `uhm`. Keep declarative fixture creation benchmark-specific, but do not reimplement the action runtime in Python.

The benchmark path intentionally omits interactive review, history, telemetry, and recovery capture. It must still use the same validated action representation and low-level execution behavior as production. Dependency injection may supply the benchmark workspace, fixed runtime inventory, and resource limits without adding a second product runtime.

Add parity tests that run the same normalized action through a production test harness and the Docker worker and compare:

- argv, cwd, stdin behavior, and environment allowlist;
- program read/staging manifest construction;
- parent-shell typed rendering;
- exit, signal, timeout, overflow, and bounded diagnostic outcomes; and
- successful artifact commit behavior.

### 1.3 Derive model context from the worker actually used

Remove hard-coded runtime versions from benchmark proposal input. The worker build manifest is the source for Python path/version, isolated/no-site support, shell, architecture, and available tool names. Refuse to start if the context offered to a candidate differs from the selected image.

## 2. Enforce schemas and image identity before spending API calls

### 2.1 Repair and enforce every JSON contract

Repair the worker-result schema and add explicit schemas for:

- corpus v2;
- worker success and worker error envelopes;
- image build manifest; and
- append-only run events.

Use one pinned Draft 2020-12 validator in preflight and tests. Validate the schemas themselves, then validate every loaded corpus, worker response, checkpoint event, and final event. Unknown fields remain rejected. The worker must never return an ad hoc error shape outside the schema.

Acceptance gate: malformed schemas, unknown result fields, invalid worker errors, and truncated event records fail before aggregation; a deliberately malformed worker response fails an integration test.

### 2.2 Content-address the worker contract

Generate an image manifest containing hashes for:

- the fixture bundle and oracle contract;
- worker source and Dockerfile;
- all enforced schemas;
- canonical action description and benchmark helper binary;
- tool/version manifest;
- worker contract version;
- resolved base-image digest and architecture; and
- build timestamp for provenance, not identity.

Define and hash one canonical identity projection that excludes timestamps and other provenance-only fields, then derive the image tag from it. Preflight compares that projection/hash with the host expectation and records the full provenance manifest separately. `--skip-worker-build` may skip a build only when identity matches; otherwise it refuses to run. A stale image and corpus mismatch must be discovered before the first model request.

### 2.3 Keep answers and secrets outside generated-code reach

Do not place reference actions, negative controls, judge rubrics, or expected answers in the candidate container. The container receives only the selected fixture and validated action, then returns bounded raw evidence: exit/signal/timeout, stdout/stderr, before/after manifests and hashes, parent-shell state, limits, and truncation flags. The trusted host runner applies the oracle after the container exits. Do not pretend a supervisor and generated child sharing one container/UID form an answer-secrecy boundary.

Keep API keys exclusively in the trusted runner. Scrub all provider secrets and host sentinel variables from every non-network child environment, including image inspection, build helpers, and report generation.

Add active Docker canaries proving generated code cannot:

- connect to an external or host service;
- read provider keys or a host environment sentinel;
- access the repository, home directory, Docker socket, references, or expected answers;
- write the read-only root filesystem;
- gain capabilities or new privileges; or
- survive the declared wall, output, PID, memory, or workspace limits.

Argument inspection remains a useful unit test but is not containment proof.

## 3. Create corpus v2 around outcomes and semantic families

### 3.1 Separate allowed and preferred routes

Replace `expected_tools` with an explicit route oracle:

```json
{
  "route_oracle": {
    "allowed": ["run_program", "run_shell"],
    "preferred": "run_program",
    "rationale": "Structured transformation is clearer as a bounded program."
  }
}
```

Record independent raw fields:

- `wire_valid`;
- `client_valid` after pure canonical Rust validation;
- `preflight_valid` when runtime-dependent program preflight applies;
- `route_allowed`;
- `route_preferred`;
- `execution_attempted`; and
- `oracle_pass`, containing only the raw deterministic oracle result.

Derive `completed_outcome = client_valid && preflight_valid && route_allowed && oracle_pass`. Do not hide route policy inside `oracle_pass`. A safe executable but disallowed route may be replayed as a diagnostic, but it receives no completion credit; legitimate shell/program alternatives belong in `route_oracle.allowed` instead.

Execute an allowed shell/program alternative and grade the outcome. Keep route choice hard where behavior genuinely changes: prose cannot satisfy executable work, execution cannot satisfy answer-only work, clarification cannot replace a complete request, and persistent shell state must use the typed parent-shell path. Under the current product contract, `--local-input` is program-only; changing that decision is outside this benchmark plan.

### 3.2 Replace hidden formatting with semantic oracles

Use exact text only when the prompt explicitly requires exact formatting. Otherwise prefer structured JSON/CSV comparison, normalized count maps, unordered or ordered record sets, filesystem state, environment state, and semantic answer/clarification assertions.

For prose and clarification, use bounded declarative concept checks for required alternatives, forbidden claims, question count, and the missing-fact class. The LLM judge remains responsible for factual nuance that cannot be encoded reliably; a narrow lexical regex must not silently become product ground truth.

Every task family must include:

- at least two meaningfully different known-valid actions when alternate implementations exist;
- targeted negative actions for its matcher and safety edge cases;
- explicit output-format requirements when exactness is intentional; and
- a recorded manual disposition for any deterministic/judge disagreement found during validation.

### 3.3 Mark families, variants, and product weighting

Add stable `family_id` and `variant_id` fields. A family represents one semantic task design; variants change fixtures or operands without pretending to be independent ideas. Freeze family assignments before inspecting candidate results. Keep category, effects, difficulty, and safety tags.

Separate the task/oracle set from contract-versioned reference-action bundles. Plan 11 owns the corpus-v2 schema, semantic prompts/oracles, family assignments, and development/holdout split. Plan 12 adds a schema-v4 reference bundle after its helper contract freezes; it does not mutate task/oracle identity or reuse the name for a different corpus.

Oracle/reference maintainers may inspect and validate holdout tasks before lock. After recording the holdout hash, provider and prompt tuning sees only the development split. Keep the holdout private until the release decision, then publish it with the report and rotate a new private holdout for the next decision.

Report three distinct views:

1. Fixed-corpus task-weighted completion.
2. Equal-family macro completion.
3. Optional product-usage-weighted completion from a separately versioned, content-free weight file.

If representative usage weights do not exist, report that view as unavailable rather than inventing weights. Programs may remain deliberately prominent as a stress stratum, but that weighting must not be described as observed production usage.

## 4. Use family-aware paired statistics

Keep all three trials, but treat them as repeated attempts rather than independent samples. Freeze this primary estimator before candidate inspection:

1. Average trials within each task/variant.
2. Average task/variant rates within each family.
3. Compute the paired candidate difference for each family.
4. Average family differences with equal family weight.
5. Bootstrap family IDs with replacement and recompute that equal-family mean.

Do not pool cloned tasks in a way that gives a larger family more primary weight. Task-weighted completion remains descriptive; if it receives an interval, label and test a separate cluster-ratio estimator explicitly.

The primary comparison is a paired family-clustered bootstrap with 10,000 seeded resamples. Report:

- effect size and family-clustered 95% interval;
- task- and family-weighted completion;
- per-stratum rates and intervals;
- 0/3, 1/3, 2/3, and 3/3 task consistency counts; and
- transport, wire validity, client validity, allowed-route, preferred-route, timeout, and broad-scope rates separately.

Keep task-level McNemar only as a labeled secondary diagnostic. Add a family-level paired permutation or sign test if a discrete secondary test is needed. Synthetic tests must prove that cloning variants can change the task-weighted descriptive rate but cannot manufacture narrower family-level significance.

Power analysis uses pilot family discordance. Do not lower gates or count trials/variants as extra independent evidence when a comparison is underpowered.

## 5. Keep judging useful, blinded, and correctly accounted

Separate these counters and artifacts:

- actual judge API calls;
- synthetic invalid/disallowed-route outcomes;
- judge transport or format errors and their one permitted retry;
- calibration repeats; and
- independent audit judgments.

Report actual-judge means only across real judge calls. Synthetic failures are never labeled API calls or included in an actual-judge mean.

For executable tasks, `completed_outcome` remains primary. Report judge `pass`, `minor`, `fail`, and `critical_error` separately; one judge is not a silent hard veto. A judge critical error on an oracle-passing action requires independent adjudication before qualification. The versioned qualification policy defines an audited critical-error ceiling.

Answer and clarification tasks have no execution oracle and are a separate semantic-quality stratum. Predeclare whether `pass` and `minor` are acceptable, require no critical error, and apply the independent audit below. Do not mix them into executable completion without an explicit versioned weighting rule.

Judge every deterministic pass because the first run showed that successful fixtures can still hide unsafe operands, locale dependence, portability defects, or factual contradictions. For deterministic failures, judge only a seeded stratified diagnostic sample plus every deterministic failure in a safety-critical task family. Keep those diagnostics out of pass-rate denominators.

Repeat the fixed 12-item calibration slice to measure consistency. Independently audit every deterministic/judge disagreement up to a cap of 20; if fewer exist, fill a 20-item blinded audit with a seeded stratified sample of passes and failures. Record reviewer/judge identity, rubric, disposition, and any corrected oracle. Do not describe repeat agreement as independent validation.

Record candidate and judge input/output tokens separately. Raw usage is authoritative; an optional pricing snapshot may estimate cost but must include source and effective date.

## 6. Make runs resumable and reports reproducible

Use one append-only event stream:

```text
run_started
candidate_completed
judgment_completed
calibration_completed
summary_computed
run_completed
```

Compute a run fingerprint over corpus, action/program contracts, helper and runner hashes, worker image identity, candidate and judge providers/models/endpoints, reasoning and token settings, judge prompt, trials, sampling policy, and seed.

`--resume` accepts only an exact fingerprint match and skips completed candidate/judge keys. Duplicate or conflicting keys fail closed. On completion, fsync the event file, write and fsync the computed summary, atomically rename the final `0600` artifact, then fsync the parent directory.

Record enough non-secret provenance to explain a result:

- UTC start/end and attempt ordinal;
- git commit and dirty-state flag;
- runner, helper, schema, corpus, judge-prompt, and worker hashes;
- Docker, kernel, and architecture metadata;
- provider/API family, endpoint identity, model, resolved fingerprint when returned, and sanitized request ID;
- reasoning, token, streaming, retry, and timeout settings; and
- full worker tool manifest.

Generate HTML and a redacted machine-readable summary directly from the finalized event artifact. Embed its SHA-256, task-family count, actual/synthetic judge accounting, token totals, trial consistency, and corpus-specific limitations. Never hand-copy headline figures.

## Implementation sequence

### Phase A — Stop integrity drift

- Repair and enforce schemas.
- Add production contract export/validation.
- Add image fingerprint preflight and actual-runtime context.
- Move oracle application to the trusted host runner.
- Implement the frozen equal-family estimator and synthetic clone tests.
- Add stale-image, malformed-result, and contract conformance tests.

This is the minimum trustworthy milestone and gates even a corrected provider smoke. Resume, HTML polish, and optional second-judge automation do not delay it.

### Phase B — Restore execution and containment fidelity

- Factor and reuse production execution primitives in the worker.
- Remove candidate-readable references and answers.
- Add active containment and limit canaries.

### Phase C — Define and lock corpus v2

- Add route oracles, semantic matchers, family/variant IDs, alternate references, and targeted negatives to the development set.
- Audit all v1 disagreements and carry forward only explicitly dispositioned tasks.
- Validate and hash the holdout task/oracle set before candidate tuning; add the active-contract reference bundle after Plan 12 freezes that contract.
- Record the v1 report erratum without changing v1 evidence.

### Phase D — Correct statistics, judging, and persistence

- Add family-clustered comparisons and consistency reporting.
- Split actual/synthetic judge accounting and failure sampling.
- Add run fingerprints, exact resume, atomic finalization, and generated reports.

### Phase E — Validate without a premature full run

- Run all offline reference, negative, parity, schema, statistics, resume, and containment tests.
- Run a 12-task provider smoke only after every preflight passes.
- Defer the next full paid comparison until Plan 12 supplies the program contract intended for selection.

## Expected outcomes

- A quality rate means the requested outcome happened through an action UHM itself would accept.
- Shell-versus-program preference remains measurable without disguising correct outcomes as failures.
- Confidence intervals reflect semantic diversity rather than the number of fixture clones.
- Judge results are useful diagnostics with honest API counts, limitations, and cost.
- Interrupted runs resume safely, and every published figure is derivable from private evidence.
- The Docker boundary remains practical and actively tested without claiming VM-grade isolation.

## Definition of done

- Production and benchmark action validation share one Rust authority and pass the same conformance vectors.
- Benchmark action execution reuses production shell, program, and parent-shell primitives.
- All JSON schemas are valid, strict, tested, and enforced.
- Host and image fingerprints match before any API call; stale images are rejected.
- Candidate code cannot read provider keys, host files, benchmark references, or expected answers in active canary tests.
- Corpus v2 has explicit allowed/preferred routes, semantic oracles, family/variant IDs, alternate valid references, and targeted negatives.
- Wire/client/preflight validity, route allowance/preference, execution attempt, raw oracle result, derived completion, and judge verdict are separate result fields.
- Primary intervals resample semantic families, and variant cloning cannot create false significance.
- Actual judge calls, synthetic outcomes, errors, calibration, tokens, and cost reconcile exactly.
- Checkpoint resume is fingerprinted, idempotent, and equivalent to an uninterrupted seeded run.
- Final artifacts are private, fsynced, atomically finalized, and generate their own HTML/redacted summaries.
- The v1 artifact remains unchanged and its limitations are documented.
- All offline tests and a provider smoke pass before Plan 12's full qualification run.

## Anti-goals

- Do not add benchmark execution to normal `uhm` user workflows.
- Do not build a general hostile-code sandbox or claim VM-grade isolation.
- Do not add Kubernetes, a service, queue, database, dashboard, or distributed runner.
- Do not permit task-specific grader code or make an LLM the primary metric.
- Do not relax quality gates to make a provider qualify.
- Do not count trials, fixture variants, or templated prompts as independent tasks.
- Do not retrofit new oracles into corpus v1 or alter its stored scores.
- Do not publish raw proposals, private/local fixture contents, event logs, or credentials. A synthetic frozen holdout may be published after its release decision and then retired from future tuning.
- Do not run another full paid comparison merely to test runner plumbing.

## Primary code areas

- `src/action.rs`
- `src/api.rs`
- `src/prompt.rs`
- `src/program.rs`
- `src/shell.rs`
- `src/lib.rs`
- `src/bin/uhm-bench-contract.rs`
- `src/bin/uhm-bench-worker.rs`
- `scripts/provider-bakeoff.py`
- `scripts/provider-benchmark-report.py`
- `benchmark/generate_corpus.py`
- `benchmark/test_benchmark.py`
- `benchmark/test_containment.py`
- `benchmark/docker/Dockerfile`
- `benchmark/schemas/`
- `tests/fixtures/action-validation-cases-v1.json`
- `tests/fixtures/provider-execution-benchmark-v2.json`
- `docs/model-selection.md`
