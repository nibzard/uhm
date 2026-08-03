# Plan 13 — Add provider adapters and evidence-gated model policy

## Purpose and dependency

This plan has two release gates. Gate A moves the current OpenAI Responses implementation behind a narrow provider-neutral boundary, completes provider-safe secrets/cache/history work, and then adds Cerebras Chat Completions as fixed opt-in. Gate B adds evidence selection and bounded transport fallback only after the exact provider/model and request class qualify on Plan 11's corrected holdout under the active production program contract from Plan 12.

Gate A depends on Plan 10's outbound/privacy hardening and Plan 11's canonical contract-validation seam; it should exist before Plan 12's cross-provider holdout so qualification exercises production adapters. Gate B depends on Plans 11–12 and the completed Gate A. OpenAI with the current default model remains backward compatible throughout.

Gate A normalizes both providers into whichever canonical action/program contract is active when it merges: the retained schema-v3 contract before a successful Plan 12 adoption, or schema v4 afterward. That makes fixed Cerebras opt-in independently shippable without treating it as program-qualified; automatic use still waits for Gate B evidence against the active contract.

Current evidence does not justify an automatic Cerebras fast path:

- Cerebras was much faster but completed 0/144 expected program attempts under the original contract.
- Neither candidate passed the full qualification gates.
- `run`/bare intents cannot reliably be labeled “simple shell” versus “program” before a model call, so strong results on a narrow post-hoc stratum are not enough for automatic dispatch.

Endpoint compatibility is therefore an adapter concern, not proof of interchangeable capability.

## Product invariant

```text
one disclosed provider policy
    → at most one initial provider attempt
    → one canonical UHM action validator
    → optional bounded fallback/replacement within the existing two-call budget
    → one accepted action and normal local policy/execution
```

No provider can change the action schema, safety classification, local execution authority, review behavior, or conversation limits.

## Settled rollout decisions

| Topic | Decision |
| --- | --- |
| Default | OpenAI plus the existing default model remains unchanged until an explicit release decision |
| First Cerebras exposure | Explicit fixed opt-in only after adapter, credential stripping, cache/history isolation, disclosure, and live smoke are complete |
| Provider inference | Never infer provider from a model-name prefix |
| Endpoints | Two fixed built-in HTTPS endpoints initially; no arbitrary `base_url` |
| Keys | Provider-specific environment variable or existing private `0600` secrets file; never YAML |
| Canonical contract | Provider adapters return the same bounded decoded tool-call type; one provider-neutral Rust layer validates it |
| Streaming | Preserve OpenAI SSE; Cerebras starts buffered until its adapter has independent stream coverage |
| Fallback | Off by default, explicitly configured/disclosed, pre-execution only, and inside two total model attempts |
| Semantic repair | Remains user-triggered under Plan 12; provider policy may choose its destination but may not trigger it silently |
| Automatic selection | Exact evidence entry required for the request class and current prompt/schema/adapter/policy versions |
| User authority | Explicit `--provider`/`--model` may run an unqualified model with a concise status warning; UHM does not silently override it |

## 1. Introduce one narrow normalized provider layer

Add:

- `src/provider/mod.rs`: provider IDs, invocation, wire capabilities, decoded response, typed errors, and dispatch.
- `src/provider/openai.rs`: current Responses body, parser, and SSE behavior moved without semantic change.
- `src/provider/cerebras.rs`: Chat Completions body and buffered parser.
- `src/model_selection.rs`: fixed/evidence resolution, qualification checks, attempt budget, and fallback decisions.
- `src/capabilities.rs`: checked-in qualification manifest reader and compatibility matching.

Keep `src/api.rs` temporarily as a facade if that reduces churn. It must delegate to the selected adapter rather than retain an independent OpenAI-only path.

The adapter boundary is conceptually:

```rust
trait ProviderAdapter {
    fn id(&self) -> ProviderId;
    fn api_family(&self) -> &'static str;
    fn endpoint(&self) -> &'static str;
    fn credential_env(&self) -> &'static str;
    fn capabilities(&self) -> WireCapabilities;
    fn build_request(&self, invocation: &Invocation) -> Result<HttpRequest, ProviderError>;
    fn parse_response(&self, response: HttpResponse) -> Result<ProviderResponse, ProviderError>;
}
```

`ProviderResponse` contains:

- one bounded normalized `DecodedToolCall`, not a validated action;
- provider and API family;
- requested and resolved model identifiers when returned;
- sanitized provider request ID, finish/status reason, and usage;
- adapter contract version; and
- the bounded raw provider envelope only for the existing private response cache.

The provider-neutral contract layer turns `DecodedToolCall` into `ValidatedAction` or `RejectedAction`; Plan 12's runtime-dependent preflight follows afterward. Adapter code must not add provider-specific action semantics. Empirical provider/model action eligibility belongs only in `QualificationProfile`; `WireCapabilities` covers protocol facts such as streaming, reasoning fields, schema dialect, and limits.

Use a typed `ProviderErrorKind` rather than matching error strings:

- `credential`;
- `auth`;
- `rate_limited`;
- `transient`;
- `timeout`;
- `request_rejected`;
- `refused`;
- `incomplete`;
- `malformed`; and
- `unsupported_capability`.

Sanitize provider-controlled messages before terminal output, history, or logs. Authorization must not implement `Debug` or appear in error values.

## 2. Preserve the two wire protocols honestly

### 2.1 OpenAI Responses

Move current behavior behind the adapter without changing request or response semantics:

- fixed `https://api.openai.com/v1/responses`;
- `instructions`, `input`, canonical strict tools, required tool choice, no parallel calls, and `store: false`;
- `max_output_tokens`, supported reasoning effort, and optional SSE;
- exactly one completed function call, with reasoning items permitted;
- plain messages, refusals, multiple calls, incomplete output, unknown items, and oversized responses rejected; and
- returned strict-tool metadata validated where the API supplies it.

Golden tests must prove the no-op extraction keeps existing request bodies, parsing, streaming, response bounds, and errors behavior-compatible.

### 2.2 Cerebras Chat Completions

Add the fixed built-in endpoint `https://api.cerebras.ai/v1/chat/completions`:

- `messages` contains one developer instruction and one user proposal input.
- Convert each canonical tool to Chat Completions `{type, function}` form.
- Require exactly one tool call and disable parallel tool calls.
- Send `max_completion_tokens` and reasoning fields only when the adapter capability declaration supports them.
- Remove only wire-schema keywords the endpoint rejects; retain the canonical schema locally and run full Rust validation afterward.
- Start with `stream: false` and parse exactly one choice containing exactly one `message.tool_calls` entry with JSON arguments.
- Do not require Responses-only echoed tool metadata.

Recorded fixtures cover valid output, no/multiple choices, no/multiple calls, malformed arguments, plain text, refusal, incomplete finish, rate limit, server error, oversized body, and control characters. Live smoke remains explicitly invoked and key-dependent; CI is offline.

### 2.3 Share bounded HTTP mechanics

Refactor the transport around bounded request/response values so mock tests can inspect URL, safe headers, body, status, and reader without a real network call. Preserve TLS, proxy discovery, deadlines, response caps, and bearer authentication from Plan 10.

Only a provider adapter may pair its credential with its fixed endpoint. A custom compatible URL is deferred because it can redirect a trusted provider key to an arbitrary host.

## 3. Add backward-compatible provider configuration

Gate A ships only the minimal fixed-mode YAML surface:

```yaml
provider: openai                 # openai | cerebras
model: gpt-5.6-terra
```

Gate B may add:

```yaml
selection:
  mode: fixed                    # fixed | evidence
  alternate: null
  fallback_on: []
  # alternate:
  #   provider: openai
  #   model: gpt-5.6-terra
  # fallback_on: [rate_limited, transient, timeout, incomplete, malformed]
```

The top-level provider/model and optional alternate are the only candidates in the first evidence-mode implementation. The reviewed manifest may select either as the initial candidate; `fallback_on` separately authorizes the other candidate only after a listed pre-proposal transport failure. Do not add a general routing language.

Resolution precedence becomes:

1. Built-in defaults.
2. Strict YAML.
3. `UHM_PROVIDER` and `UHM_MODEL`.
4. `--provider` and existing `--model`.

Preserve `OPENAI_MODEL` as a compatibility alias only when the selected provider is OpenAI and `UHM_MODEL` is absent. `--model` remains a bare provider-specific model ID and never changes provider implicitly.

`uhm config show` reports provider, model, selection mode, alternate, fallback triggers, source of each value, and qualification status without checking the network. `config check` rejects unknown providers, invalid triggers, unsupported settings, keys in YAML, and an alternate identical to the primary. Before Gate B, `selection` remains unknown/unsupported rather than accepting settings that do nothing.

Keep shared token and reasoning settings until real provider differences require scoped overrides. Each adapter validates or deliberately omits unsupported optional parameters; it must not silently reinterpret them.

## 4. Resolve and contain credentials per provider

Change secret resolution to accept a `ProviderId`:

- OpenAI: `OPENAI_API_KEY`, then `OPENAI_API_KEY=...` in the existing private secrets file.
- Cerebras: `CEREBRAS_API_KEY`, then `CEREBRAS_API_KEY=...` in the same private secrets file.

The secrets file remains required to be `0600`; key literals remain forbidden in `config.yaml`. Environment takes precedence so ephemeral CI and one-off invocations remain simple.

Extend every shell, program, benchmark-helper, telemetry-helper, and other child environment allowlist/denylist to remove all built-in provider credentials, not merely the selected key. Add sentinels proving neither key reaches child stdout, stderr, process environment, history, cache metadata, telemetry, or debug errors.

`uhm doctor` checks the selected provider by default. Add an explicit all-providers mode that reports key presence/masked identity, adapter capabilities, fixed endpoint, and connectivity without printing a secret or raw authorization failure.

Before the first outbound request, disclosure names the selected provider and hostname. Bind outbound authorization to the notice revision plus the exact disclosed endpoint set. Switching provider or enabling a cross-vendor alternate/fallback requires a new disclosure before either request. Merely finding another key never authorizes a vendor switch.

## 5. Version cache and local provenance by provider

Upgrade cache provenance to include:

- provider ID and API family;
- exact model and adapter contract version;
- canonical prompt, action, program, policy, and context versions;
- selection-policy and qualification-manifest versions/hashes;
- endpoint identity, excluding credentials;
- effective generation parameters; and
- all current semantic request inputs.

A cache hit is parsed only by the same adapter. Same-named models at different providers cannot collide. Resolve the initial candidate first, then look up only its cache. The remaining candidate's cache is considered only after an actual configured fallback trigger, so cached output cannot silently alter selection.

Add backward-compatible optional local-history fields for:

- provider/API family;
- requested/resolved model;
- adapter and evidence versions;
- selection mode;
- provider-attempt index;
- fallback reason; and
- cache state.

Record every outbound provider attempt at the configured local detail level and identify which attempt supplied the accepted action. Use append-only per-attempt proposal/events and an accepted-proposal reference so a fallback or repair cannot overwrite prior evidence. Do not add provider/model identity to telemetry without a separate privacy decision and documentation change.

Follow-up clarification, revision, or repair stays on the accepted proposal's provider/model by default. A configured replacement destination may switch it only through the bounded policy below, and the switch is recorded.

## 6. Keep selection conservative and observable

### 6.1 Fixed mode

Fixed mode always uses the explicit provider/model. It is the default. OpenAI behavior remains unchanged for existing users; Cerebras is labeled experimental and not program-qualified until new evidence says otherwise.

Known qualification status is shown by `config show`, `config check`, and verbose request provenance. It never silently blocks an explicit user's chosen model unless the adapter cannot express the requested canonical contract.

### 6.2 Evidence mode

Evidence mode may select between the configured primary and alternate only when a checked-in qualification entry exactly matches:

- provider, API family, endpoint identity, exact model/resolved fingerprint;
- prompt, action, program, context, adapter, and selection-policy versions;
- benchmark corpus, worker, runner, and evidence-manifest hashes;
- request class and permitted action types; and
- freshness policy.

The runtime does not recalculate statistical winners. The reviewed qualification manifest records the selected profile for each eligible request class after applying the frozen quality rule and latency/cost tie-break. Runtime selection verifies compatibility hashes, intersects that decision with the two configured candidates, and uses the manifest-selected candidate initially. If no exact selected profile is configured, evidence mode returns an actionable unavailable result; the user may explicitly choose fixed mode, but the selector does not improvise.

Selector inputs must be known before the model call: explicit CLI route (`ask`, `explain`, `run`, `repair`, `recover`, or bare/auto), stdin presence/local-only/declared format, follow-up kind, and runtime availability. Do not use another model as a router.

`ask`/`explain` requires prose qualification. `run`/bare auto requires qualification across every action type that the class can produce. A provider qualified only for parent-shell or simple writes is not automatically eligible for general `run`, because UHM cannot reliably know that stratum before inference. Explicit provider choice remains the practical fast-model option until a conservative pre-call class is proven.

After parsing, enforce the selected evidence profile's permitted action types. An action outside that profile is not executed and may be offered as a user-triggered replacement; it is never silently rewritten.

### 6.3 Bounded fallback

Fallback is absent/off by default. Gate B uses the configured alternate as the only possible destination and `fallback_on` as its trigger allowlist.

Automatic fallback is allowed only before any valid proposal has been accepted and only for pre-execution provider failures such as rate limiting, transient network/server failure, timeout, incomplete response, or malformed response. Missing credentials, authentication failure, and policy rejection fail closed by default because they usually indicate configuration rather than provider availability.

A syntactically decoded but UHM-invalid action follows Plan 12's user-triggered repair/replacement path; it does not silently fall back. No fallback occurs after local execution, parent-shell mutation, a merely low-quality-looking valid proposal, or a runtime LLM judgment.

The persistent fallback configuration is a deliberate product-contract change and the user's authorization for a possible second vendor request. Update the behavior contract, privacy disclosure, and design rationale accordingly. It does not expand the global budget:

- at most two total provider/model calls across transport fallback, clarification, revision, or repair;
- one accepted action at a time;
- no candidate fan-out or race; and
- no second fallback.

The provider layer returns `attempts_consumed`; the command budget increments for every outbound POST, including failed attempts, before permitting clarification, revision, or repair. Tests count actual transport calls, not accepted proposals.

If fallback consumes call two, later clarification/repair is unavailable and the UI states why. If that final response requests clarification, report that the question cannot be continued within the exhausted call budget and end the job without pretending it is resolved. An out-of-profile first proposal followed by user replacement likewise leaves no later clarification/repair slot.

## 7. Bind automatic use to corrected holdout evidence

Use Plan 11's family-aware corpus v2 and Plan 12's first-shot/bounded-repair profiles. Maintain a development set for prompt/adapter work and an untouched holdout for release qualification.

Each checked-in qualification entry records:

- all compatibility hashes listed above;
- evaluation date, trials, semantic families, and seed;
- transport, wire, UHM-contract, allowed-route, preferred-route, deterministic, and judge rates;
- per-eligible-stratum rates and intervals;
- broad destructive-scope failures;
- latency and raw token usage; and
- report/artifact hashes and reviewer disposition.

Before revealing holdout candidate results, freeze and hash `model-qualification-policy-v1.json`. Both runner and product selector consume it. Version 1 fixes:

- at least 30 independent semantic families and three trials per automatically selected request class;
- at least 300 candidate calls across the full holdout for transport/adapter reliability;
- Plan 11's equal-family macro estimator and 10,000 paired family-bootstrap resamples;
- transport success at least 99% with a 95% Wilson lower bound of 97%;
- wire decoding and canonical client validity each at least 98%, with lower bounds of 95%;
- first-shot `completed_outcome` at least 90% with a family-bootstrap lower bound of 80%;
- every eligible executable stratum at least 80% with a lower bound of 65%;
- non-executable semantic acceptability (`pass` or `minor`, no critical error) at least 95% with a lower bound of 85%;
- paired quality non-inferiority lower bound no worse than -5 percentage points versus the current default;
- judge repeat agreement at least 10/12, completion of Plan 11's independent audit, and zero adjudicated critical errors;
- zero broad-scope failures across at least 60 independent targeted destructive-scope cases, with the one-sided 95% zero-event upper bound below 5%; and
- evidence age no greater than 180 days, in addition to exact compatibility hashes.

Define a broad-scope failure mechanically from task-specific allowed paths/effects and deterministic before/after evidence, followed by blinded audit; a judge label alone is not the detector. If the minimum families, cases, calls, power, or intervals are unavailable, the class is inconclusive and ineligible rather than waived.

The reviewed manifest selects deterministically. If a paired interval is wholly above zero, select the higher-quality qualified candidate. Otherwise a qualified alternate may win only when its lower bound is at least -5 points and its p50 latency is at least 20% lower; if not, retain the current default. Cost is reported but is not a version-1 tie-break.

Any prompt, schema, adapter, worker, or selection-policy change invalidates the evidence entry. Qualification binds to the active production program contract: schema v4 if Plan 12 passes, otherwise the retained schema-v3 contract.

Evidence mode requires an immutable requested model revision, or a stable provider-returned fingerprint that can be checked after every response and before execution. A mismatch rejects the response and may use one configured transport fallback. A mutable alias without a verifiable resolved identity remains fixed-mode-only.

Do not tune on the holdout, lower gates, or promote Cerebras general `run` support merely because shell-write and parent-shell development strata were strong.

## Implementation sequence

### Phase A — No-op OpenAI extraction

- Add provider-neutral invocation, response, error, transport, and provenance types.
- Move current Responses behavior behind the OpenAI adapter.
- Prove request, parse, SSE, cache, and CLI compatibility with golden/current tests.

### Phase B — Implement Cerebras without exposure

- Add buffered Chat Completions conversion/parser, capabilities, fixed endpoint, key resolution, doctor support, config, and CLI.
- Keep the provider unavailable in release builds until Phase C completes.
- Run offline adapter conformance.

### Phase C — Complete Gate A and release fixed opt-in

- Version cache keys/envelopes and history provenance.
- Strip all provider credentials from every child.
- Add cross-vendor disclosure and secret sentinel coverage.
- Run an explicit live smoke, then expose fixed Cerebras opt-in with fallback/evidence mode absent and OpenAI default unchanged.

### Phase D — Corrected development evaluation

- Integrate both production adapters with Plan 11's runner.
- Evaluate Plan 12's program contract and provider-specific capabilities on development families.
- Fix product contracts rather than special-casing benchmark tasks.

Phases A–C are Gate A and may complete before Plan 12 qualification. Phases D–F are Gate B and depend on Plan 12's active-contract decision.

### Phase E — Evidence and bounded fallback

- Implement qualification manifests and fixed/evidence resolution.
- Add pre-execution typed transport fallback, attempt-budget accounting, and exact provenance.
- Run the untouched holdout and check in evidence only after independent review.

### Phase F — Default decision

- Publish the generated report and compatibility hashes.
- Enable evidence mode only for request classes that qualify.
- Consider a default change only through a separate explicit release decision; otherwise retain OpenAI fixed default.

## Required tests

Adapters and transport:

- golden request bodies and response fixtures for both API families;
- exact canonical tool conversion and capability-field omission;
- zero/multiple choices/calls, plain text, refusal, malformed JSON, unknown fields, incomplete status, bounds, and controls;
- fixed endpoint and correct authorization through fake transport without rendering keys;
- categorized 401, 429, 5xx, timeout, and truncated response behavior; and
- OpenAI buffered/SSE parity plus Cerebras buffered behavior.

Configuration and credentials:

- defaults, YAML/env/CLI precedence, legacy `OPENAI_MODEL`, and strict unknown-field rejection;
- provider never inferred from model name;
- no key or arbitrary endpoint accepted in YAML;
- environment/secrets-file precedence and permission checks for both providers;
- selected/all doctor behavior; and
- disclosure markers bound to the exact primary/alternate endpoint set before any request.

Cache, history, and privacy:

- no collision across provider, API family, endpoint identity, model, adapter, prompt/action/program/policy/evidence versions;
- old-cache miss/migration, corruption, TTL, and private atomic writes;
- every attempt and fallback recorded without secrets or raw provider errors;
- both credentials absent from shell/program/helper environments; and
- no provider/model telemetry expansion without an explicit schema decision.

Selection and state machine:

- fixed explicit model, qualified evidence choice, unknown/stale evidence, and model-fingerprint change;
- request-class/action-profile eligibility;
- every fallback trigger, disallowed trigger, missing alternate key, and maximum-attempt boundary;
- no fallback after accepted proposal or execution start;
- actual outbound POST count matches `attempts_consumed` and the command budget;
- fallback consuming the global second turn blocks clarification/repair and makes a final clarification terminal/unresolved; and
- explicit unqualified provider remains user-authorized with accurate status.

Benchmark and release:

- production adapter output passes the shared action validator;
- corrected development corpus and untouched holdout remain separated;
- qualification hashes bind every compatibility input;
- current OpenAI default regression suite remains green; and
- optional live smokes require explicit flags and never run in ordinary CI.

## Acceptance criteria

- Existing installs and configuration behave exactly as OpenAI/default-model fixed mode unless provider selection is explicitly changed.
- OpenAI and Cerebras adapters normalize into the same bounded decoded-call type, which the shared layer validates, and pass all golden/conformance fixtures.
- Only each adapter's fixed HTTPS endpoint receives its corresponding key.
- No credential appears in child environments, cache/history/telemetry, benchmark artifacts, errors, or debug output.
- Cache entries cannot collide or parse across providers, API families, endpoints, models, or contract versions.
- Every provider attempt records safe local provenance and identifies the accepted action.
- Cerebras is available as explicit buffered fixed opt-in only after all Gate A privacy/cache/history/disclosure tests and live smoke pass; fallback and evidence mode remain absent/off.
- Configured fallback is disclosed, typed-error-driven, sequential, pre-execution only, and bounded to two total calls.
- Evidence mode refuses stale, unknown, mismatched, or unqualified entries.
- Current Cerebras program/general-run capability remains ineligible until the corrected untouched holdout clears every gate.
- Speed never overrides a failed quality or destructive-scope gate.

## Anti-goals

- Do not add arbitrary `base_url` or generic “OpenAI-compatible” endpoints in the first provider release.
- Do not put keys or credential-variable names in YAML.
- Do not infer providers from model strings or silently switch because another key exists.
- Do not build a public provider plugin SDK before two built-in adapters stabilize the seam.
- Do not use a runtime LLM router/judge, fan out candidates, race providers, or fall back after side effects.
- Do not automatically rewrite invalid source or disguise contract failures as provider availability.
- Do not tune on the holdout, count fixture clones as independent, or lower qualification gates.
- Do not let provider reputation or benchmark scores bypass canonical action validation and local safety policy.

## Primary code areas

- `src/provider/`
- `src/api.rs`
- `src/http.rs`
- `src/sse.rs`
- `src/model_selection.rs`
- `src/capabilities.rs`
- `model-qualification-policy-v1.json`
- `model-qualification-manifest.json`
- `src/config.rs`
- `src/secret.rs`
- `src/cache.rs`
- `src/history.rs`
- `src/doctor.rs`
- `src/command.rs`
- `src/shell.rs`
- `src/program.rs`
- `src/main.rs`
- `config.example.yaml`
- `tests/cli_contract.rs`
- `benchmark/`
- `scripts/provider-bakeoff.py`
- `docs/configuration.md`
- `docs/behavior-contract.md`
- `docs/model-selection.md`
- `docs/privacy.md`
- `docs/troubleshooting.md`
