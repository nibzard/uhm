# Plan 19 — Close the v0.6.4 review findings

> Executor handoff: read this entire plan before implementation. Follow the
> dependencies, keep each fix separate, and run its regression gate before moving
> on. Update the work-package status table and record actual verification results.
> This file contains the context from the review; no conversation history is
> required. This is a plan, not evidence that any fix has shipped.

## Status and scope of the request

- **Status:** TODO — planning complete; implementation has not started.
- **Planned at:** commit `7ee816f`, package version `0.6.4`, 2026-09-17.
- **Priority:** P1 for unintended probe execution and recovery durability; P2 for
  the remaining defects and dependency remediation.
- **Effort:** L overall. Individual estimates below include regression tests.
- **Risk:** MED overall; recovery reconciliation and subprocess cleanup require
  particular care.
- **Dependencies:** the capabilities already present at the planned commit.
  Historical Plans 10, 11, 13, 14, 16, and 18 provide design context; do not rerun
  their completed release or publication steps.
- **Deliverable:** one consolidated implementation plan covering all ten review
  findings, including the separate defects grouped within those findings, plus
  dependency remediation and integration coverage.
- **Authorization boundary:** this document does not request a version bump,
  release, deployment, live provider benchmark, production telemetry submission,
  publication of security findings, or modification of real user recovery data.

The architecture recommendation and three product-direction options are tracked
at the end. They are explicitly separate from closing the defects: adding
runbooks or conducting a paid model evaluation is not necessary to fix them.

### Drift check — run before implementation

From the repository root:

```sh
git status --short
git rev-parse --short HEAD
git diff --stat 7ee816f..HEAD -- src tests benchmark scripts telemetry-worker Cargo.toml Cargo.lock .github docs PRIVACY.md config.example.yaml
```

Inspect any existing uncommitted changes too; the commit diff does not include
them. Compare changed files against the excerpts in this plan before editing.
Changes made by earlier completed work packages are expected: record their commit
or diff and continue from that state. If unrelated changes invalidate a finding or
its proposed fix, stop that work package and report the discrepancy. Preserve
other people's changes.

## Product and compatibility constraints

The implementation is a Rust 2021 CLI, with MSRV **1.89**, supported on Linux and
macOS. Provider HTTP uses synchronous `ureq` and rustls. The telemetry gateway is
a small JavaScript Cloudflare Worker. The benchmark and qualification tools are
Python, with Rust helpers sharing production validation and execution logic.

Preserve these existing decisions:

1. One intent produces one bounded job. The ceiling remains two ordinary model
   calls, one optional machine-answered probe expansion, and two executions. No
   autonomous repair loop or extra user/model turns are added by this plan.
2. Effect detection is advisory, `--force` remains explicit authority, and there
   is no general sandbox or universal rollback claim. This does not authorize
   the host itself to run undisclosed positional action operands while probing.
3. Follow-ups remain stateless. Include the minimum previously displayed context
   needed to interpret a reply; do not introduce provider-hosted conversations.
4. `--local-input` bytes, child output, environment values, and retained private
   artifacts must not leak into a new model request or telemetry field.
5. Recovery snapshots remain separately disclosed and off by default. Verified
   undo is restricted to supported managed regular-file outputs with retained
   evidence. Per-file atomic rename does not become a multi-file transaction.
6. Preserve descriptor-relative validation, hash and mode checks, supported-file
   restrictions, retention deadlines, locking, and sticky forced-restore
   provenance. Do not resolve an interruption by silently enabling force.
7. Telemetry remains enum-only, identity-free, best effort, and subject to the
   existing first-use and opt-out gates. Worker invocation/payload logging stays
   disabled; that is a deliberate privacy decision.
8. The empty qualification manifest remains empty until a separate genuine
   holdout/audit authorizes entries. Synthetic regression fixtures cannot qualify
   a shipped provider. Do not lower qualification thresholds to make tests pass.
9. Preserve CLI grammar, output bytes/channels, exit statuses, current defaults,
   shell integration behavior, and backward-compatible reading of persisted
   state. Internal prompt/contract versions must track actual contract changes.

These constraints come from `PRODUCT.md`, `docs/behavior-contract.md`,
`docs/architecture/0003-content-free-telemetry.md`,
`docs/architecture/0004-bounded-evidence-recovery.md`, and
`docs/architecture/0005-provider-adapters-and-qualified-selection.md`.

## Coverage, priorities, and execution order

Every row starts TODO. Valid statuses are TODO, IN PROGRESS, DONE, and BLOCKED
with a concrete reason. A package is DONE only when its named regression cases
exist, pass, and are included in the relevant automated gate.

| Package | Review coverage | Work | Priority | Effort / fix risk | Depends on | Status |
| --- | --- | --- | --- | --- | --- | --- |
| W01 | Finding 1 | Remove action operands from automatic help probing | P1 | S / LOW | Baseline | DONE |
| W02 | Finding 2 | Complete managed-write and snapshot durability barriers | P1 | M / MED | Baseline | DONE |
| W03 | Finding 6a | Resume partial commits containing creations | P2 | S / LOW | W02 | DONE |
| W04 | Finding 6b | Reconcile undo interrupted after a filesystem mutation | P2 | M / MED | W02, W03 | DONE |
| W05 | Finding 3 | Preserve history-export parent permissions | P2 | S / LOW | Baseline | DONE |
| W06 | Findings 4 and 10 | Repair telemetry schema interoperability and bounded reads | P2 | M / LOW | Baseline | DONE |
| W07 | Finding 5 | Preserve the question in clarification follow-ups | P2 | S / LOW | Baseline | DONE |
| W08 | Finding 7a/7b | Bound Python inventory and context/help subprocesses | P2 | M / MED | W01 | TODO |
| W09 | Finding 8a/8b | Correct consent timing and unanswered-consent persistence | P2 | M / LOW | W01, W08 | TODO |
| W10 | Finding 9a/9b | Repair qualification resume and action-kind interoperability | P2 | M / LOW | Baseline | TODO |
| W11 | Dependency-scan results | Update rustls and affected Worker development dependencies | P2 | S–M / MED | Baseline | TODO |
| W12 | All findings | Gate the new boundaries in CI and synchronize documentation | P1 completion gate | M / LOW | W01–W11 | TODO |

Recommended order: W01, W02, W03, W04, W05, W06, W07, W08, W09, W10, W11,
W12. Independent small fixes may land earlier, but serialize W02–W04 in
`src/recovery.rs` and W01/W08/W09 in the probing path. W12 must include all
completed work rather than testing independently passing branches only.

### Scope boundaries

Only the following implementation paths are in scope, subject to each package's
narrower list:

- `src/tool_surface.rs`, `src/context.rs`, `src/runtime.rs`, `src/command.rs`,
  `src/main.rs`, `src/lib.rs`, `src/recovery.rs`, `src/history.rs`, `src/dirs.rs`,
  `src/prompt.rs`, `src/api.rs`, `src/model_selection.rs`, `src/capabilities.rs`,
  `src/contract.rs`, `src/config.rs`, `src/doctor.rs`, `src/program.rs`.
- `src/provider/openai.rs`, `src/provider/cerebras.rs`,
  `src/provider/deepseek.rs` for request-body tests only unless a demonstrated
  adapter change is required by W07.
- `src/bin/uhm-bench-contract.rs`, `src/bin/uhm-bench-exec.rs` for changed
  discovery signatures and the canonical qualification test boundary.
- `src/probe.rs` (new, if the narrow shared subprocess runner needs its own file).
- `tests/cli_contract.rs`, `tests/docs_examples.rs`,
  `tests/fixtures/action-validation-cases-v2.json` for necessary coverage.
- `scripts/provider-bakeoff.py`, `scripts/qualification_policy.py`,
  `scripts/provider-qualification-manifest.py`, `benchmark/test_benchmark.py`,
  `benchmark/schemas/run-event.schema.json` if checkpoint-schema compatibility
  requires it; `scripts/test-telemetry-contract.mjs` (new).
- `telemetry-worker/src/index.js`, `telemetry-worker/test/worker.test.js`,
  `telemetry-worker/queries.sql`, `telemetry-worker/README.md`,
  `telemetry-worker/package.json`, `telemetry-worker/package-lock.json`.
- `Cargo.toml`, `Cargo.lock`, `.github/workflows/ci.yml`,
  `.github/workflows/deps.yml`, `deny.toml` for the scoped changes below.
- Current documentation: `README.md`, `PRIVACY.md`, `CONTRIBUTING.md`,
  `config.example.yaml`, `docs/privacy.md`, `docs/configuration.md`,
  `docs/qualification.md`, `docs/behavior-contract.md`, `docs/troubleshooting.md`,
  `docs/reference/history.md`, `docs/reference/recovery.md`,
  `docs/explanation/trust-boundaries.md`,
  `docs/architecture/0003-content-free-telemetry.md`,
  `docs/architecture/0004-bounded-evidence-recovery.md`.
- This plan and its index entry in `plans/README.md`.

Out of scope: new providers, new action types, runbook implementation, changing
the sealed holdout or frozen qualification policy, expanding telemetry content,
rewriting the shell/program executor, unrelated render cleanup, changes to
historical release notes, release tagging, deployments, and issue publication.
The narrow optional module-graph follow-through has a separate scope below.

Use Conventional Commits for logical units if commits are requested, e.g.
`fix(tool_surface): restrict automatic help argv`. A suggested local branch is
`fix/review-correctness-v0.6.4`. Do not discard existing changes or push merely
because a work package passes.

## Baseline and verification conventions

The review at the planned commit passed formatting, Clippy, docs checks,
installer tests, Worker tests, provider self-test, and the offline Python suite.
Rust recorded 678 successful test executions: 310 library tests, 313 binary
tests, and 55 integration/example tests. The module graph duplicates 310 tests;
678 is not a count of distinct tests. The Python run had 27 tests with eight
opt-in skips. No live provider, deployed Worker, Docker containment, or physical
power-failure test was run.

Dependency scans were the known exception: rustls `0.23.43` triggered
RUSTSEC-2026-0285, and npm reported four affected development-dependency nodes
through Wrangler. Record their remediation under W11; do not confuse those
known failures with a broken functional baseline.

All commands below run from the repository root unless specified otherwise.

| Gate | Command | Expected result |
| --- | --- | --- |
| Formatting | `cargo fmt -- --check` | Exit 0 |
| Rust lint/type checking | `cargo clippy --all-targets --locked -- -D warnings` | Exit 0, no warnings |
| Rust tests | `cargo test --all-targets --locked` | All tests pass |
| Docs drift | `python3 scripts/check-docs.py` | Exit 0, documentation synchronized |
| Installer syntax | `sh -n docs/install.sh` | Exit 0 |
| Installer portability | `sh scripts/test-installer.sh` | `installer portability test: ok` |
| Worker tests | `node --test telemetry-worker/test/worker.test.js` | All tests pass |
| Provider runner | `PYTHONDONTWRITEBYTECODE=1 python3 scripts/provider-bakeoff.py --self-test` | `provider-bakeoff self-test: ok` |
| Python contracts | `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest benchmark/test_benchmark.py benchmark/test_containment.py` | Pass; record opt-in skips |
| Minimum Rust version | `cargo +1.89.0 check --all-targets --locked` | Exit 0 |
| Rust dependencies | `cargo deny --locked check` | Advisories, bans, licenses, and sources pass |
| Worker dependencies | `npm --prefix telemetry-worker audit --package-lock-only --ignore-scripts --audit-level=high` | No high/critical findings after W11 |

Use `--offline` for Cargo checks when dependencies are already available. Do not
treat failure to fetch a dependency or advisory database as evidence of a code
regression or a clean audit. Missing tools must be reported rather than silently
skipping their gate.

### Regression-test rules

- Follow the existing inline Rust `#[cfg(test)]` modules, `tempfile` fixtures, and
  `Result<_, String>` error propagation. The CLI harness in
  `tests/cli_contract.rs:9` already isolates HOME/XDG paths; use the same pattern.
- Prefix new Rust regressions with `audit19_`. Before accepting a filtered run,
  check its test list; **zero selected tests is a failure**, not a green gate.
- For each bug, add its reproducer first. On the baseline it must fail for the
  intended assertion, not a missing runtime, fixture, credential, or compilation
  error. After the implementation, the same test must pass.
- Use harmless temporary executables and isolated directories. Never exercise
  the positional probe issue against a real destructive utility, kill unrelated
  processes, or modify a user's actual recovery store.
- Subprocess timeout fixtures need an outer watchdog and cleanup of the exact
  process group they created. Do not leave sleeping descendants behind.
- Fault-injection hooks must be local/test-only, passed explicitly or isolated
  per test. Do not add production environment-variable switches that bypass
  durability or validation. Do not implement tests that merely search for a
  `sync_all` string; verify the production operation ordering and failure effects.
- Preserve unknown-field rejection, negative tests, and current corruption and
  concurrency checks. A successful happy path does not substitute for these.

**Baseline verification:** run the functional commands above once and record
results. Add scoped tests as each package starts, rather than writing all failing
tests into a shared branch at once. Run the complete final gate only after the
packages have been integrated, and again only if further changes warrant it.

## W01 — Restrict automatic help argv

**Scope:** `src/tool_surface.rs`; its existing inline tests. Touch
`src/command.rs` only if the consent wording needs to match the actual argv.

### Current state

`src/tool_surface.rs:128`:

```rust
const HELP_FLAGS: [&str; 3] = ["--help", "help", "-h"];
```

`probe` and `probe_subcommand_help` iterate these arguments. The caller in
`src/command.rs:173` obtains permission to run `--help` before any model proposal
or consequential-action review. A positional token is not generically a help
request. A harmless fixture confirmed that the fallback reaches an action branch.

### Implementation and tests

1. Extend the existing `fake_tool`/`consented_tool` fixture pattern in
   `src/tool_surface.rs`. Record exact argv in a temporary file. Make the
   advertised long help option fail or return empty stdout, and make any
   positional fallback set a harmless marker. Add top-level and subcommand cases.
   **Verify:** `cargo test --all-targets --locked audit19_help_` must select the
   new tests and fail on the original positional fallback.
2. Default generic probing to the explicitly disclosed `--help` argv only.
   Remove bare `help`; do not replace it with another guessed operand or claim
   that `-h` universally means help. If maintaining another form is necessary,
   use a closed, reviewed tool-specific mapping and disclose the exact probe;
   otherwise omit help for that tool. Keep direct argv, no shell, existing
   identity/consent checks, input closure, output limits, and subcommand tokens.
   **Verify:** `cargo test --all-targets --locked tool_surface::tests` passes;
   fixture logs contain only supported probe argv and no action marker exists.
3. Cover both nonzero-exit and successful-but-empty help, a working help response,
   unknown/unconsented tool, and invalid subcommand. Existing constant argv tests
   must assert the new deliberately smaller surface.
   **Verify:** `cargo test --all-targets --locked audit19_help_` passes on Linux
   and macOS without invoking real process-control commands.

**Done:** approval to inspect help cannot trigger a generic positional action.
Losing automatic context for an unsupported help convention is acceptable; adding
another execution/model call or a blanket list of guessed commands is not.

## W02 — Establish recovery durability before acknowledging writes

**Scope:** `src/recovery.rs`, regression coverage in its test module; inspect
`src/program.rs` for the call boundary, changing it only if required to enforce
the same production ordering. Preserve ADR 0004's per-file guarantees.

### Current state

`src/program.rs:762` chooses between two different commit paths:

```rust
let commit = if let Some(coordinator) = &mut recovery {
    for output in &mut staged {
        output.cleanup = false;
    }
    coordinator.commit(req.config.workspace_max_bytes)
} else {
    commit_outputs(staged, req.config.workspace_max_bytes)
};
```

The ordinary `commit_outputs` syncs a staged file before rename
(`src/program.rs:919`). `Coordinator::commit` opens and hashes it
(`src/recovery.rs:834–845`), renames it, and syncs the destination directory,
but never syncs staged contents. `resume_commit` has the same missing file sync.
Snapshot copying syncs its file at `src/recovery.rs:2320`; publication of the
manifest syncs the run directory, not the nested snapshots directory containing
the new snapshot names. File and directory synchronization are distinct; see the
[fsync documentation](https://man7.org/linux/man-pages/man2/fsync.2.html).

### Required ordering

```text
validated snapshot file written + file synced
  -> snapshot directory entries and newly created recovery ancestry synced
  -> snapshot-ready manifest durably published
  -> all staged output files validated, hashed, and file-synced
  -> all destination preimages revalidated
  -> commit-partial intent durably published
  -> one destination renamed
  -> destination directory synced
  -> that item's completion durably published
  -> repeat, then available manifest durably published
```

### Implementation and tests

1. Add a narrow test seam around the actual recovery sync/rename/publication
   operations. Test fixtures must execute the production coordinator and record
   ordering or inject a failure at a specific barrier. Match
   `multi_output_commit_preflights_every_destination_before_first_rename`.
   **Verify:** `cargo test --all-targets --locked audit19_durability_` fails for
   missing pre-rename file sync and missing snapshot-directory publication.
2. Sync each snapshot's containing directory after its file has been synced and
   before the manifest calls it ready. Durably link any directories this code
   created: snapshots into run, run into runs, and runs into the application data
   root, including newly created owned roots when applicable. Sync the immediate
   parent of each newly created directory; do not chmod unrelated ancestors.
   Propagate failures while original destinations are still unchanged.
   **Verify:** snapshot publication/failure tests pass; a snapshot-sync failure
   prevents program commit and retains an honest incomplete state.
3. Sync every opened staged output descriptor after validation and before the
   first destination rename, in both `Coordinator::commit` and `resume_commit`.
   Hashing consumes `File` today; arrange descriptor ownership so file syncing is
   not accidentally replaced with a directory sync or a racy path reopen.
   Keep all-output preflight ahead of the first mutation. Propagate destination
   directory-sync errors and preserve resumable journal state.
   **Verify:** `cargo test --all-targets --locked audit19_durability_` passes for
   normal commit, resumed commit, file-sync failure, directory-sync failure, and
   multi-output cases; no success receipt is issued after a required sync fails.
4. Check capture, commit, resume, and undo ordering together without weakening
   existing file identity, mode, hash, or filesystem checks.
   **Verify:** `cargo test --all-targets --locked recovery::tests` and
   `cargo test --all-targets --locked program::tests` pass.

**Done:** both normal and resumed managed commits implement the stated ordering;
all required sync failures remain visible. Describe testing as fault-injection
and ordering verification, not as proof from a physical power-loss experiment.

## W03 — Resume partial commits that create files

**Scope:** `src/recovery.rs` and its inline tests. Depends on W02.

### Current state

Creation records set `preimage_mode: None` (`src/recovery.rs:682`). Resume uses:

```rust
let preimage = item.existed.then_some(Identity {
    device: item.device,
    inode: item.inode,
    len: item.preimage_bytes,
    mode: item.preimage_mode.ok_or("preimage mode is missing")?,
    modified_seconds: item.modified_seconds,
    modified_nanoseconds: item.modified_nanoseconds,
});
```

`then_some` evaluates its argument eagerly. A valid creation therefore errors
before reaching resume. The current partial-commit test covers replacements.

### Implementation and tests

1. Extend `interrupted_partial_commit_resumes_only_hash_matching_items` with
   creation-only and mixed creation/replacement fixtures, before and after the
   first rename. Use valid durable hashes, modes, and item states.
   **Verify:** `cargo test --all-targets --locked audit19_resume_creation_`
   reproduces `preimage mode is missing` on the original implementation.
2. Construct the preimage in an explicit `if item.existed { Some(...) } else {
   None }` branch. Require complete preimage metadata for replacements only.
   Preserve atomic no-replace for creations, completed-item reconciliation, and
   all preflight checks and durability barriers established by W02.
   **Verify:** the filtered command passes and the final manifest is `available`
   with all expected output bytes and modes.
3. Add negative cases for a destination appearing concurrently, changed staging
   bytes, mismatched completed destination, and missing replacement preimage
   metadata. None may overwrite unrelated data or be reported as available.
   **Verify:** `cargo test --all-targets --locked recovery::tests` passes.

**Done:** new files and mixed output sets resume without inventing a preimage.

## W04 — Reconcile undo interrupted between mutation and journaling

**Scope:** `src/recovery.rs`, its inline tests, and a CLI regression in
`tests/cli_contract.rs` if management preview otherwise hides the repaired path.
Depends on W02 and W03.

### Current state

Restore replaces the destination at `src/recovery.rs:1431`, marks the item
`Restored` at 1443, and writes its manifest at 1491. A crash between mutation and
publication leaves `UndoPending` even though the bytes are already the preimage.
Retry only recognizes already-journaled `Restored`/`Removed` items; other items
are compared against their old postimage (`item_conflict`, line 1188). A valid
retry becomes a conflict and suggests force.

### Implementation and tests

1. Use the existing `committed_replacement` and partial-commit fixture patterns.
   Persist `UndoInProgress`/`UndoPending`, perform the authorized replacement,
   omit item completion, and invoke the production preview/restore path again.
   Add an interrupted creation-removal fixture and a two-item restore with only
   the first mutation complete.
   **Verify:** `cargo test --all-targets --locked audit19_undo_resume_` fails on
   the original false-conflict result.
2. Classify pending items under the recovery lock before ordinary conflict
   detection. Use a shared read-only classification for preview and the mutation
   path so the CLI does not refuse before reaching reconciliation. Rules:

   | Pending item observation | Resume behavior |
   | --- | --- |
   | Replacement still equals recorded postimage hash and mode | Perform normal verified restore |
   | Replacement equals validated retained preimage hash and preimage mode, and durable state proves undo was started | Sync parent, mark restored, persist; do not rewrite it |
   | Created destination absent, and durable state proves removal was started | Reconcile completed removal conservatively; handle any known quarantine before finalization |
   | Recorded completed item no longer equals its final hash/mode or expected absence | Refuse; preserve evidence |
   | Any other bytes, mode, symlink, unsupported type, missing snapshot, or invalid evidence | Keep ordinary conflict/unsupported handling |

   Do not accept a preimage match as prior undo in an `Available` manifest that
   has no durable undo intent. Sticky `forced_restore` remains sticky and is
   never converted to verified provenance.
   **Verify:** replacement, creation, mixed-output, mode-conflict, symlink, and
   forced-provenance cases pass in the filtered test command.
3. Handle creation quarantine interruptions explicitly. Current quarantine names
   include the process ID, which changes on retry. For new operations, persist an
   optional, strictly validated item-level removal intent containing the exact
   private sibling name before moving the destination. Give the new field a
   backward-compatible default; test reading existing schema-v1 manifests.
   On retry, validate the named quarantine descriptor and recorded hash/mode
   before unlinking, sync its parent, then persist removal. Do not scan/delete
   arbitrary `.uhm-*` files or infer ownership from a prefix. For legacy pending
   creation removals lacking that linkage, reconcile a verified absent
   destination without deleting unknown sibling files, and report any limitation
   honestly. Document additive-state compatibility and older-binary limitations.
   **Verify:** add interruption cases immediately after quarantine rename and
   after unlink, plus mismatched/forged quarantine linkage. The filtered command
   passes; outside files and unknown siblings remain untouched.
4. Preserve all-output preflight: validate every unresolved item before applying
   another filesystem mutation. Persist newly recognized completed items with
   W02's ordering; repeated retry must be idempotent.
   **Verify:** `cargo test --all-targets --locked recovery::tests` and
   `cargo test --test cli_contract --locked` pass. A second verified retry needs
   no force and does not duplicate successful mutations or completion events.

**Done:** authorized work interrupted after its filesystem effect can resume
from evidence. Stop this package if implementation requires a schema migration
that cannot read old state; specify that migration before proceeding.

## W05 — Preserve existing export-directory permissions

**Scope:** `src/history.rs`, `tests/cli_contract.rs`; `src/dirs.rs` only for a
specifically named helper if needed. Do not weaken private application storage.

### Current state

`history::export` passes an arbitrary absolute output path to the application's
private-file writer (`src/history.rs:1487`). That writer calls:

```rust
let parent = path.parent().ok_or("history path has no parent")?;
dirs::ensure_private_dir(parent)?;
```

`ensure_private_dir` unconditionally sets the parent to `0700`. The review
reproduced a successful export changing a preexisting `0755` directory to `0700`.

### Implementation and tests

1. Add `audit19_export_preserves_existing_parent_permissions` to
   `tests/cli_contract.rs`, using the `configured_command` fixture with private
   test HOME/XDG paths and a separate existing export directory. Seed known
   receipts, export, and assert parsed contents, successful status, unchanged
   parent mode, and a `0600` export file. Cover both `0755` and `0770` parents.
   **Verify:** `cargo test --test cli_contract --locked audit19_export_` fails
   on parent-mode preservation before the fix.
2. Separate initialization of application-owned directories from publication of
   a private output file into a user-selected directory. Keep same-directory
   temporary creation, file sync, atomic replacement, and parent-directory sync.
   Preserve the existing creation behavior for missing destination directories,
   making only newly created directories private; never chmod existing ancestors.
   Leave the existing private-storage writer behavior intact for journal files.
   **Verify:** the filtered tests pass, including a parent reached through a
   directory symlink; its real existing directory's mode is unchanged.
3. Cover a missing parent, unwritable parent, existing output replacement, and
   an error before publication. Failure must not truncate the old output or
   alter existing directory permissions.
   **Verify:** `cargo test --all-targets --locked history::tests` and
   `cargo test --test cli_contract --locked audit19_export_` pass.

**Done:** privacy is applied to the export file and newly created owned paths,
without changing access to a directory the user already owns and uses.

## W06 — Align telemetry schemas and stop reading at the body limit

**Scope:** `src/telemetry.rs` tests if needed; Worker source/tests/queries/docs;
new `scripts/test-telemetry-contract.mjs`; the telemetry CI job; current privacy
documentation for accuracy. Keep the existing deployed endpoint and logging
configuration unchanged.

### Current state

The Rust event always serializes this field (`src/telemetry.rs:89`):

```rust
#[serde(default = "default_expansion")]
pub expansion_outcome: String,
```

The Worker still defines `KEYS_V2 = [...KEYS_V1, "parent_action"]` and rejects any
other exact key set. Feeding an actual `uhm telemetry preview` into the local
handler returned 422 and wrote zero points; deleting only `expansion_outcome`
made validation succeed. HTTP rejection is intentionally dropped by the client,
so this is an interoperability fix, not a request to make telemetry reliable or
retry indefinitely.

The Worker also currently does:

```javascript
const bytes = await request.arrayBuffer();
if (bytes.byteLength >= MAX_BODY) return response(413);
```

An 8 KiB stream without Content-Length was entirely consumed before returning
413 even though `MAX_BODY` is 2048. Body conversion buffers the request; see the
[Cloudflare Request API](https://developers.cloudflare.com/workers/runtime-apis/request/#instance-methods).

### Implementation and tests

1. Create `scripts/test-telemetry-contract.mjs` taking the built CLI path as its
   sole argument. Resolve the path before changing directories. Run
   `telemetry preview` in a temporary HOME/XDG environment with `DO_NOT_TRACK=1`,
   parse stdout, and invoke the imported Worker `handle` in-process with mocked
   rate-limit and Analytics Engine bindings. Never send an HTTP request to the
   live endpoint. Assert 202, exactly one point, the expansion value, existing
   column positions, and no unexpected event fields. Clean up in `finally`.
   **Verify:** `cargo build --locked --bin uhm`, then
   `node scripts/test-telemetry-contract.mjs target/debug/uhm` fails with the
   existing Worker and current real CLI payload.
2. Accept exactly three released shapes: v1, legacy v2 with `parent_action`, and
   expanded v2 with both `parent_action` and `expansion_outcome`. Do not silently
   allow arbitrary optional/unknown keys. For expanded v2 require one of
   `none`, `probed`, `probe_empty`, `invalid_probe`; reject all other values and
   types. Normalize absent legacy expansion to `none` in the point projection.
   This preserves already-shipped expanded-v2 clients and queued events without
   relabeling them as another schema. Preserve existing notice-revision rules.
   **Verify:** `node --test telemetry-worker/test/worker.test.js` passes old and
   new schema cases, unknown-field rejection, and invalid expansion variants.
3. Append expansion as a new Analytics Engine blob after all existing blobs;
   preserve blob1 through blob14, existing doubles, and indexes. Document the new
   blob15 in `queries.sql` and add a coarse expansion aggregation example.
   Keep historical samples meaningful when the new column is absent.
   **Verify:** Worker projection tests assert old offsets and the appended value;
   the real-CLI interoperability script now passes. No live query is required.
4. Replace unbounded buffering with a reader that counts bytes incrementally.
   Keep the existing **strictly below 2048 bytes** contract. Reject declared
   oversized bodies early, but enforce the same cap when the header is absent
   or inaccurate. On reaching the limit, stop pulling, cancel/release the reader,
   and return 413 without parsing or writing. Do not accumulate beyond the
   accepted prefix; a single already-delivered oversized chunk must be rejected
   immediately. Map malformed/read-failed bodies to a bounded error response,
   without logging their bytes.
   **Verify:** add streaming tests for 2047-byte valid JSON with whitespace,
   2048-byte and larger bodies, absent length, misleading small length, a large
   first chunk, invalid JSON, and reader failure. Assert no writes on rejection,
   cancellation, and that later chunks are not drained after overflow. Run the
   Worker tests; all pass.
5. In the Worker CI job, install Rust, build `uhm` from the same checkout, run the
   existing Node tests and the real-CLI script. Keep this script separate from
   `cargo test` so Rust crate tests do not acquire a hidden Node prerequisite.
   Update current privacy docs only to match the already-disclosed enum field;
   do not expand content or enable connection logging.
   **Verify:** `node scripts/test-telemetry-contract.mjs target/debug/uhm`,
   `node --test telemetry-worker/test/worker.test.js`, and
   `python3 scripts/check-docs.py` all pass.

**Done:** all supported released payloads pass the same strict ingestion
boundary; oversized streaming input is stopped at the boundary rather than
buffered to completion. Record a deployment handoff: the backward-compatible
Worker must be deployed before relying on telemetry from corrected releases.
Deployment is a later operational step, not part of executing this local plan.

## W07 — Carry the clarification question into the final call

**Scope:** `src/command.rs`, `src/prompt.rs`, and offline request-body tests in
`src/provider/*`/`src/api.rs` as needed. No provider calls or conversation storage.

### Current state

`src/command.rs:431` sends:

```rust
Some(json!({"kind":"clarification","answer":answer}))
```

`prompt::proposal_input` includes the original intent, current context/stdin, and
this follow-up in a fresh request. There is no previous-response link. The
question displayed immediately before the answer is therefore absent.

### Implementation and tests

1. Add a small production follow-up constructor if needed to make this boundary
   directly testable. Use a question containing two named alternatives and an
   answer such as “the second one.” Build the actual provider request through
   the existing prompt/adapter methods with a dummy credential; do not call a
   transport. Assert the original question, answer, and original intent survive.
   **Verify:** `cargo test --all-targets --locked audit19_clarification_` fails
   because the question is missing on the old request path.
2. Include a `question` field alongside `kind` and `answer`. Keep question and
   answer in the untrusted JSON data layer, not developer instructions. Preserve
   Unicode, existing question bounds, total request-size checks, and all
   `--local-input` restrictions. Do not add diagnostics, child output, retained
   history, or a previous-response ID. Increment the prompt contract version
   once for the changed follow-up semantics, and update affected version
   assertions/cache expectations; leave the release version and empty manifest
   alone.
   **Verify:** the filtered request tests pass for all three adapters and confirm
   `store: false` where applicable and unchanged instructions.
3. Cover yes/no and option-index answers, control-character display handling,
   request overflow, missing-terminal cancellation, and exhaustion of the one
   replacement slot. A clarification answer still consumes exactly that slot.
   **Verify:** `cargo test --all-targets --locked command::tests`,
   `cargo test --all-targets --locked prompt::tests`, and
   `cargo test --all-targets --locked provider::` pass.

**Done:** the last allowed model call has enough context to understand the
answer, without creating a hidden conversation or broadening outbound data.

## W08 — Bound discovery subprocesses and reuse runtime inventory

**Scope:** `src/context.rs`, `src/runtime.rs`, a narrow new `src/probe.rs` if
needed, module declarations in `src/lib.rs`/`src/main.rs`, and discovery callers
in `src/main.rs`, `src/command.rs`, `src/tool_surface.rs`, `src/doctor.rs`, and
the benchmark helper binaries. Do not refactor `src/shell.rs` or rewrite the
program executor. Inspect their cleanup patterns as examples only.

### Current state

Python inventory runs `command.output()` without a timeout (`src/runtime.rs:60`)
and trims the output only afterward. Normal dispatch inventories Python at
`src/main.rs:414`, and `context::gather` repeats it at `src/context.rs:64` before
starting its context deadline. A sleeping Python shim blocked even a local alias.

The context runner waits for process exit before reading the pipe:

```rust
if let Some(status) = child.try_wait().ok()? {
    // stdout is read only here, after exit
    child.stdout.take()?.take(4096).read_to_string(&mut out).ok()?;
    return status.success().then(|| out.trim().into());
}
```

Large output can block the child before it exits. An exited child with a
descendant holding stdout can instead block the subsequent read past the
deadline. The review reproduced both. Git status errors also currently become
`dirty: false` through `unwrap_or(0)`.

### Implementation and tests

1. Add harmless fixtures for a hanging executable, output larger than a pipe,
   a child that exits while a descendant holds stdout, missing/non-executable
   paths, nonzero exits, and valid short output. Use explicit paths and isolated
   test process groups, not global mutation of PATH in parallel unit tests.
   **Verify:** new `audit19_probe_` tests fail or hit their outer watchdog on the
   old collector. Tests themselves must terminate and clean up.
2. Implement a narrowly shared direct-argv probe collector. Preserve closed
   stdin and discarded stderr. Drain stdout while the child runs using
   nonblocking descriptors or a cancellable bounded reader; retain only the
   capped prefix and track truncation. Keep the same absolute deadline through
   child wait and pipe drainage. On timeout/error, close readers, terminate only
   the owned process/group, reap the child, and apply a small explicit bounded
   cleanup allowance. Do not leave an unbounded reader-thread join or post-exit
   `read_to_end`. Preserve correct handling of SIGINT/SIGTERM; avoid introducing
   another competing process-wide signal owner.
   **Verify:** `cargo test --all-targets --locked audit19_probe_` passes with
   assertions for elapsed bound, output cap, correct exit handling, and cleanup.
3. Route `context::run` and both help-probe paths through the bounded collector.
   Keep the context deadline and 4096-byte capture limit. A failed Git-status
   probe must produce unavailable/null Git context, not assert that the tree is
   clean. Keep valid empty status as clean and valid nonempty status as dirty.
   Treat truncated captured status as bounded evidence, never an exact complete
   file count. Update context policy/version assertions only if its outward
   representation or contract changes, with corresponding cache invalidation.
   **Verify:** `cargo test --all-targets --locked context::tests` and
   `cargo test --all-targets --locked tool_surface::tests` pass, including Git
   clean/dirty/failed/truncated cases using a fake Git executable.
4. Give Python inventory its own short deadline (start with a documented 500 ms
   internal limit) and a small byte cap. Timeout, excessive output, malformed
   version, or nonzero exit means unavailable inventory, with no fallback to
   an unbounded call. Keep the cleared environment, minimal PATH, and `-I -S`.
   Reuse the same inventory value between request-class selection and the
   request snapshot; pass it explicitly rather than caching globally across
   changing PATH/configuration. Standalone doctor/context/benchmark entrypoints
   each obtain one bounded result. Preserve the public no-argument inventory
   entrypoint if helpers need it, delegating to the bounded implementation.
   **Verify:** `cargo test --all-targets --locked runtime::tests` and
   `cargo test --test cli_contract --locked audit19_runtime_` pass. A sleeping
   shim no longer prevents a local shell alias from completing; its invocation
   counter proves the normal request path does not probe Python twice.
5. Check module declarations in both current Rust roots and all direct callers.
   Use existing execution helpers as references, not a reason to merge probe,
   shell, and program policies into one broad abstraction.
   **Verify:** `cargo clippy --all-targets --locked -- -D warnings` and
   `cargo test --all-targets --locked` pass. Host scheduling tolerance in tests
   must be explicit; do not claim precise real-time behavior under a blocked OS.

**Done:** Python and context/help discovery have bounded output and waiting,
unavailable discovery remains distinguishable from a successful observation,
and the normal invocation uses one runtime inventory result.

## W09 — Separate human consent from machine probe budgets

**Scope:** `src/tool_surface.rs`, `src/command.rs`, their tests,
`tests/cli_contract.rs`, and current trust/context documentation. Depends on W01
and W08; do not restore removed probe fallbacks while touching these paths.

### Current state

The deadline is constructed at `src/command.rs:177` before invoking the consent
callback. Its default budget is 150 ms (`src/config.rs:288`). `surface` waits for
the user at `src/tool_surface.rs:255`, then probes with the expired deadline.
When there is no terminal or `--json` is set, the callback returns false; the
store persists that as a rejection even though nobody answered.

### Implementation and tests

1. Change the callback result from a boolean to an explicit internal decision:
   `Allow`, `Decline`, or `Unavailable`. The command caller returns Unavailable
   when prompting is impossible; it must not call that a user's decline. Store
   only actual Allow/Decline decisions. Explicit decline continues suppressing
   probing and repeated prompts for the same identity.
   **Verify:** `cargo test --all-targets --locked audit19_consent_` covers an
   unavailable first encounter followed by an interactive encounter, explicit
   decline reuse, and explicit allow reuse. Only the actual answers persist.
2. Pass a machine-time budget rather than a deadline created before user input.
   Track remaining probe time across the at-most-three tools and their permitted
   attempts. Suspend accounting while waiting for consent; create each probe's
   absolute deadline after consent from the remaining budget. Do not grant a
   fresh full budget per tool, and keep the one subcommand-expansion ceiling.
   **Verify:** a delayed affirmative callback longer than 150 ms still yields
   help from a quick fixture in that invocation; slow probes exhaust the shared
   machine budget. The `audit19_consent_` tests pass without increasing the
   default context timeout to hide the problem.
3. Migrate the store explicitly. Bump its local format version and read the
   existing v1 shape. Preserve allowed records and compatible retained help.
   Legacy false records have unknowable provenance: drop them back to unknown
   and re-ask once when interactive, never silently allow them. Document this
   one-time migration behavior. New explicit declines must survive reload.
   Unknown/corrupt versions remain conservative. Preserve private permissions
   and existing identity checks. If migration changes publication, use a unique
   private same-directory temporary file, not a fixed shared temp filename.
   **Verify:** fixtures for v1 true/false, new true/false, corrupt JSON, and
   unknown versions pass; false records cannot authorize any probe.
4. Cover the CLI's noninteractive and JSON pathways, as well as the pure callback
   logic. Reuse the existing PTY harness where a real affirmative is necessary.
   **Verify:** `cargo test --test cli_contract --locked audit19_consent_` and
   `cargo test --all-targets --locked tool_surface::tests` pass.

**Done:** first consent actually contributes help, absence of a prompt does not
become a permanent decision, and machine time remains bounded independently of
human response time.

## W10 — Make qualification artifacts resumable and usable by Rust

**Scope:** `scripts/provider-bakeoff.py`, `scripts/qualification_policy.py`,
`scripts/provider-qualification-manifest.py`, `benchmark/test_benchmark.py`,
`src/contract.rs`, `src/capabilities.rs`, `src/model_selection.rs`,
`src/bin/uhm-bench-contract.rs`, and focused tests in `src/api.rs`. Use the
existing `BenchmarkTests.qualification_fixture` and Rust contract bridge.

### Current state: checkpoint reconciliation

The resume loop compares candidate and judged records after dropping only two
derived fields (`scripts/provider-bakeoff.py:1687`):

```python
base = {name: value for name, value in latest[key].items()
        if name not in {"judgments", "synthetic_outcome"}}
judged_base = {name: value for name, value in record.items()
               if name not in {"judgments", "synthetic_outcome"}}
if base != judged_base:
    raise ValueError(f"conflicting resume event key: {key}")
```

Judging later adds `semantic_acceptable` for semantic records (line 1852), so an
ordinary checkpoint cannot resume. The qualification workflow deliberately
pauses for independent audit and requires that resume (`docs/qualification.md:74`).

### Current state: action-kind disagreement

`qualification_policy.evaluate` copies wire tool names from
`route_oracle.allowed` into `permitted_action_types` (line 278). The manifest
generator copies them unchanged (line 266). Rust's `model_selection::action_type`
returns different strings; API acceptance compares them literally. A generated
qualified profile with `run_shell` therefore rejects a runtime `shell` action.

| Wire tool | Canonical runtime action kind |
| --- | --- |
| `return_answer` | `answer` |
| `request_clarification` | `clarification` |
| `run_shell` | `shell` |
| `run_program` | `program` |
| `require_parent_shell` | `parent_shell` |

`probe_subcommand` is a routing step and remains outside executable evidence
profiles. The intentionally empty shipped manifest currently prevents these
latent runtime-selection bugs from affecting fixed-mode jobs.

### Implementation and tests

1. Extract the existing resume reconciliation into a small production helper
   used by the runner, retaining fingerprint, sequence, and duplicate-event
   enforcement. Define the immutable candidate projection explicitly, excluding
   exactly the supported judgment-derived fields: `judgments`,
   `synthetic_outcome`, and `semantic_acceptable`. Validate/recompute the semantic
   verdict from its judgments so excluding a field does not permit tampering.
   Do not ignore arbitrary new fields or nested changes to candidate evidence.
   **Verify:** add Python tests for candidate -> semantic judgment -> audit pause
   -> resume for both answer and clarification. They fail on the old comparison
   and pass on the helper actually used by the runner.
2. Add negative resume cases for changed action bytes, candidate/model identity,
   execution/oracle evidence, task/trial, reordered/duplicate events, stale
   fingerprints, and an inconsistent derived verdict. Resume must reuse prior
   completed work, not call a provider again. Mock only provider/worker I/O;
   exercise real checkpoint serialization and production reconstruction.
   **Verify:** `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest benchmark/test_benchmark.py`
   passes with the new resume tests. The test transport's unexpected-call path
   fails immediately if resume repeats completed work.
3. Make Rust authoritative for the mapping above. Expose a closed
   `tool_to_action_kind` mapping through the private contract helper's `describe`
   response; Python consumes that mapping rather than keeping another unchecked
   handwritten table. Tie each pair in a Rust test to a decoded canonical action
   and `model_selection::action_type`. Reject unknown wire names or manifest
   action kinds instead of guessing or treating them as unrestricted.
   **Verify:** `cargo test --all-targets --locked audit19_qualification_` checks
   all five mappings and unknown-name rejection. Existing describe consumers
   and provider self-tests still parse the expanded private helper contract.
4. Map qualification profiles before manifest publication. Test the actual
   Python-generated profile/entry against the Rust validator and the same
   membership predicate used by `api::request`. If necessary, extract that
   predicate into `model_selection` and expose a bounded private helper operation
   for the cross-language test. Do not test a second Python approximation of it.
   Include valid and out-of-profile examples for all five kinds, unknown kinds,
   and the empty-manifest fail-closed path.
   **Verify:** the Python suite, `cargo test --all-targets --locked audit19_qualification_`,
   and `PYTHONDONTWRITEBYTECODE=1 python3 scripts/provider-bakeoff.py --self-test`
   pass. A permitted generated `shell` action is accepted and an unpermitted
   kind remains rejected.
5. Bump private contract version/fingerprint inputs if their payload semantics
   change, using existing version mechanisms. Do not edit the frozen policy,
   seal a holdout, insert a synthetic qualified entry, or bypass source hashes
   to rescue an artifact generated by an older runner. Such historical migration
   would require its own reviewed conversion and evidence decision.
   **Verify:** the production `model-qualification-manifest.json` still has zero
   entries; version assertions and all benchmark helper builds pass.

**Done:** checkpoints written by the corrected runner survive the documented
audit pause, and profiles generated by Python are recognized by actual Rust
selection/acceptance logic without weakening qualification.
