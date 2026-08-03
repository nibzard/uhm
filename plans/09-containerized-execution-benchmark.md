# Plan 9 — Add a containerized end-to-end model benchmark

## Purpose and dependency

This plan turns the existing proposal-only provider bakeoff into an end-to-end quality benchmark. Every candidate receives the same versioned tasks and fresh fixtures. Executable proposals run inside disposable Docker workers, deterministic assertions decide whether the requested outcome happened, and a blinded LLM judge scores the remaining semantic qualities.

The benchmark is quality infrastructure, not a new production execution path. It depends on the typed action contracts from Plans 2, 4, and 6, but it does not change normal `uhm` execution or configuration.

The practical invariant is:

```text
trusted runner with API keys and provider network access
    → one model proposal
    → one fresh, keyless, offline Docker worker
    → deterministic result assertions
    → one blinded judge call with sanitized evidence
    → recorded result
```

The worker never receives an API key, Docker socket, repository mount, home directory, or network interface. “No network” applies to generated action execution; the trusted runner still needs outbound HTTPS for candidate and judge API calls.

## Settled design

| Topic | Decision |
| --- | --- |
| Corpus size | 120 independent tasks in one versioned JSON file |
| Repetition | Three candidate trials per task/model for the full comparison |
| Statistical unit | Task ID, with all trials kept together; trials are not counted as independent tasks |
| Primary quality result | Deterministic end-to-end task success |
| Secondary quality result | Blinded LLM judge score and verdict |
| Worker platform | One digest-recorded Debian slim image for the initial benchmark |
| Worker network | None (`--network none`) |
| Worker credentials | None; API keys remain in the trusted runner only |
| Execution lifecycle | Fresh worker and fixture state for every candidate attempt |
| Host mounts | None in the worker |
| Proposal execution | Shell and Python actions only; answer/clarification remain non-executing |
| Parent-shell actions | Applied only inside a disposable shell and inspected afterward |
| Result format | Private JSONL run artifact plus a concise terminal summary |
| Winner policy | Quality first; latency breaks a quality tie only after the fixed gates and paired confidence interval pass |

## 1. Create a statistically useful fixed corpus

Add `tests/fixtures/provider-execution-benchmark-v1.json` with exactly 120 unique task IDs. Seed it from the existing 24-task proposal corpus, but rewrite every executable task to include a complete fixture and deterministic oracle.

Use this route distribution:

| Stratum | Tasks | Notes |
| --- | ---: | --- |
| `run_shell`, read/search/inspect | 28 | Files, Git, sizes, filtering, local process/socket inspection |
| `run_shell`, bounded write/delete | 20 | Exact paths, quoting, directories, archives, consequential scope |
| `run_program`, stdout result | 28 | JSON, JSONL, CSV, TSV, logs, text statistics and transformations |
| `run_program`, artifact/replace | 20 | New artifacts, exact rewrites, multifile inputs, overwrite behavior |
| `require_parent_shell` | 8 | Cwd, set/unset environment, source file |
| `request_clarification` | 8 | Missing path, format, delimiter, scope, overwrite, or destination |
| `return_answer` | 8 | Terminal and Git explanations with factual oracles |
| **Total** | **120** | Independent tasks |

Within the executable strata, deliberately cover:

- Empty, single-item, and multiple-item fixtures.
- Spaces, Unicode, leading dashes, hidden files, nested directories, and similar prefixes.
- Ordered and unordered output.
- UTF-8 stdin, declared structured formats, and local-only file inputs.
- Staged, unstaged, untracked, clean, and detached Git states.
- Exact bounded deletion and writes without broad globs.
- Valid alternate implementations rather than one preferred command spelling.
- Expected nonzero exits only when the task explicitly asks to detect a condition.

Exclude tasks that depend on the public internet, wall-clock “now,” host processes, host usernames, external services, package installation, or unavailable hardware. Freeze timezone to UTC, locale to `C.UTF-8`, umask, fixture timestamps, Git identity, Git commit dates, `HOME`, `TMPDIR`, and working directory.

### Corpus JSON contract

Keep task setup and assertions declarative. Do not put arbitrary setup or grading shell scripts in the corpus.

```json
{
  "version": 1,
  "prompt_version": 8,
  "action_schema_version": 3,
  "worker_contract_version": 1,
  "task_count": 120,
  "tasks": [
    {
      "id": "json-category-totals",
      "route": "run",
      "prompt": "sum amount by category and print sorted JSON",
      "tags": {
        "expected_tool": "run_program",
        "category": "structured-data",
        "difficulty": "medium",
        "effects": ["read_local"]
      },
      "fixture": {
        "cwd": "/work",
        "stdin": {
          "encoding": "utf-8",
          "declared_format": "application/json",
          "text": "[{\"category\":\"b\",\"amount\":2},{\"category\":\"a\",\"amount\":3}]"
        },
        "directories": [],
        "files": [],
        "symlinks": [],
        "environment": {}
      },
      "limits": {
        "wall_ms": 10000,
        "stdout_bytes": 1048576,
        "stderr_bytes": 262144,
        "workspace_bytes": 67108864
      },
      "expected": {
        "tools": ["run_program"],
        "exit_codes": [0],
        "stdout": {
          "matcher": "json_equals",
          "value": {"a": 3, "b": 2}
        },
        "stderr": {"matcher": "empty"},
        "filesystem": [],
        "forbid_undeclared_changes": true
      },
      "judge_rubric": "Check that the proposal is precise, locally bounded, and uses only the Python standard library."
    }
  ]
}
```

Allow a small fixed matcher vocabulary:

- `exact_text`
- `contains_lines`
- `unordered_lines`
- `regex`
- `json_equals`
- `csv_equals`
- `empty`
- `file_exists`
- `file_absent`
- `file_sha256`
- `directory_exists`
- `tree_equals`
- `environment_equals`
- `cwd_equals`

Large fixture bytes may live under `tests/fixtures/provider-execution/`, but the JSON must name each referenced file and its SHA-256. A task may not execute a task-specific grader.

## 2. Build one purpose-made worker image

Add `benchmark/docker/Dockerfile` based on a pinned Debian slim base. Build one immutable worker image and record its resolved image digest in every result. Do not start with a distribution matrix; add Alpine or another image only after the Debian corpus is stable and portability evidence is needed.

Install the tools used by the corpus, not every tool a generated command might invent:

- Bash and POSIX shell.
- GNU coreutils, findutils, grep, sed, gawk, and diffutils.
- Git.
- Python 3 standard library.
- `jq` and `ripgrep`.
- `file`, `tar`, gzip, xz, zip, and unzip.
- `procps`, `iproute2`, and util-linux.

The image also contains:

- The versioned corpus and referenced fixtures at `/opt/uhm-bench/`.
- A small trusted worker entrypoint.
- A non-root `bench` user with a fixed UID/GID.
- A machine-readable tool/version manifest generated at build time.

The entrypoint reads one validated action envelope from stdin, copies the named fixture from the read-only image into a fresh `/work` tmpfs, executes the action, applies declarative assertions, and writes one JSON result envelope. It must never make an LLM request.

Run every executable attempt with controls equivalent to:

```sh
docker run --rm \
  --network none \
  --read-only \
  --user 10001:10001 \
  --cap-drop ALL \
  --security-opt no-new-privileges=true \
  --pids-limit 128 \
  --memory 512m \
  --cpus 1 \
  --tmpfs /tmp:rw,nosuid,nodev,noexec,size=32m \
  --tmpfs /work:rw,nosuid,nodev,size=128m,uid=10001,gid=10001 \
  --workdir /work \
  uhm-bench-worker@sha256:<digest>
```

Keep Docker's default seccomp profile. Do not add `--privileged`, capabilities, devices, host PID/IPC/network namespaces, or `seccomp=unconfined`. Prefer rootless Docker where available, but do not make rootless setup part of the benchmark implementation.

Docker is practical containment, not a promise that hostile code cannot exploit the shared kernel. The combination of offline workers, no secrets or host mounts, non-root execution, fresh tmpfs state, dropped capabilities, and resource limits is the declared risk boundary.

## 3. Execute actions and assert observable outcomes

Extend `scripts/provider-bakeoff.py` rather than create a second competing benchmark command.

For `run_shell`:

- Pass the validated command as data to the trusted worker entrypoint.
- Execute it inside the worker with a fixed Bash invocation and controlled environment.
- Feed the task's exact stdin bytes when `stdin_mode=original`; otherwise close stdin.
- Capture exit code, timeout/signal, stdout, stderr, and workspace state.

For `run_program`:

- Write the validated source to private worker storage.
- Invoke `python3 -I -S` directly, never through a shell.
- Set `PYTHONDONTWRITEBYTECODE=1` and the same input/output manifests used by UHM.
- Enforce declared inputs, outputs, result mode, and workspace changes through assertions.

For `require_parent_shell`:

- Start one disposable Bash process inside the worker.
- Apply only the existing audited typed renderer, not model-authored shell source.
- Query the resulting cwd or environment and compare it with the task oracle.

For `return_answer` and `request_clarification`:

- Do not start Docker.
- Apply route/schema checks and the task's deterministic text constraints, then send the result to the judge.

The trusted runner owns the external timeout and output byte caps. On timeout or overflow, stop the worker, record the attempt as a deterministic failure, and remove it. Malformed proposals, wrong routes, provider errors, timeouts, nonzero unexpected exits, and assertion failures all remain in the end-to-end denominator.

The worker result contains only bounded evidence:

```json
{
  "started": true,
  "exit_code": 0,
  "signal": null,
  "timed_out": false,
  "stdout": "...",
  "stderr": "",
  "stdout_truncated": false,
  "stderr_truncated": false,
  "workspace_manifest": [],
  "assertions": [
    {"name": "stdout.json_equals", "passed": true, "detail": null}
  ],
  "deterministic_pass": true
}
```

## 4. Keep the LLM judge secondary and blinded

The LLM judge receives:

- Task prompt and task-specific rubric.
- Anonymous validated proposal.
- Bounded stdout/stderr.
- Exit/timeout outcome.
- Filesystem assertion summary and sanitized manifest.
- No provider, candidate model, latency, price, or trial identity.

The judge continues to score task correctness, instruction following, safety/precision, and portability on the existing 0–4 scale. It may identify defects not encoded by the oracle, but it cannot turn a deterministic failure into an end-to-end pass.

Use one fixed judge model that is not a candidate when possible. Keep the judge prompt, model, provider, and reasoning effort in the run header. Judge transport/format failures are missing judgments: retry once, then report them separately without converting them into candidate failures.

For every full benchmark, rejudge a fixed stratified 12-task calibration slice a second time. Report exact verdict agreement. If agreement is below 10/12, label judge-derived comparisons unstable; deterministic results remain valid.

## 5. Use paired statistics without pseudo-replication

The full benchmark runs 120 tasks × 3 trials for each candidate. Candidate order is randomized within task/trial blocks with a recorded seed. Every candidate starts from an identical fresh fixture.

The task ID is the independent unit:

- Aggregate three trial outcomes into a per-task success rate.
- Compute overall and per-stratum deterministic success.
- Use a paired, task-level bootstrap with 10,000 resamples for the 95% confidence interval of candidate quality differences. Resample task IDs and keep all trials for a task together.
- Report a paired majority-pass comparison (at least two of three trials) and an exact McNemar test as a secondary result.
- Report proposal validity, route correctness, provider reliability, timeout rate, judge score, and latency separately.
- Use successful-response latency only for latency percentiles; never remove failures from quality denominators.

The 120-task corpus is a practical baseline intended to resolve differences of roughly 8–10 percentage points. It is not enough to promise resolution of a five-point difference. Before the first model-selection claim, use pilot discordance from the fixed corpus to calculate paired power at 80% power and two-sided 5% significance. If 120 tasks are insufficient for the observed difference, report the comparison as inconclusive and expand the next corpus version; do not count the three trials as 360 independent tasks.

For more than one predeclared candidate comparison, apply Holm correction to secondary p-values. Confidence intervals and effect sizes remain the primary report.

### Selection rule

A model/provider qualifies only when it has:

- At least 98% structured-action validity.
- At least 95% correct routes.
- At least 90% deterministic end-to-end task success overall.
- At least 80% deterministic success in every executable stratum.
- Zero proposals that target a materially broader destructive scope than requested.

Declare a quality winner only when the paired 95% confidence interval for the deterministic-success difference excludes zero. If qualified candidates are statistically tied, prefer the faster candidate only when the lower bound of its quality difference is no worse than -5 percentage points. Otherwise report no winner.

## 6. Keep two run sizes

Provide two documented commands:

1. Smoke: 12 stratified tasks, one trial, useful only to check provider credentials, image availability, and the complete pipeline.
2. Full: all 120 tasks, three trials, required for quality or model-selection claims.

The user supplies only the provider keys required by the selected candidate and judge models. The runner checks Docker, builds or resolves the worker image, validates the corpus/tool manifest, and refuses to start a full run if any fixture or expected executable is missing.

Do not cache candidate proposals in either mode. One transport retry is allowed only for an unambiguous connection/rate-limit failure and is recorded; model-format or task failures are never retried automatically.

## 7. Record enough provenance to reproduce a run

Write private `0600` JSONL under `target/` with:

- Corpus version and SHA-256.
- Prompt/action/worker contract versions.
- Worker image repository tag and resolved digest.
- Tool/version manifest hash.
- Candidate and judge provider/model/reasoning settings.
- Task order seed and trial number.
- Candidate transport timing and usage.
- Proposal, validation, execution evidence, assertions, and judge result.
- Aggregate metrics, confidence intervals, and qualification decision.

Never record API keys, authorization headers, the host environment, Docker configuration, or unrelated host paths. Cap all stored model and execution text.

## Implementation sequence

### Phase A — Freeze schema and statistics

- Add the corpus JSON schema and worker result schema.
- Add schema validation and prompt/action version gates.
- Implement paired task-level aggregation, bootstrap intervals, and McNemar reporting with unit tests over known synthetic data.
- Document the nominated primary comparison before a full run.

### Phase B — Build the worker

- Add the Dockerfile, non-root entrypoint, tool manifest, and image build script.
- Implement declarative fixture creation and assertion matchers.
- Add shell, Python, timeout, output overflow, workspace overflow, and parent-shell execution paths.
- Prove workers have no network, credentials, host mounts, extra capabilities, or Docker socket.

### Phase C — Author and validate 120 tasks

- Expand the current 24 tasks to the fixed route distribution.
- Add declarative fixtures and oracles.
- Run every oracle against a known-good reference action in the pinned image.
- Add one known-bad action per matcher class to prove the oracle rejects incorrect results.
- Review every task for ambiguity, alternate valid implementations, and accidental dependence on the host or current time.

### Phase D — Integrate candidates and judge

- Feed validated proposals to the worker and attach bounded evidence to anonymous judge calls.
- Preserve all candidate failures in the denominator.
- Separate judge failures from candidate quality.
- Add smoke/full CLI profiles and private run artifacts.

### Phase E — Validate the benchmark itself

- Run the 12-task smoke against OpenAI and Cerebras.
- Run a full comparison only after all reference actions and negative controls pass.
- Manually inspect a stratified sample of 20 results and all disagreements between deterministic assertions and the judge.
- Freeze corpus v1 after validation; later task changes create v2 rather than silently changing old scores.

## Expected outcomes

- Provider/model comparisons measure completed task outcomes rather than attractive proposals.
- Every executable attempt runs from the same clean, offline environment with the tools required by its task.
- Deterministic assertions catch concrete command/program failures; the judge adds semantic review without owning ground truth.
- The benchmark reports uncertainty and ties honestly instead of ranking noise.
- Running the benchmark requires only the repository, Docker, Python/curl already used by the harness, and the selected provider API keys.

## Definition of done

- The versioned JSON corpus contains exactly 120 unique, reviewed tasks in the declared strata.
- Corpus and worker-result JSON schemas reject unknown fields, invalid paths, arbitrary grader code, invalid matchers, and stale prompt/action/worker versions.
- Every executable task has a known-good action that passes and a targeted known-bad control that fails.
- The worker image is reproducible enough to identify by digest and publishes a complete tool/version manifest.
- Workers run non-root, offline, read-only except bounded tmpfs, with all capabilities dropped, no new privileges, default seccomp, and fixed CPU/memory/PID/output/time limits.
- Tests prove the worker receives no API keys, Docker socket, host directory, or runner environment values.
- Shell, Python, parent-shell, answer, and clarification paths produce the documented evidence envelopes.
- Deterministic matchers correctly handle structured, unordered, filesystem, environment, cwd, timeout, and truncation cases.
- Candidate provider errors, malformed proposals, wrong routes, execution failures, and assertion failures stay in quality denominators.
- Judge calls are blinded; judge failures are separate; deterministic failures cannot be promoted to passes.
- Statistical tests treat task IDs as clusters, preserve paired candidates, and reproduce results for a fixed seed.
- The smoke profile completes the full pipeline; the full profile runs 120 × 3 attempts per candidate.
- Run artifacts are private, bounded, complete enough to reproduce aggregate metrics, and contain no credentials.
- Existing Rust tests, benchmark self-tests, JSON schema tests, worker integration tests, and Docker smoke tests pass.

## Anti-goals

- Do not build a general hostile-code sandbox or claim VM-grade isolation.
- Do not run generated actions on the host.
- Do not give workers provider keys, network access, the Docker socket, or host mounts.
- Do not add Kubernetes, Docker Compose, nested Docker, a job queue, distributed workers, or a database.
- Do not start with multiple Linux distributions or architecture matrices.
- Do not install tools dynamically during a benchmark.
- Do not use task-specific assertion scripts or exact command-string matching.
- Do not let the LLM judge override deterministic outcomes.
- Do not treat repeated trials as independent tasks or claim a winner when the interval is inconclusive.
- Do not change a frozen corpus in place; create a new version.

## Primary code areas

- `scripts/provider-bakeoff.py`
- `tests/fixtures/provider-execution-benchmark-v1.json`
- `tests/fixtures/provider-execution/`
- `benchmark/docker/Dockerfile`
- `benchmark/worker/`
- `benchmark/schemas/`
- `docs/model-selection.md`

## References

- Docker run controls and resource limits: <https://docs.docker.com/reference/cli/docker/container/run>
- Docker default seccomp profile: <https://docs.docker.com/engine/security/seccomp/>
- Docker rootless mode: <https://docs.docker.com/engine/security/rootless/>
