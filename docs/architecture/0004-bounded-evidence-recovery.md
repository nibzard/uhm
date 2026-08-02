# ADR 0004: Bounded evidence-based recovery

## Decision

Reserve `undo` for retained, hash-verified preimages of regular-file outputs committed through the managed sibling-staging coordinator. Treat model-proposed inverses as a separate `recover` job that is always labeled best effort and reviewed. Report recovery as unavailable when neither evidence path applies.

Snapshot capture is separately disclosed and off by default. Manifests use an explicit crash-aware state machine and per-item states under the existing private run directory. Capture opens and validates destination parents and files descriptor-relatively, rejects unsupported metadata and filesystems, writes a durable `preparing` manifest, stores private bounded snapshots, revalidates every destination before the first commit, and journals each rename. Creation uses the platform atomic no-replace primitive.

Undo preflights the full output set, checks current postimage hashes, restores through fsynced sibling files or verified quarantine removal, and checks the final preimage hash or absence. A hash conflict requires the explicitly distinct `restore --force` command. Multi-output work is resumable but never described as one filesystem transaction.

Retention is bounded by age, total bytes, per-file bytes, scan count, and prune batch. Pruning validates owned descendant paths, skips active/pinned operations, deletes snapshots rather than arbitrary run content, and leaves an expired tombstone. Snapshot bytes and paths are absent from telemetry and model request types.

## Consequences

The word undo has a narrow, testable meaning. Recovery remains useful for arbitrary receipted actions without implying semantic rollback. The implementation is deliberately limited to Linux and macOS regular files and cannot eliminate the portable race between a final hash observation and rename. Backups remain necessary for broader recovery.

## Rejected alternatives

- Inferring reversibility from command text, because a plausible inverse is not evidence.
- Snapshotting arbitrary writes or directories, because the utility does not mediate those effects.
- Treating successful inverse execution as recovered state, because remote and semantic state cannot be verified locally.
- Silently coupling snapshots to metadata history, because duplicated file content needs separate consent.
