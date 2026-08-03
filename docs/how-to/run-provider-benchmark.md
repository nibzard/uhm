<!-- diataxis: how-to -->

# Run the provider benchmark

Use this maintainer guide to compare fixed provider/model candidates with the development corpus. Development runs cannot authorize evidence-mode selection.

## 1. Configure host credentials

```sh
export OPENAI_API_KEY='...'
export CEREBRAS_API_KEY='...'
```

Candidate and judge calls occur in the trusted host runner. Credentials are never passed to the execution worker.

## 2. Build and validate the worker

```sh
benchmark/build-worker.sh
UHM_BENCH_DOCKER_TESTS=1 python3 -m unittest \
  benchmark/test_benchmark.py benchmark/test_containment.py
```

## 3. Run the smoke profile

```sh
scripts/provider-bakeoff.py \
  --candidate openai:gpt-5.6-terra \
  --candidate cerebras:gpt-oss-120b \
  --judge openai:gpt-5.6-sol \
  --profile smoke \
  --program-profile first-shot \
  --output target/provider-smoke.jsonl
```

The smoke profile fixes 12 tasks and one trial. Use it to validate transport and runner behavior, not to make a model-selection claim.

## 4. Run the full development profile

```sh
scripts/provider-bakeoff.py \
  --candidate openai:gpt-5.6-terra \
  --candidate cerebras:gpt-oss-120b \
  --judge openai:gpt-5.6-sol \
  --profile full \
  --program-profile first-shot \
  --output target/provider-bakeoff.jsonl
```

The full profile fixes all 120 tasks and three trials per candidate. `--resume` works only when the complete run fingerprint matches. Use `--profile custom`, `--task-id`, `--task-count`, and `--trials` only for debugging subsets.

## 5. Keep raw artifacts private

The finalized `0600` JSONL artifact contains prompts, proposals, outcomes, and judge rationales. Store it privately. Redacted JSON and HTML reports are derived from it.

For automatic selection evidence, stop here and follow the separate [provider qualification runbook](../qualification.md), which requires a sealed independent holdout and a 20-item audit.
