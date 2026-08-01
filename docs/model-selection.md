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
