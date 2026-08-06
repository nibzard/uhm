<!-- diataxis: explanation -->

# ADR 0004: Bounded evidence-based recovery

## Decision

Reserve `undo` for retained, hash-verified preimages of regular-file outputs committed through the managed sibling-staging coordinator. Treat model-proposed inverses as a separate `recover` job that is always labeled best effort and reviewed. Report recovery as unavailable when neither evidence path applies.

Snapshot capture is separately disclosed and off by default. Manifests use an explicit crash-aware state machine and per-item states under the existing private run directory. Capture opens and validates destination parents and files descriptor-relatively, rejects unsupported metadata and filesystems, writes a durable `preparing` manifest, stores private bounded snapshots, revalidates every destination before the first commit, and journals each rename. Creation uses the platform atomic no-replace primitive.

Undo preflights the full output set, checks current postimage hashes, restores through fsynced sibling files or verified quarantine removal, and checks the final preimage hash or absence. A hash conflict requires the explicitly distinct `restore --force` command. Multi-output work is resumable but never described as one filesystem transaction.

Retention is bounded by age, total bytes, per-file bytes, recovery-manifest count, and prune batch. Capture persists an age deadline and immutable selection sequence under the recovery lock. A later configuration change may shorten that deadline but cannot extend it. Unpinned evidence becomes logically unavailable at the deadline, independently of physical garbage collection. Pinning must precede expiry, suspends enforcement without moving the deadline, and active partial operations remain protected.

Every selection considers the complete bounded recovery inventory and fails closed on active-count overflow, corruption, or ambiguous legacy ordering. Non-recovery artifacts and pending terminal transitions do not consume active capacity, but terminal manifests remain in the sequence high-water mark. Capture refuses admission at the manifest limit; prune scans the complete inventory as the recovery escape hatch. A live coordinator owns the recovery lock through commit and starts its stale-process lease only after every snapshot is ready. Pruning validates owned descendant paths, skips active/pinned operations, and persists non-restorable item-level intent plus event provenance before its first unlink. Partial batches are reported as `expiring`, are excluded from restore selection and active capacity, and resume within the batch bound. The `expired` state is a crash-safe transition. Management cleanup records one idempotent history event, then durably acknowledges and finalizes; a crash-left pending transition remains reportable. Automatic capture-time retention needs no event but cannot downgrade or silently finalize a management-started transition or an unacknowledged `restored` run. Finalization durably removes the snapshots directory before unlinking the manifest. Completed restore similarly persists acknowledged expiry before unlinking snapshots. Unrelated history artifacts remain. Snapshot bytes and paths are absent from telemetry and model request types.

## Consequences

The word undo has a narrow, testable meaning. Recovery remains useful for arbitrary receipted actions without implying semantic rollback. The implementation is deliberately limited to Linux and macOS regular files and cannot eliminate the portable race between a final hash observation and rename. Backups remain necessary for broader recovery.

## Rejected alternatives

- Inferring reversibility from command text, because a plausible inverse is not evidence.
- Snapshotting arbitrary writes or directories, because the utility does not mediate those effects.
- Treating successful inverse execution as recovered state, because remote and semantic state cannot be verified locally.
- Silently coupling snapshots to metadata history, because duplicated file content needs separate consent.
