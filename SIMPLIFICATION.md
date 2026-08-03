# Simplification plan for `uhm`

This plan is based on the current code and call sites, not a projected LOC total.
The aim is to reduce the number of places where behavior is encoded while keeping
the CLI contract, privacy boundaries, durability guarantees, and bounded execution
model unchanged.

Raw line count is a poor guide here. The duplicate Rust module graph, repeated
history parsing, and copied policy or filesystem invariants create more
maintenance risk than a long function by itself.

## Reviewed baseline

- Review point: `b9f4712`, one commit after the v0.3.0 tag at `588c354`.
- `src/` contains 20,820 lines of Rust, including 223 lines in two undeclared
  render files.
- `main.rs` and `lib.rs` each declare the same 32 modules. As a result, the unit
  tests in those modules are compiled and run twice: 184 under the library and the
  same 184 under the `uhm` binary.
- `#![allow(dead_code)]` in `lib.rs` currently hides about 296 warnings because the
  library copy cannot reach the CLI entry points in the binary's separate module
  graph. Removing that allowance before fixing the graph would produce noise, not
  a useful dead-code audit.
- Ten generated CPython bytecode files are tracked in Git. They occupy 302,619
  bytes and are not ignored.

Baseline verification passed:

- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets` (390 tests across the duplicated Rust targets)
- `python3 scripts/provider-bakeoff.py --self-test`
- `python3 -m unittest benchmark/test_benchmark.py benchmark/test_containment.py`
  (27 tests, 8 skipped because they are opt-in)
- `python3 scripts/check-docs.py`
- `(cd telemetry-worker && npm test)` (5 tests)

## Rules for the work

1. Make one conceptual change per commit. Most batches below are independent.
2. Preserve observable error categories and machine-readable output. Error prose
   may be normalized only when it is not part of a documented or tested contract.
3. Keep these orderings explicit at their call sites:
   - model-call and execution budget transitions;
   - program versus shell edit-budget consumption;
   - history lock acquisition and journal validation;
   - telemetry policy rechecks before queue or network access;
   - file sync, publication, and parent-directory sync.
4. Share an invariant only when the policies really match. Similar syntax is not
   enough.
5. Do not add a general `util` module. New shared code should have a narrow name
   and one reason to change.

## Batch 1: remove generated and unreachable code

This batch has no intended runtime effect.

### Delete tracked build artifacts

Remove the ten tracked files below and add `__pycache__/` and `*.py[cod]` to
`.gitignore`:

- `scripts/__pycache__/` (5 files)
- `benchmark/__pycache__/` (3 files)
- `benchmark/worker/__pycache__/` (2 files)

### Delete undeclared render modules

- `src/render/markdown.rs` is 204 lines, is not declared by `render.rs`, and calls
  render APIs that do not exist.
- `src/render/sync.rs` is 19 lines, is not declared by `render.rs`, and has no
  references. `UHM_SYNC_OUTPUT` occurs nowhere else.

Deleting either file cannot change the compiled crate.

### Delete obsolete provider-bakeoff helpers

The following functions have no callers:

- `string_array`
- `effects_schema`
- `synthetic_judgment`
- `add_qualification`
- `selection_decision`

After `add_qualification` is removed, `wilson95` is also unused. The live
qualification path is `qualification_policy.evaluate()`.

Replace the hand-written `shutil_which` with `shutil.which` at the same time.

### Remove test-only policy skew

Delete `model_selection::fallback_allowed` and its synthetic unit test. The helper
is compiled only for tests and does not match the real fallback branch in
`api.rs`. The API tests already exercise the production rules: typed triggers,
one sequential fallback, credential failure, identity checks, and the two-attempt
ceiling.

### Review unreferenced repository fixtures separately

These are not automatic deletions because the project may intentionally retain
old release artifacts:

- `tests/fixtures/provider-bakeoff-v1.json` has no references (9,580 bytes).
- `tests/fixtures/provider-execution-benchmark-v1.json` is referenced only by the
  completed plan 09 (290,191 bytes); live tooling uses v2.
- `tests/fixtures/action-validation-cases-v1.json` is referenced only by the
  completed plan 11 (3,262 bytes); tests use v2.

Choose a fixture retention policy first. If completed plans are historical notes
rather than reproducible snapshots, remove these files and update the plans.

`benchmark/schemas/reference-actions.schema.json` is also not loaded. Either wire
it into holdout validation as the envelope authority or remove it during the next
qualification contract revision. Do not leave both an unused schema and a manual
copy of the same shape indefinitely. If it is wired in, register or resolve its
relative `$ref` to `corpus.schema.json`; constructing a validator from the
reference-actions schema alone does not reliably resolve that resource.

## Batch 2: use one Rust module graph

This is the first structural change and should precede the dead-code lint cleanup.

`src/main.rs` and `src/lib.rs` declare the same 32 source modules as two separate
crate-local graphs. This causes duplicate compilation, duplicate unit-test runs,
and the blanket dead-code suppression in the library.

Change the ownership of the CLI entry point:

1. Move `run`, management dispatch, help text, and their private helpers from
   `main.rs` into a library-owned `app` or `cli` module.
2. Expose one deliberately narrow, doc-hidden entry function for the binary.
3. Reduce `src/main.rs` to argument collection plus a call into `uhm_cli`.
4. Keep the existing public modules needed by `uhm-bench-contract`,
   `uhm-bench-exec`, and `uhm-provider-call`.
5. Remove the duplicate `mod` declarations from `main.rs`.

Acceptance checks:

- the 184 module tests appear once rather than under both `lib.rs` and `main.rs`;
- all three helper binaries still build;
- the provider-bakeoff self-test still exercises the contract and provider-call
  bridges, and targeted Docker worker tests still exercise `uhm-bench-exec`;
- `CARGO_BIN_EXE_uhm` integration tests remain unchanged;
- CLI help, JSON output, and exit codes are byte-for-byte stable where tests cover
  them.

Only after this change should the blanket `#![allow(dead_code)]` and the two
per-module allowances in the old binary root be removed. Run a forced dead-code
check then. Public library items need a separate API decision because Rust does
not warn about unused public exports.

## Batch 3: take the small, local wins

These changes remove repetition without creating cross-module frameworks.

### Safety classification

In `safety.rs`:

- derive `PartialOrd` and `Ord` for `Tier`;
- replace `severity()` and `higher()` with direct comparisons and `max`;
- add one helper that raises the tier and always appends the reason;
- replace the 33 paired `tier = higher(...)` / `reasons.push(...)` sites in
  `classify_segment`.

The helper must append every reason even when the tier is already higher. Reason
order and multiplicity are part of the current warning output. Add an explicit
ordering test for `None < Low < Network < Destructive < Irreversible`; deriving
`Ord` otherwise makes a future enum reorder capable of silently weakening gates.

### Argument parsing

Add `take_value(argv, &mut i, flag)` for the ten string-valued options in
`args::parse_from`. Keep `--uhm-parent-status` separate because it parses `i32`.

Build `prompt` before moving `intent` into `operands`; this removes the final clone.
Keep opaque-argument behavior and all current missing-value errors.

### Directory resolution

Extract the three XDG root matches in `dirs::resolve` into a small helper. Preserve
lazy HOME failure: resolution must still succeed without `HOME` when all three
absolute XDG roots are present.

### Time

Replace the private `Clock` trait and its single `SystemClock` implementation with
`clock::unix_seconds()`. Update cache, the receipt function in `command.rs`,
history, recovery, and shell integration to call that function rather than routing
time through `history::now_secs`.

There is no injected clock implementation today. Calling the clock module
directly also stops recovery and shell integration from depending on history just
to read the time.

### Small one-site cleanups

- Alias `contract::CONTEXT_POLICY_VERSION` to `context::POLICY_VERSION` instead of
  maintaining two literals.
- Build the repeated `doctor::network_check` result through one local constructor.
- Remove `history::private_file`'s `create_new` parameter; both callers pass
  `false`.
- Make `shell_integration::validate_response` validate and read one opened
  response descriptor. Merely deleting the first `validate_fixed_file` call
  changes which open is the linearization point, and therefore changes race and
  error timing if the response appears or changes between opens. Treat the
  one-descriptor version as an intentional hardening change and add a concurrent
  replacement test.
- Remove the unused `RecoveryConfig` parameter from `effective_enabled`, and
  compute the value once in `recovery::status`.
- Compute the signed child/process-group target once in `shell::execute` and use
  it for both signal forwarding and timeout termination.
- Replace the `PrivateMode` extension trait in `uhm-bench-exec` with an ordinary
  `OpenOptions` variable plus a `#[cfg(unix)]` mode assignment. The crate already
  depends on Unix-only recovery primitives, so this trait does not provide real
  cross-platform support.
- Add `ProgramFileAccess::can_read` and `can_write`, then use those predicates in
  `program.rs` instead of repeating access-mode matches.
- Introduce a named cache envelope version constant for the two literal `2`
  values.

`contract::rejection_code` should stay an ordered decision tree for now. Several
branches overlap and one predicate is compound; a table would be shorter but not
clearer.

## Batch 4: centralize real cross-module invariants

### Atomic private replacement

Create a focused private-filesystem module with two explicit operations:

- atomic private replacement that syncs the temporary file before publication;
- durable atomic private replacement that also syncs the parent directory.

Use it for the five copies in cache, first-run state, history, recovery, and
telemetry. Callers should supply context for error messages and remain responsible
for creating or validating the parent.

History export needs an explicit decision before this extraction. Its current
writer calls `ensure_private_dir` on the caller-selected output parent, which can
chmod an existing directory to `0700`. A behavior-preserving move must retain
that. The preferable behavior is to require an existing parent, leave its mode
alone, and create only the exported file as `0600`, but that is a separate CLI
behavior correction and needs an export test.

Recovery must use the parent-synced operation and retain its recognizable
temporary prefix. Add shared tests for replacement, mode `0600`, and an existing
destination. Add a first-run marker permission assertion; its current test name
says "private" but does not check the file mode.

Do not route these writes through the helper:

- `program.rs` deliberately uses `create_new` for workspace files;
- shell integration publishes descriptor-relative files with `openat`, hard-link
  checks, and exclusive creation.

Private lock-file opening can use the same focused module after the writer lands,
but locking itself should remain at callers. Telemetry currently sets `0600` only
at creation, whereas history and recovery also repair permissions on existing
lock files. Standardizing on the stronger behavior is a small security change and
needs a test.

### Run ID syntax

Share one boolean predicate for the four copies of the `8..=64`, ASCII
alphanumeric-or-dash rule in history, recovery, and shell integration. Keep each
caller's contextual error text.

Test lengths 7, 8, 64, and 65, plus slash, underscore, and non-ASCII input. Do not
change ID generation in this refactor.

### Shell naming

Share basename extraction and the command-shell support predicate across:

- argument validation;
- config validation;
- command shell normalization;
- the doctor shell check.

The existing `shell` module is a reasonable home because it has no dependency on
args or config.

Keep `auto` and empty-string handling in each caller. Args accepts raw `auto` and
empty values, config accepts basename `auto` but rejects empty, command
normalization treats raw `auto` and empty as detection requests, and doctor has no
auto case.

There is intentionally no universal shell allow-list. Parent-shell integration
supports only Bash, Zsh, and Fish. Telemetry accepts the command shells plus
`other`. Those policies must remain separate.

### Executable files

Share `is_executable_file` between context collection and runtime discovery. Their
Unix and non-Unix behavior matches.

Doctor currently checks only `is_file()` for clipboard tools. Requiring execute
permission would be a correctness fix, not a behavior-preserving extraction. Make
that change separately with a test for a non-executable file on PATH.

### Qualification thresholds

`capabilities::validate_policy` and `evidence_meets_policy` hardcode the same v1
thresholds. Parse the embedded policy once into a typed value, validate its frozen
version/hash/shape, and evaluate manifest evidence against that same value.

This must remain fail-closed. A malformed or changed policy cannot silently relax
selection, and compatible-entry checks must not bypass frozen-policy validation.

### Provider HTTP errors

Keep the non-2xx check in `provider::invoke_with`; custom `Transport`
implementations can reach it even though production `NetworkTransport` currently
maps status errors earlier.

Simplify the boundary instead:

- make the HTTP streaming function return the provider `HttpResponse` and
  `ProviderError` types directly, removing DTOs that are immediately converted;
- use one HTTP-status-to-`ProviderErrorKind` function in both paths;
- preserve bounded, sanitized provider error messages.

### Qualification script support

Add one small host-side support module for mechanics shared by the qualification
scripts:

- the production source-bundle hash, currently copied in two Python scripts and
  embedded again in `benchmark/build-worker.sh`;
- private atomic replacement, currently copied in four scripts.

Expose the source hash through a tiny CLI subcommand so the shell build script uses
the same implementation. Preserve caller-specific overwrite checks and decide
explicitly which outputs require parent-directory fsync. Keep JSON serialization
at the callers because their `sort_keys` and `ensure_ascii` choices differ; share
only publication after the caller has produced bytes or text.

The support file becomes part of the frozen qualification identity. Add it to all
of these inventories in the same commit:

- `Cargo.toml`'s explicit package include list;
- `capabilities::runner_hash`'s `include_bytes!` inputs;
- the `qualification_tooling_sha256` lists in `provider-bakeoff.py` and
  `provider-qualification-manifest.py`;
- the explicit `COPY` inputs in `benchmark/docker/Dockerfile`.

Otherwise source checkouts, packaged crates, evidence compatibility checks, and
Docker builds can disagree about which tooling was qualified.

Do not turn this module into a general statistics library. Delete the obsolete
qualification functions first, then reassess what live statistical duplication
remains.

## Batch 5: make history read and write once per operation

This is the highest-value runtime cleanup. The journal is bounded, so the current
behavior is not an emergency, but its control flow is needlessly expensive and
hard to reason about.

Current read amplification:

- `events_for` reads the journal twice: `resolve_run` reads once, then `events_for`
  reads again.
- `repair_seed` and `recovery_seed` each parse it four times before reading a
  retained proposal.
- `append_locked` parses and verifies the whole journal for every appended event.
- an executed receipt performs one preliminary read followed by four appends;
  `record_proposal`, feedback updates, and provider-attempt batches have the same
  pattern.

### Read-side change

Add pure helpers that operate on an already-loaded journal:

- `resolve_run_in(&Journal, id)`;
- filtering events for a resolved ID;
- loading a proposal for an already-resolved ID.

Public operations should acquire/read once, resolve once, and reuse that state.

### Write-side change

Introduce a locked journal writer/session that:

1. acquires the existing history lock and performs legacy migration;
2. reads and verifies the journal once;
3. repairs a truncated final line once;
4. opens the append file once;
5. tracks the next sequence per run in memory;
6. appends one or more checksummed events and updates its in-memory journal.

Callers that need proposal counts, accepted attempt indices, or related IDs should
read them from the session's loaded journal. Keep the existing sync-after-event
behavior unless a separate durability decision changes it. Before pruning
atomically replaces the journal path, close only the append file while retaining
the session's lock guard. Prune from the updated in-memory journal under that same
lock; an open writer must never continue appending to the replaced inode.

Required tests:

- corruption still blocks all writes;
- truncated-tail repair retains complete events;
- a same-run batch receives strictly increasing sequence numbers;
- concurrent writers remain monotonic under the advisory lock;
- multi-event receipts retain their current event order;
- retained proposal and feedback linkage is unchanged.

### Linear pruning

`prune_locked` currently reads the journal twice. Save the original count before
moving its events.

Its byte-bound loop also serializes the retained set repeatedly and removes index
zero on every iteration. Measure or serialize each JSONL event once, subtract
oldest line sizes until the bound is met, drain the prefix once, and publish one
final encoding. This matters because configuration permits far more than the
default 500 records.

### Run-directory sweeping

A shared sweep is worthwhile, but it is not a mechanical extraction. `clear`
rejects a symlinked run root and handles stray files; `prune` and `clear_before`
do not. A common implementation should adopt the stronger policy:

- reject a symlinked root and symlink children;
- preserve directories containing `recovery.json`;
- preserve IDs still present in the journal;
- remove stray regular files without calling `remove_dir_all` on them.

Treat this as safety hardening. Preflight structural refusals such as a symlinked
root before publishing a rewritten journal, and define the publication/sweep
order explicitly. The operation cannot be fully transactional because entries can
race and deletion can fail. Add explicit partial-failure tests before changing the
three call sites.

Do not add a sidecar sequence index unless measurement shows that one verified
journal read per public operation is still a problem. A second durable authority
would add more complexity than it removes.

## Batch 6: reduce orchestration plumbing

Do this after the module graph and history API settle. It touches the most control
flow and should not be mixed with persistence changes.

### Proposal context in `command.rs`

The initial proposal and eight follow-ups pass the same stable arguments to
`propose`; only the follow-up payload changes.

Create a local `ProposalContext` or narrowly scoped session after the snapshot,
shell name, and run ID are available. Its method should take only
`Option<Value>` and return the existing proposal result. Keep these at callers:

- `Budget::replace_with_model`;
- replacement kind;
- `profile_allowed` assignment;
- conversion of provider failures to `app_error`;
- loop `continue` and terminal returns.

Do not couple budget mutation to network invocation. This removes the long
argument lists without hiding the two-call policy. Keep the context borrowing and
lazy: constructing it must not serialize, hash, touch the cache, or do other model
setup because aliases and preset actions can finish without requesting a proposal.

### Edited action constructors

Extract one constructor/validator for edited programs and one for edited shell
commands. Edited shell metadata can use
`ProposalMetadata { summary, ..Default::default() }`.

Preserve the current ordering exactly:

- program edits validate before `replace_with_edit`, so invalid source leaves the
  edit budget unused;
- shell edits consume the replacement after editor success but before action
  validation, so invalid edited shell input still consumes it.

### Terminal input

Move the exact `y`/`yes` confirmation primitive into `tty` and reuse it in the CLI
and command flow. A separate choice reader may share prompt, flush, read, and
lowercasing for the review menus.

Do not make confirmations case-insensitive unless that is an explicit UX change.

### Receipt context

The seven receipt sites repeat config, run ID, mode, context mode, and the stable
start instant. Group only those stable fields in a receipt context. At each call,
sample the elapsed duration and pass the current `budget.second_used()` along with
the outcome-specific route, decision, status, signal, and effects. The second-turn
state can change after a replacement and must not be captured when the context is
constructed. This removes positional-argument risk without merging program and
shell outcomes.

### Repair and recovery preparation in the CLI

The `repair` and best-effort `recover` setup blocks repeat operand-tail joining,
seed lookup, disclosure, terminal gating, confirmation, and argument mutation.
Use an enum or small specification object to share that preparation while keeping
their wording and error names distinct. Keep `recover --force` rejection outside
the helper.

### Management dispatch

After the reductions above, split the large management match into handlers by
domain: recovery/restore, history, telemetry/feedback, doctor/config/context.
Keep date parsing with history management.

This last step is for navigation and testability. Do not claim a LOC reduction,
and do not create one function per trivial match arm.

### Do not extract the full post-failure flow

The program and shell "Repair, edit, or stop?" blocks look similar, but their
privacy treatment, diagnostic payloads, failed-attempt recording, edited action
construction, and budget behavior differ. A generic helper would need callbacks
for nearly every consequential step.

Once proposal plumbing, choice reading, and edited constructors are shared, leave
the remaining branches explicit.

## Batch 7: provider, telemetry, and test cleanup

These changes are useful but lower leverage than the module graph, history, and
command work.

### Provider and model selection

- Let API invocation accept the candidate model/key/identity directly. The
  fallback path currently reconstructs a full `ApiConfig` just to build one
  invocation.
- Resolve an alternate qualification entry once, then read both its fingerprint
  and resolved model instead of searching the same vector twice.
- Inline the one-use `resolve_evidence` wrapper while preserving the sealed
  holdout lookup.
- Move the byte-identical 128-character metadata bounding helper from the OpenAI
  and Cerebras adapters into `provider`.

Keep provider response parsing and `ProviderResponse` construction in each
adapter. Token field names, finish semantics, API family, provider identity, and
tool-call shapes differ. A shared response builder would make those differences
harder to audit.

`capabilities::compatible_entry` has no in-tree caller, but it is public. Remove
it only after deciding that the published crate has no supported external Rust
API, or deprecate it before removal. Zero internal references alone do not make a
public item dead.

### Cache provenance

Replace the 20-positional-argument `key_hash_with_versions` test seam with direct
construction and hashing of the existing `Provenance` value. Keep a small builder
or test constructor only if tests need to substitute version fields. The serialized
field names and values must remain byte-stable so existing cache keys do not change
accidentally.

### Telemetry

- Extract a base event constructor for fields genuinely shared by interaction and
  feedback events.
- Replace terminal no-op `SendResult` matches with a `PreSend` check.
- Consider a scoped sender guard for open, exclusive lock, and policy recheck in
  `complete`, `feedback`, and `ack_parent`.

The sender guard should retain `send.lock` through the send. Inside
`flush_older_locked`, the separate `queue.lock` scope must end before network I/O.
Add an early-return lock-release test.

Do not assume `enum_or` falls back to `"unknown"`; it currently chooses the last
member of each allow-list. Changing that is a telemetry semantics change and
should use an explicit per-field fallback.

### Tests and worker

- Replace `configured` / `configured_fresh` in `tests/cli_contract.rs` with one
  command builder plus a freshness option.
- In the telemetry worker, compare event keys against a precomputed set instead
  of sorting a fresh copy of the expected keys for every element.
- Table-driving the handful of benchmark manifest file hashes is optional; the
  current explicit mapping is already easy to audit.

## Deferred or rejected abstractions

These ideas either change behavior or add more machinery than they remove:

- one kitchen-sink `dirs` or `fs_util` module containing paths, locks, IDs, PATH
  lookup, and shell policy;
- one supported-shell list for execution, integration, and telemetry;
- treating doctor's file-only PATH check as identical to executable discovery;
- folding program `create_new` files or shell-integration publication into the
  atomic replacement helper;
- deleting the provider-level non-2xx check as "unreachable";
- deriving `Effect::wire_name`, labels, and advisory policy from Serde without a
  clear single-source mechanism;
- replacing `contract::rejection_code` with an order-sensitive substring table;
- merging entire OpenAI and Cerebras response parsers;
- parameterizing recovery manifest scans that intentionally differ in whether
  malformed or overflowing entries fail closed, are reported, or are skipped;
- merging shell and program timeout loops;
- adding a closure-heavy generic failure-repair framework;
- counting a split of `management()` as source reduction.

`recovery::current_hash` and `current_mode` are not always called together. A
`current_state` result could still avoid reopening files at paired sites, but that
changes race/error timing in security-sensitive code. Consider it only as a later
hardening change with tests for missing files, mode changes, oversized files,
symlink swaps, and concurrent replacement.

## Product decision outside this refactor

Evidence-based model selection is dormant in the checked-in release: the
qualification manifest is empty and the holdout status is unavailable. Removing
evidence mode and its qualification pipeline would be a much larger simplification
than any refactor above, but it would also remove a documented release workflow
and configuration mode.

Decide that at the product level. Do not smuggle it into a cleanup patch. The same
rule applies to history, recovery, and telemetry as features.

## Recommended order and acceptance gate

1. Generated files and unreachable code.
2. One Rust module graph, then dead-code lint cleanup.
3. Small local reductions.
4. Shared private filesystem, ID, shell, policy, and HTTP invariants.
5. History read/write session and linear pruning.
6. Command and CLI orchestration plumbing.
7. Provider, telemetry, scripts, and test cleanup.
8. Optional management split and recovery hardening.

Run the full gate after every batch:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --locked
python3 scripts/provider-bakeoff.py --self-test
python3 -m unittest benchmark/test_benchmark.py benchmark/test_containment.py
python3 scripts/check-docs.py
(cd telemetry-worker && npm test)
```

For the module-graph, packaging, or qualification-script batches, also run:

```sh
cargo build --locked --bins
cargo package --allow-dirty --locked
```

For the qualification-support batch, also build the real worker image and run the
opt-in worker and containment tests. These cover the Docker `COPY` list, source
hash, and `uhm-bench-exec` bridge:

```sh
bash benchmark/build-worker.sh
UHM_BENCH_DOCKER_TESTS=1 \
  python3 -m unittest benchmark/test_benchmark.py benchmark/test_containment.py
```

The plan is complete when each retained abstraction has one explicit policy,
blanket dead-code suppression is gone, the Rust modules compile through one graph,
history operations parse once per lock/session, and the full gate remains green.
