# Bounded recovery

`uhm` has two deliberately different recovery paths.

- `undo` is a hash-verified restore of file outputs that `uhm` committed through its managed sibling-staging path while recovery capture was enabled.
- `recover` asks OpenAI for one reviewed, best-effort inverse based on a bounded retained receipt. Running that proposal does not prove that the original state was recovered.
- Everything else is reported as unavailable. History is evidence, not a universal rollback mechanism.

For schema-v4 generated programs, any `write_only` or `read_write` helper resource enters this same managed staging and recovery path. The model sees only the helper's private staging path, never the logical destination through its writable capability; `read_write` receives a separate validated current-read path. The one-use resolved launcher contract is deleted before model source runs and is never recovery evidence.

## Enable snapshot capture

Recovery is off by default and has consent separate from metadata history because a preimage duplicates file content:

```sh
uhm recovery on
uhm recovery status
uhm run --recoverable rewrite report.txt as compact JSON  # one job only
uhm recovery off
uhm recovery off --prune
```

The enable command discloses the storage path and limits before writing its private consent marker. `--recoverable` does not persist. Recovery also requires metadata history for durable run linkage; it never silently changes history detail or output capture.

Initial verified recovery covers only current-user-owned, single-link regular files on the tested local filesystem allowlist. Files with symlinks, hard links, ACLs, extended attributes/resource forks, unsupported types, unsupported mounts, or preimages above the per-file limit are ineligible. V1 restores bytes and Unix permission bits, not ownership, timestamps, ACLs, xattrs, sparse layout, or link identity. `--force` on the original job may continue without a snapshot, but the preview and receipt say verified restore is unavailable.

Snapshots and `recovery.json` live below the private `runs/<run-id>/` directory shown by `uhm recovery status`. Snapshot files are mode `0600`. They are never referenced by telemetry or serialized into an OpenAI request. Normal history export excludes them.

## Verified undo and forced restore

```sh
uhm undo <run-id|last>
uhm restore <run-id|last> --force
```

Undo previews every destination and preflights the entire set before changing the first item. A replacement must still have the recorded committed hash. A created output must still be an owned regular file with that hash. Every retained snapshot is rehashed. Replacements use a fsynced sibling file and rename; created outputs are moved to a sibling quarantine, rehashed, and deleted only on a match. The final preimage hash or absence is checked and each item transition is journaled.

A later edit is a conflict, not an undo. `restore --force` is the explicit escape hatch: it uses retained evidence, still rejects unsupported destination types, previews conflicts, and records `forced_restore`. It never reports verified or semantic recovery.

Each individual supported rename is atomic. A multi-output set is not a filesystem transaction, and there is no portable compare-and-swap joining the last hash observation to rename. Another writer can race that window. An interruption may leave `commit_partial` or `undo_in_progress`; `uhm recovery status <id>` reports it. Undo resumes completed item states safely. A reviewed `uhm recovery resume <id>` can resume a partial managed commit only when the retained preimage, stage, and already-committed hashes still match.

## Best-effort inverse

`uhm recover <run-id|last> [guidance]` requires full history detail for the original intent and diagnostic/full detail for the typed proposal. It prints the exact bounded subset and instruction before sending them, then asks for confirmation. The normal selected current context is also sent and disclosed. The full journal, unrelated runs, snapshots, and snapshot bytes are excluded.

The resulting action is always reviewed and linked to the original run. The existing global limit of two model calls and two executions applies; clarification, revision, edit, or failure repair share the single replacement slot. A recovery job cannot recover another linked recovery job, and no inverse is automatically executed, retried, or chained.

## Retention and failure states

```sh
uhm recovery status [<run-id|last>]
uhm recovery pin <run-id|last>
uhm recovery unpin <run-id|last>
uhm recovery prune --dry-run
uhm recovery prune
```

Age, total-byte, per-file, scan, and prune-batch limits are configurable. Pruning is explicit and bounded, oldest first, skips active operations and pinned manifests, validates every owned snapshot path, removes only expected private files, and leaves an `expired` manifest tombstone. There is no daemon or cloud backup. Pinning cannot be used to legitimize storage already above the configured total cap.

States are `preparing`, `commit_partial`, `available`, `undo_preflight`, `undo_in_progress`, `restored`, `conflicted`, `expired`, and `corrupt`. Illegal transitions fail closed. A crash during capture leaves `preparing`; a corrupt or missing snapshot blocks both verified undo and forced restore. Backups remain the right tool for directories, repositories, databases, remote systems, unsnapshotted deletions, and recovery beyond these narrow managed files.
