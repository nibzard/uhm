# Model selection benchmark

The release default is selected with `scripts/model-bakeoff.sh`, using the versioned corpus in `tests/fixtures/result-first-eval.json`. The harness makes fresh, stateless Responses requests with minimal context and `--dry-run`, so it never executes a proposal. It records strict structured-action validity, route correctness, and wall time to the first complete validated proposal. Correction rate is measured separately in interactive acceptance testing because the offline harness cannot invent user feedback responsibly.

A candidate qualifies only if it supports all four strict tools and achieves 100% structured validity and route correctness on this small release gate. Among qualifying candidates, the lowest median proposal time wins. These measurements compare models on one machine and network path; they are not a general latency promise.

Run:

```sh
scripts/model-bakeoff.sh | tee target/model-bakeoff.jsonl
```

The checked-in result below was measured on 2026-08-01. Raw output is intentionally kept under `target/`, not committed, because repeated measurements age quickly.

| Model | Structured valid | Route correct | Median complete proposal | Qualified |
|---|---:|---:|---:|---:|
| `gpt-5.6-luna` | 6/6 | 6/6 | 2,247 ms | yes |
| `gpt-5.6-terra` | 6/6 | 6/6 | 1,824 ms | yes |
| `gpt-5.6-sol` | 6/6 | 6/6 | 2,656 ms | yes |

All 18 initial proposals passed, so correction-needed rate was 0/6 for each candidate and the interactive correction path was not invoked. Terra was the fastest qualifying model at the median and is therefore the v0.1 default. Observed p95 wall times were 2,776 ms (Luna), 4,466 ms (Terra), and 5,290 ms (Sol); the six-case sample is deliberately a release comparison, not a stable percentile estimate.

## Cross-provider end-to-end benchmark

The original corpus-v1 run and report are frozen historical evidence. Its published conclusion—that neither tested candidate qualified—still stands, but its exact route score, program-contract validity, confidence interval, and judge-call totals are not decision-grade: the runner duplicated weaker validation than production, treated preferred routes as required routes, used brittle formatting oracles, resampled task variants rather than semantic families, and mixed synthetic judgments with actual API calls. Do not revise the v1 artifact or reuse its headline figures for a new provider-selection claim.

`scripts/provider-bakeoff.py` now uses the 120-task corpus v2 in `tests/fixtures/provider-execution-benchmark-v2.json`. Candidate calls go through the same compiled Rust provider adapters and canonical validator as the product; judge calls remain a separate blinded evaluation client. The run fingerprint binds the provider helper, policy, manifest, corpus, runner, schemas, worker, and action contract. It reports wire validity, canonical UHM contract validity, runtime preflight, allowed and preferred routes, execution attempts, raw deterministic oracle outcomes, and derived completion separately. The primary paired interval resamples 52 semantic families; task weighting remains descriptive.

Generated shell, Python, and parent-shell actions run in a fresh Docker worker with no network, API keys, host mounts, Docker socket, or writable image filesystem. The worker is non-root, has all capabilities dropped, and gets bounded `/tmp` and `/work` tmpfs filesystems. Docker is a practical containment boundary, not VM-grade isolation. Candidate and judge API calls happen only in the trusted host runner.

API keys are environment-only:

```sh
export OPENAI_API_KEY='...'
export CEREBRAS_API_KEY='...'
```

Build and validate the worker once:

```sh
benchmark/build-worker.sh
UHM_BENCH_DOCKER_TESTS=1 python3 -m unittest benchmark/test_benchmark.py benchmark/test_containment.py
```

Run the 12-task, one-trial smoke profile in first-shot mode:

```sh
scripts/provider-bakeoff.py \
  --candidate openai:gpt-5.6-terra \
  --candidate cerebras:gpt-oss-120b \
  --judge openai:gpt-5.6-sol \
  --profile smoke \
  --program-profile first-shot \
  --output target/provider-smoke.jsonl
```

For development measurement of the one user-approved replacement ceiling, repeat the same frozen tasks with `--program-profile bounded-repair`. The runner offers repair only for a semantic contract failure or a production-visible nonzero, timeout, or overflow outcome. It never gives the replacement prompt an oracle failure, expected answer, fixture metadata, child output, or resolved host path. First-shot completion and cumulative-if-approved completion remain separate measures.

Only the full profile is suitable for model-selection claims. It fixes all 120 tasks and three trials per candidate:

```sh
scripts/provider-bakeoff.py \
  --candidate openai:gpt-5.6-terra \
  --candidate cerebras:gpt-oss-120b \
  --judge openai:gpt-5.6-sol \
  --profile full \
  --program-profile first-shot \
  --output target/provider-bakeoff.jsonl
```

The runner builds the worker when its image tag is missing; use `--skip-worker-build` to require an exact matching prebuilt image. `--profile custom`, `--task-id`, `--task-count`, and `--trials` support debugging subsets. `--resume` continues only an exact run-fingerprint match. The finalized private `0600` JSONL event artifact is fsynced and atomically renamed; redacted JSON and HTML reports are generated from it. The worker receives fixtures and validated actions, never reference actions, oracles, rubrics, expected answers, repository mounts, or API keys.

The schema-v4 helper contract additionally records hard diagnostics and warnings separately, execution startup, runtime outcome, artifact commit, model-call count, candidate tokens, repair latency, and launcher/helper setup time. Reference actions come from the separate schema-v4 bundle named by corpus v2; the locked corpus-v1 artifact remains unchanged.

The frozen qualification gates live in `model-qualification-policy-v1.json`; the runtime consumes the same policy and only trusts reviewed entries in `model-qualification-manifest.json`. The checked-in manifest and holdout commitment are intentionally unavailable: no automatic provider/model choice is qualified yet. `--profile full` is development-only. The separate `--profile qualification` refuses anything except a sealed, independently authored all-holdout corpus and stops for a structured 20-item audit before finalization. See the [provider qualification runbook](qualification.md).

Gate A's explicit Cerebras fixed-mode smoke passed on 2026-08-03 with `gpt-oss-120b` through the compiled buffered adapter and shared validator. This verifies endpoint compatibility only; it is not program, request-class, or automatic-selection qualification.

Judge transport/format failures, real calls, retries, calibration repeats, and synthetic outcomes are reported separately. Every deterministic pass is judged; failures use a seeded diagnostic sample plus all safety-critical families. The judge never sees provider, model, latency, price, or trial identity and cannot promote a deterministic failure. A 20-item independent blinded audit remains required before qualification. Raw artifacts include prompts, proposals, and rationales, so keep them private even though credentials are never recorded.

Cerebras supports strict tool calls but rejects schema size bounds and string patterns used by the OpenAI Responses transport. The production adapter removes `maxLength`, `maxItems`, and `pattern` only from the Cerebras wire schema and still enforces the complete canonical schema locally before accepting or judging an action.
