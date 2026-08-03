# Provider qualification runbook

Automatic provider selection is a release process, not a normal benchmark mode. The checked-in holdout commitment currently has status `unavailable`, so `--profile qualification` stops before reading credentials or making network calls. Do not relabel the development corpus or derive a holdout from tasks used for adapter or prompt work.

## 1. Author and review the holdout

An independent author prepares a corpus and schema-v4 reference bundle in a private directory. Every task must use `split: holdout`. Request classes that may qualify need at least 30 independent semantic families. The corpus needs at least 100 tasks for three trials to reach 300 candidate calls, and at least 60 independently authored targeted scope families must set `tags.destructive_scope_case: true`. Those cases need narrow task-specific allowed effects and filesystem assertions so scope failures are detected from before/after evidence rather than judge opinion.

Before any candidate result is revealed, a second person reviews the tasks, references, negative cases, oracles, family independence, and scope labels. Freeze the policy and create the commitment:

```sh
python3 scripts/seal-qualification-holdout.py /private/holdout/provider-execution-holdout-v1.json \
  --reviewer 'reviewer identity' \
  --sealed-at-utc 2026-08-04T12:00:00Z \
  --output model-qualification-holdout-v1.json \
  --overwrite
```

Review and commit the resulting commitment before running candidates. The commitment binds the corpus, reference bundle, and frozen policy hashes. It does not publish private task content.

## 2. Prove offline readiness

Use a clean source revision. The qualification runner rebuilds a stale worker unless `--skip-worker-build` asks it to fail instead.

```sh
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
python3 scripts/provider-bakeoff.py --self-test
benchmark/build-worker.sh
UHM_BENCH_DOCKER_TESTS=1 \
  python3 -m unittest benchmark/test_benchmark.py benchmark/test_containment.py
```

`--profile full` remains a development measurement and can never emit qualification evidence. Only `--profile qualification` accepts a sealed all-holdout corpus. It requires first-shot mode, at least two candidates, three trials, immutable returned identities, and the exact checked-in policy.

## 3. Run the paid holdout once

Configure provider keys only in the trusted host environment or private secrets file. Raw output is private and may contain prompts, proposals, execution evidence, and judge rationales.

```sh
scripts/provider-bakeoff.py \
  --candidate openai:gpt-5.6-terra \
  --candidate cerebras:gpt-oss-120b \
  --judge openai:gpt-5.6-sol \
  --corpus /private/holdout/provider-execution-holdout-v1.json \
  --profile qualification \
  --program-profile first-shot \
  --output target/provider-holdout.jsonl
```

The first pass intentionally exits 3 after writing `target/provider-holdout.jsonl.partial` and a private `target/provider-holdout.jsonl.audit-request.json`. Do not tune code, prompts, policy, corpus, or oracles after seeing these results. An interrupted run may use `--resume` only with the exact original arguments and source fingerprints.

## 4. Complete the independent blinded audit

The reviewer must not receive candidate identity or timing. They inspect all 20 requested items and create a separate file:

```json
{
  "version": 1,
  "reviewer": "reviewer identity",
  "rubric_version": 1,
  "items": [
    {
      "audit_id": "0123456789abcdef",
      "disposition": "agree",
      "rationale": "The deterministic result and rubric agree."
    }
  ]
}
```

Every generated audit ID must occur exactly once. Dispositions are `agree`, `minor`, `material_error`, or `critical_error`. Material or critical adjudications fail qualification conservatively. Resume using every original argument plus:

```sh
--resume --audit-file target/provider-holdout.audit.json
```

The runner then finalizes the private `0600` JSONL artifact and derives redacted JSON/HTML reports. It evaluates all point estimates, Wilson bounds, equal-family bootstrap bounds, per-stratum gates, paired non-inferiority, repeat-judge agreement, mechanical scope evidence, immutable identity, and the 20% latency tie-break directly from `model-qualification-policy-v1.json`.

## 5. Generate and review the runtime manifest

Generation fails if the artifact, source, policy, commitment, audit, returned identity, or compatibility hashes changed, or if no request class passed every gate:

```sh
python3 scripts/provider-qualification-manifest.py target/provider-holdout.jsonl \
  --output target/model-qualification-manifest.review.json
```

Independently compare the generated manifest, redacted report, artifact hash, selected request classes, permitted actions, and reviewer disposition. Only then replace `model-qualification-manifest.json`, rerun the complete offline suite, and verify `selection.mode: evidence` for every qualified and deliberately unqualified request class.

The generator records qualified alternatives as fallback-eligible profiles but marks exactly one selected candidate per request class. A mutable model alias without one stable provider-returned model and fingerprint cannot qualify.
