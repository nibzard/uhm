<!-- diataxis: reference -->

# Recovery reference

Recovery schema version 1 covers eligible regular-file outputs committed through UHM's managed sibling-staging coordinator.

## Operations

| Operation | Provider call | Evidence | Review |
|---|---:|---|---:|
| `undo` | no | retained preimage plus matching current postimage | local preview |
| `restore --force` | no | retained preimage; explicit conflict override | local preview |
| `recover` | yes | bounded retained intent/proposal/outcome | always |
| `recovery resume` | no | matching partial-commit manifest and files | always |

## Eligibility

Verified capture accepts current-user-owned, single-link regular files on the supported local-filesystem allowlist and within the configured per-file limit. It rejects symlinks, hard links, unsupported types or mounts, ACLs, extended attributes/resource forks, and other metadata it cannot restore faithfully.

Version 1 restores bytes and Unix permission bits. It does not restore ownership, timestamps, ACLs, extended attributes, sparse layout, or link identity.

If capture is requested but an original output is ineligible, `--force` on that original job may continue without a snapshot; its preview and receipt report that verified restore is unavailable.

## Manifest states

`none`, `preparing`, `available`, `commit_partial`, `undo_preflight`, `undo_in_progress`, `restored`, `conflicted`, `expired`, and `corrupt`.

Illegal state transitions fail closed. Missing or corrupt snapshots block both verified undo and forced restore.

Snapshots and `recovery.json` live under the private `runs/<run-id>/` directory reported by `uhm recovery status`. Snapshot files are mode `0600`, are excluded from telemetry and normal history export, and are never serialized into provider requests.

## Commands

```text
uhm recovery on
uhm recovery off [--prune]
uhm recovery status [<run-id|last>]
uhm recovery prune [--dry-run] [--all]
uhm recovery pin <run-id|last>
uhm recovery unpin <run-id|last>
uhm recovery resume <run-id>
uhm undo <run-id|last> [--review]
uhm restore <run-id|last> --force
uhm recover <run-id|last> [guidance]
```

## Default retention limits

| Limit | Default |
|---|---:|
| Age | 14 days |
| Total snapshot bytes | 134,217,728 |
| Per-file bytes | 8,388,608 |
| Scan count | 1,000 |
| Prune batch | 100 |

Each capture records an expiry deadline from the age limit in effect at capture time. Reducing `max_age_days` may shorten that deadline; increasing it never extends already-captured evidence. At the deadline, an unpinned `available` or `conflicted` run becomes logically expired immediately: `undo`, forced restore, and `last` selection cannot use it even if physical cleanup has not run yet.

Pinning must happen before the deadline. It suspends ordinary age expiry without moving the deadline, so unpinning after that time makes the run immediately expired. Pinning cannot legitimize storage that already exceeds the total-byte cap. Active partial operations remain protected until they are completed or their preparation lease expires.

Pruning is oldest first, skips active and pinned manifests, and deletes only validated recovery-owned snapshots. Plain prune enforces age and byte limits; `--all` also retires otherwise-current unpinned evidence. Before the first snapshot unlink, prune durably marks the selected items as retirement intent and preserves whether a local expiry event is required. A partly pruned manifest is reported as `expiring`, is never eligible for explicit restore or `last`, does not consume active scan capacity, and resumes within the configured batch bound after a crash. An `expired` manifest is a crash-recovery transition, not permanent history: recovery-owned snapshots and the manifest are finalized on the current or next cleanup pass while other run artifacts remain. Management prune records one idempotent local expiry event when history is available, acknowledges it in the manifest, and only then finalizes. A crash between those steps leaves a pending manifest that the next management prune reports and retries. Capture-time automatic retention needs no separate expiry event and can finalize its own transition, but it never downgrades or silently finalizes a management-started transition or a crash-left `restored` run whose completion event is uncertain. A successfully restored manifest is acknowledged by its already-durable completion event and likewise persists `expired` before deleting snapshots.

`last` is ordered by an immutable sequence allocated when capture starts; later status changes, pinning, or restore attempts do not reorder runs. Selection scans the complete recovery-manifest inventory and fails closed if its active count exceeds `scan_limit`, if a manifest is corrupt, or if legacy ordering is tied. Non-recovery history directories and pending terminal transitions do not consume the active limit; pending transitions still participate in the sequence high-water mark and remain visible to management prune. Capture refuses to create another active manifest once the limit is full, while `recovery prune` deliberately scans the complete inventory so it remains the escape hatch. A live capture owns the recovery lock through commit and renews its stale-process lease only after every preimage is durable, preventing retention from expiring evidence still owned by a running coordinator.

## Atomicity

Each supported rename is atomic. A multi-output set is not a filesystem transaction, and another writer can race between the final hash check and rename. Partial commits and interrupted undo are represented explicitly for bounded resumption.
