# Plan 7 — Add bounded, evidence-based recovery

## Purpose and dependency

This post-release plan adds hash-verified restoration against a captured preimage for a narrow class of file outputs that `uhm` manages through Plan 4's staging path, then offers a separately labeled best-effort inverse proposal for other receipted actions. It depends on Plan 4's declared input/output manifests and Plan 5's event journal and per-run artifact ownership.

The vocabulary is a product contract: `undo` means a verified restore backed by retained bytes and hashes. `recover` means a model-proposed, best-effort new action that the user must review. When neither has defensible evidence, `uhm` says recovery is unavailable. History alone never makes an arbitrary shell, remote, package, database, or destructive action reversible.

## Full implementation description

### 1. Define recovery eligibility before taking snapshots

At proposal validation, classify every declared effect as:

- `verified_restore_eligible`: a size-bounded, current-user-owned, single-link regular-file output on a supported local filesystem, committed by `uhm` from Plan 4's same-filesystem sibling staging path.
- `best_effort_only`: an action with a useful receipt but no controlled preimage, such as many Git, package, shell, or remote mutations.
- `unavailable`: an effect without a defensible inverse or adequate evidence, including unsnapshotted deletion, secret exposure, sent messages, and unknown external state.

Initial verified restore supports only current-user-owned regular files with link count one, no ACLs or extended attributes/resource forks, and sizes below the configured snapshot limit on filesystems where sibling rename behavior has passed the platform tests. V1 restores file bytes and Unix permission bits; it does not promise original ownership, timestamps, ACLs, extended attributes, resource forks, sparse layout, or hard-link identity. Symlinks, devices, sockets, directories, hard-linked files, remote/unknown mounts, and unsupported metadata are ineligible. State this before execution. `--force` may let a user continue with an ineligible Plan 4 action, but its receipt must say that verified restore will not exist.

Do not call an action recoverable merely because its command text appears reversible. Store the classification and reason in the Plan 5 run timeline.

Recovery capture is a separate, explicit content-storage choice. Default `recovery.enabled` to false, because snapshots duplicate user file bytes even when detailed history is disabled. A user may run `uhm recovery on` after seeing the snapshot classes, paths, proposed retention/byte limits, and disable/clear commands, or select `--recoverable` for one job. Enabling recovery requires at least Plan 5's metadata journal for durable linkage but must not silently enable diagnostic/full history. If history or recovery is disabled, no new snapshot, recovery manifest, or recovery event is created and the job remains executable under the ordinary policy with `recovery=unavailable` shown before a consequential managed overwrite.

### 2. Snapshot eligible preimages before executing the program

Extend Plan 4's artifact commit coordinator with a staged recovery manifest:

```text
<data-dir>/uhm/runs/<run-id>/
  recovery.json
  snapshots/
    <output-id>.preimage
```

For each eligible destination, record the descriptor-resolved destination, whether it previously existed, preimage content hash, staged content hash, committed postimage hash, permission bits, byte size, and snapshot state. Hash using the project's established cryptographic digest implementation. Snapshot bytes stay private (`0600`) under a validated run directory and are never included in telemetry or an OpenAI request.

Before the generated program starts:

1. Open and validate the destination parent descriptor-relatively; reject symlinks, traversal, wrong owner, multiple links, unsupported metadata/filesystems, and paths beyond the size limit.
2. If the destination exists, copy its regular-file bytes and permission bits to a private snapshot, fsync it, verify its hash, and durably record `preparing`. If it is absent, durably record the nonexistence marker. Snapshot failure stops execution by default because no later capture can prove the pre-program state.

After the program succeeds in its supplied staging paths:

3. Validate and hash every staged output.
4. Immediately before commit, reopen existing destinations and verify identity, metadata, and hash still match the pre-execution snapshot; abort on a detected program side effect or concurrent change. Recheck that absent destinations are still absent.
5. For an existing destination, commit the sibling staged output with same-filesystem rename. For a previously absent destination, use a platform-tested atomic no-replace primitive so a concurrent creator is never overwritten; if that primitive is unavailable, creation is ineligible for verified recovery. Never describe copy as atomic.
6. Fsync the committed file and parent, read/verify the postimage hash, and append the committed recovery event only after verification.

For a newly created file, its verified inverse begins only after `uhm` observes the recorded postimage immediately before removal and remains subject to the documented concurrent-writer window. A snapshot, revalidation, no-replace, or verification failure removes verified eligibility. It stops execution or commit at the point detected and explains the choice; a user may explicitly continue under normal `--force` authority, recorded as unrecoverable. If generated Python ignored staging and changed a destination, the snapshot remains evidence but that run is not mislabeled as a clean managed commit.

Multi-output commits cannot be described as filesystem-wide atomic. Preflight every output before the first commit, journal each transition, and retain enough postimage data to resume or report a partial commit after interruption. `undo` reports success only when every eligible output in the selected recovery set has been restored and hash-verified at the completion check.

### 3. Reserve `uhm undo` for verified restore

Provide:

```text
uhm undo <run-id|last> [--review]
uhm restore <run-id|last> --force
```

The command is available only when the referenced run has a complete retained recovery manifest. It previews destinations, create-versus-replace operations, snapshot sizes, and any conflicts. Before changing anything, preflight every destination:

- A replaced file's current hash must equal the recorded postimage hash.
- A newly created file must still exist as a regular file with that hash.
- Snapshot hashes, ownership, modes, paths, and manifest/run linkage must validate.

If preflight passes, create and fsync a sibling temporary file in each replacement destination directory, restore the captured bytes and permission bits, then rename it into place and fsync the parent. For a created output, atomically move the current path to a unique sibling quarantine, hash it there, and delete it only if it matches the recorded postimage; on mismatch, retain the bytes and restore without clobbering any newly created destination or report the exact conflict. Verify final hashes/absence and append a new linked `undo_started|undo_item_finished|undo_finished` job timeline. An interrupted multi-output restore is resumable from its item states. Each supported rename is atomic, while the collection is not one filesystem transaction.

There is no portable filesystem compare-and-swap between the final preflight hash and rename. “Verified” therefore means `uhm` observed the recorded postimage immediately before replacement and the recorded preimage immediately after its operation. A concurrent writer can race that window; detected pre/post mismatches make the run conflicted, but this feature is not concurrency-proof. State that limitation in review copy and documentation.

A mismatch means later work exists. `uhm undo` must refuse to call that a verified restore and direct the user to inspect, start `uhm recover`, or explicitly run `uhm restore <run-id> --force`. The latter previews the conflicting current hashes and overwrites only the supported destinations with retained snapshot bytes under a `forced_restore` outcome; it never reports conflict-free `undo` or semantic recovery. This preserves the user's authority without corrupting the meaning of verified undo.

Undoing an undo is not part of the initial contract. The undo job records enough facts for diagnosis, but no chain is offered until cycles, retention, and partial restoration are designed explicitly.

### 4. Make best-effort recovery a new reviewed job

Provide one explicit command:

```text
uhm recover <run-id|last> [-- <guidance>]
```

`recover` is not an alias for `undo`. It extracts the smallest relevant receipt subset, previews exactly what will be sent to OpenAI, and requests one strict normal action proposal labeled `best_effort_inverse`. It never sends the full journal, unrelated runs, snapshots, file contents, or unbounded output. If the retained receipt lacks sufficient evidence, say so rather than inventing an inverse.

The proposed inverse is always shown in review mode before execution. Once accepted, it goes through current context, schema validation, effect detection, consequential warnings, confirmation, `--force`, execution, and history recording as a new run linked to the original. The receipt distinguishes “proposal executed” from “original state recovered”; process success alone cannot verify remote or semantic restoration.

Each `recover` invocation has its own global limit of two model calls and two executions. Its one second-turn slot may be used by clarification, model revision/repair, or a post-failure local replacement, never more than one of them. Do not automatically execute, retry, chain inverses, or recursively recover a recovery job.

### 5. Add bounded snapshot retention and honest expiry

Add configuration and commands for snapshot age, total bytes, and per-file maximum:

```text
uhm recovery on|off
uhm recovery status [<run-id|last>]
uhm recovery prune [--dry-run]
```

`recovery on` persists the separate enablement only after its disclosure; `off` prevents new capture immediately and asks whether retained snapshots should be kept until expiry or pruned now. Status reports disabled, eligible, available, conflicted, expired, partially restored, completed, or unavailable with a literal reason. Retention values must be visible in `history status` as well, but Plan 7 owns snapshot pruning. Never let generic history pruning leave a manifest claiming that a missing snapshot is available; coordinate deletion through typed tombstone events.

Candidate limits must be benchmarked against real Plan 4 workloads before becoming defaults. Prune in bounded slices, oldest eligible snapshots first, while respecting active operations and locks. Preview explicit prune operations and allow users to pin selected snapshots within the configured total cap. There is no unbounded retention default and no background cleanup daemon.

Snapshots contain user data. Local export excludes them by default; telemetry and model-request types cannot reference their bytes or paths. `history clear --all` and recovery-specific prune share validated descendant/path and private-permission code, but neither reaches outside the application data root.

### 6. Build around a crash-aware state machine

Implement recovery as explicit states rather than boolean flags: `none`, `preparing`, `available`, `commit_partial`, `undo_preflight`, `undo_in_progress`, `restored`, `conflicted`, `expired`, and `corrupt`. Illegal transitions fail closed with a diagnostic. On startup, do only a bounded check of interrupted recovery operations; full reconciliation belongs to `recovery status`.

Add a focused recovery module that integrates with Plan 4's staging coordinator and Plan 5's journal. Expected code areas include artifact/staging modules from Plan 4, `src/history.rs`, `src/hash.rs`, `src/dirs.rs`, `src/args.rs`, `src/command.rs`, `src/config.rs`, `src/render.rs`, and new snapshot, recovery-manifest, and restore modules.

Document the exact guarantees by file type and platform, failure recovery after process death, backup expectations, and the difference among verified restore, best-effort recovery, and unavailable.

## Expected outcomes

- Users can restore an unchanged eligible `uhm`-managed file output to its captured preimage and verify the bytes observed immediately afterward while the snapshot remains available.
- Created managed outputs can be quarantined and removed only after `uhm` observes bytes matching what it committed; the concurrent-writer limitation remains explicit.
- Later edits produce a clear conflict instead of being silently overwritten or mislabeled as an undo.
- Arbitrary actions may receive a clearly labeled, reviewed best-effort recovery proposal without creating a false rollback guarantee.
- Recovery state, storage use, retention, expiry, corruption, and partial operations are inspectable from local receipts.
- Users always receive an honest unavailable result when `uhm` lacks the evidence or control required to reverse an effect.
- Snapshot bytes are never captured merely because the user enabled metadata history; recovery requires a separate disclosed global or per-job choice.

## Definition of done

- Eligibility tests classify supported regular-file replacement/creation correctly and reject symlinks, directories, special files, unsupported metadata, undeclared outputs, and paths outside the managed commit contract.
- Recovery is off by default. Enablement tests prove the disclosure precedes the first snapshot; one-job `--recoverable` does not persist; `recovery off` and disabled history create no new recovery artifact/event while ordinary execution remains available with an explicit unavailable label.
- Snapshot fixtures prove preimages/nonexistence markers are durable before the Python child starts, then verify private permissions, staged/postimage hashes, immediate pre-commit revalidation, atomic no-replace creation, fsync/rename ordering, manifest linkage, and zero telemetry/model serialization paths.
- Commit fault-injection tests cover snapshot failure, disk full, rename failure, process death before and after each journal transition, postimage mismatch, and partial multi-output commit with explicit status/resume behavior.
- `undo` succeeds for unchanged replacement and creation fixtures, verifies the final state, and records a linked completed restore job.
- Conflict tests cover a program that ignores staging, user edits, concurrent creation, missing or replaced destinations, symlink swaps, corrupt/expired snapshots, permission changes, quarantine mismatch/restore, and observable concurrent-writer races; none is reported as a clean managed commit or verified restore. Documentation states the residual compare/rename and hash/quarantine races that cannot be eliminated portably.
- Multi-output undo preflights all destinations, journals per-item progress, resumes safely after interruption, and reports partial state without claiming collection-wide atomicity.
- `undo` never falls through to model generation, and `--force` cannot relabel a hash-conflicted operation as verified.
- `restore <id> --force` requires the literal force flag, previews every conflict/destination, uses retained snapshots only, and records `forced_restore` rather than `undo`; unsupported path types remain unavailable with the exact manual snapshot location when disclosure is safe.
- `recover` previews the exact bounded receipt subset sent to OpenAI, requires review of the resulting strict proposal, and follows current warnings and execution policy as a linked new job.
- A `recover` job cannot exceed two total model calls or executions; clarification, model revision/repair, and post-failure local replacement share its one second-turn slot, and no inverse is automatically executed or chained.
- Representative unsnapshotted deletion, remote API, package, database, privilege, message-send, and secret-exposure receipts yield best-effort or unavailable classifications without invented certainty.
- Retention tests enforce configured age, total-byte, and per-file limits; pruning coordinates with history tombstones, locks out active operations, and never deletes outside validated snapshot descendants.
- Linux and macOS integration tests cover supported atomic file behavior, cancellation, signals, SSH/tmux invocation, and non-TTY structured output.
- Documentation and CLI copy use `undo` only for verified restore, `recover` only for best effort, and explicitly state the unsupported file/effect cases.

## Anti-goals

- Do not promise universal undo, transactional shell execution, filesystem-wide snapshots, remote rollback, or recovery after unsnapshotted deletion.
- Do not use `undo` for model-proposed inverses, hash conflicts, expired/corrupt snapshots, or unsupported effects.
- Do not automatically execute, retry, or chain recovery proposals, and do not create an autonomous repair loop.
- Do not snapshot arbitrary shell writes, entire directories, repositories, home directories, databases, remote systems, or files outside Plan 4's managed output contract.
- Do not let a receipt, zero exit status, or plausible inverse command stand in for verified restoration.
- Do not upload snapshots, file bytes, manifests, paths, commands, errors, hashes, or any receipt content to telemetry. `recover` may send only its explicitly previewed bounded receipt subset to OpenAI; it must never send snapshots, unrelated runs, unpreviewed fields, or full history.
- Do not make shell integration, background monitoring, a daemon, cloud storage, accounts, or provider-side memory part of recovery.
- Do not retain snapshots without visible, configurable limits or silently discard them while claiming recovery remains available.
- Do not equate metadata-history consent with consent to duplicate file contents for recovery.
