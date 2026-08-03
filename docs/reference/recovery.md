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
uhm recovery prune [--dry-run]
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

Pruning is explicit, oldest first, skips active and pinned manifests, deletes only validated owned snapshots, and leaves an `expired` tombstone.

Pinning protects an otherwise valid manifest from ordinary expiry; it cannot legitimize storage that already exceeds the total cap.

## Atomicity

Each supported rename is atomic. A multi-output set is not a filesystem transaction, and another writer can race between the final hash check and rename. Partial commits and interrupted undo are represented explicitly for bounded resumption.
