# v0.3.0 tmp-drive evaluation archive

This directory preserves the inputs, assertion runner, summarized records, and raw stdout/stderr from the evaluation described in [`../../uhm-v0.3.0-eval-report.md`](../../uhm-v0.3.0-eval-report.md). The report was moved into the repository root before this archive was added, so its relative link from here is `../../uhm-v0.3.0-eval-report.md`.

The evidence is historical: it records one pass per task against the installed v0.3.0 binary, followed by eight corrected invocations. Do not regenerate the published report and silently replace these records.

## Recorded configuration

- Provider: `openai`
- Model: `gpt-5.6-terra` (the v0.3.0 default at evaluation time)
- Reasoning effort: `low`
- Streaming: enabled
- Context: `standard`
- Cache: bypassed with `--fresh`
- Rendering: `--plain --json`
- Telemetry: disabled

The provider API does not expose a user-configurable temperature in this client, so no sampling-temperature setting was supplied.

## Files

- `battery.json`: all 38 original scenarios, exact prompts, invocation modes, and assertions.
- `battery2.json`: the eight corrected round-2 invocations.
- `setup_corpus.py` and `expected.json`: deterministic fixtures and expected values.
- `run_battery.py`: the original assertion logic, made path/provider/model-configurable for replay while retaining the original checks.
- `results.jsonl` and `results2.jsonl`: complete structured run records.
- `out/`: raw stdout and stderr for every original and corrected invocation.

## Stronger follow-up protocol

For release qualification, run every model-routed scenario at least three times in separately regenerated sandboxes, report both scenario pass rate and per-task consistency, and keep every attempt. Record `uhm --version`, binary SHA-256, provider, requested and resolved model identity when available, reasoning effort, streaming mode, context mode, OS/runtime versions, timestamps, and end-to-end wall time.

The broader repository test suites already exercise quoting and whitespace, symlink rejection, traversal rejection, malformed envelopes and data, bounded input/output/workspaces, hostile untrusted content, process termination, and timeout handling. A future live battery should sample those boundaries too; this archive does not retroactively claim that it did.

`repeat_battery.py` implements that repeatability protocol for the corrected 38-scenario contract in `battery-contract.json`. It regenerates the corpus for every attempt, defaults to three attempts, records environment and binary metadata, and preserves every structured and raw result under `target/tmp-drive-eval/`:

```console
cargo build
python3 evaluation/tmp-drive-v0.3.0/repeat_battery.py --repeats 3
```

## Piped-ask fix verification

On 2026-08-03, the B9 request was run three times with `--fresh` against the updated source build, OpenAI `gpt-5.6-terra`, and the original `events.log`. All three attempts returned rc 0 with `outcome=answer`; none proposed or executed a local action. The three summaries independently reported the exact `31` successes and `29` errors (`404: 15`, `403: 7`, `500: 7`). This focused 3/3 check verifies the routing fix but is not represented as a rerun of the other 37 live scenarios.

The corrected full battery was then run three times in separately regenerated sandboxes. It passed 113/114 attempts: B9 passed 3/3, and the only miss was one C16 clarification asking the user for a log format that the named file could reveal. Prompt contract v12 now forbids clarification for facts discoverable by the proposed read action. After rebuilding, C16 passed 3/3 focused reruns and produced the exact histogram each time (`200: 31`, `403: 7`, `404: 15`, `500: 7`). The pre-fix full-run records remain under `target/tmp-drive-eval/20260803T215812Z/` in this workspace and are deliberately not folded into the historical v0.3.0 archive.
