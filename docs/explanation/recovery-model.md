<!-- diataxis: explanation -->

# Why recovery is evidence-based

“Undo” is easy to promise and difficult to define. A command may touch several files, a remote service, a database, a running process, or state UHM never observed. Generating a plausible inverse does not prove that the original state was restored.

## Verified undo has a narrow meaning

UHM reserves `undo` for managed file outputs with a retained preimage and recorded postimage. Before restoration it rechecks file type, ownership assumptions, the current output hash, and the snapshot hash. A later edit is a conflict because overwriting it would destroy evidence rather than restore a known state.

## Forced restore is explicit

`restore --force` exists for cases where the user intentionally wants retained bytes despite a current-state conflict. It preserves type and evidence checks but records a different outcome. Calling it “forced restore” rather than “undo” keeps the receipt honest.

## Generated inverses are best effort

`recover` can still be useful when verified evidence is unavailable. It sends a previewed bounded receipt subset to the selected provider and always reviews the returned action. Successful execution proves only that the inverse action ran successfully, not that semantic or remote state returned to its original value.

## Recovery remains separate from history

Metadata history records decisions without retaining file contents. Snapshot capture duplicates bytes, so it has separate consent, limits, retention, and lifecycle controls. Clearing history does not silently delete recovery-owned evidence.

This design favors narrow truthful guarantees over broad rollback language. Backups and version control remain the correct tools for general recovery.

See the [recovery tutorial](../tutorials/recover-file.md), [recovery how-to](../how-to/recover-work.md), and [recovery reference](../reference/recovery.md).
