<!-- diataxis: reference -->

# Provider benchmark reference

The current development benchmark uses the 120-task corpus v2 in `tests/fixtures/provider-execution-benchmark-v2.json` and the separate schema-v4 reference-action bundle. Candidate calls pass through the compiled production provider adapters and canonical validator; judge calls use a separate blinded client.

## Measurement structure

| Property | Current behavior |
|---|---|
| Development corpus | 120 tasks, 52 semantic families |
| Full profile | all tasks, three trials per candidate |
| Smoke profile | 12 fixed tasks, one trial |
| Primary paired interval | resamples semantic families |
| Program profiles | `first-shot`, or development-only `bounded-repair` |
| Runtime validation | production canonical decoder and preflight |
| Qualification authority | none; development profiles cannot emit it |

The runner reports wire validity, canonical contract validity, runtime preflight, allowed and preferred routes, execution attempts, deterministic oracle outcomes, derived completion, latency, token use, model-call counts, and judge behavior separately.

## Run fingerprint

Resume and report validity bind the provider helper, policy, manifest, corpus, runner, schemas, worker, reference actions, prompt/action contract, and other compatibility inputs. `--resume` rejects a different fingerprint.

## Execution worker

Generated shell, Python, and parent-shell actions run in a fresh Docker worker with:

- no network;
- no API keys;
- no repository or host mounts;
- no Docker socket;
- a read-only image filesystem;
- a non-root user and dropped capabilities;
- bounded `/tmp` and `/work` tmpfs filesystems.

Docker is a practical containment boundary for benchmark execution, not VM-grade isolation. Candidate and judge API calls remain in the trusted host runner. The worker never receives reference actions, oracles, rubrics, expected answers, credentials, or resolved host paths.

## Repair measurement

`bounded-repair` offers at most one user-approved complete replacement after a semantic contract failure or a production-visible nonzero, timeout, or overflow. The replacement prompt excludes oracle failures, expected answers, fixture metadata, child output, and resolved host paths. First-shot and cumulative-if-approved completion remain separate measures.

## Judging and artifacts

Every deterministic pass is judged. Failures use a seeded diagnostic sample plus all safety-critical families. The judge does not receive provider, model, latency, price, or trial identity and cannot promote a deterministic failure.

Final raw JSONL artifacts are private mode `0600` and may contain prompts, proposals, outcomes, and rationales. Redacted JSON and HTML reports are derived from them. Judge transport failures, retries, calibration repeats, real calls, and synthetic outcomes are reported separately.

## Current qualification status

The checked-in manifest and holdout commitment are unavailable. Gate A's explicit Cerebras fixed-mode smoke passed on 2026-08-03 with `gpt-oss-120b`; this establishes endpoint compatibility only, not program, request-class, or automatic-selection qualification.

## Historical evidence

The original cross-provider corpus-v1 run concluded that neither candidate qualified. Its exact route score, program-contract validity, confidence interval, and judge-call totals are not decision-grade because the runner duplicated weaker validation, treated preferred routes as required, used brittle formatting oracles, resampled task variants instead of semantic families, and mixed synthetic judgments with real calls. The artifact remains frozen and must not support a new selection claim.

Before cross-provider infrastructure, the 2026-08-01 v0.1 release bakeoff used six minimal-context dry-run cases per OpenAI model:

| Model | Structured valid | Route correct | Median complete proposal |
|---|---:|---:|---:|
| `gpt-5.6-luna` | 6/6 | 6/6 | 2,247 ms |
| `gpt-5.6-terra` | 6/6 | 6/6 | 1,824 ms |
| `gpt-5.6-sol` | 6/6 | 6/6 | 2,656 ms |

Terra became the v0.1 default because it was fastest at the median among those 18 valid proposals. The six-case comparison is historical release evidence, not stable latency evidence or current qualification.
