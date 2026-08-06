<!-- diataxis: how-to -->

# Recover prior work

Choose the recovery operation that matches the evidence you have.

## Capture evidence for a managed file change

```sh
uhm recovery on
uhm run --recoverable rewrite report.txt as compact JSON
uhm recovery status last
```

Persistent capture and the one-job `--recoverable` flag are separate. Both require eligible managed writable resources.

## Perform a verified undo

```sh
uhm undo <run-id|last>
```

Undo succeeds only when the retained snapshot and current output hashes match the recorded state. It preflights the entire output set before changing the first item.

## Handle a later-edit conflict

Inspect the current file and retained status first:

```sh
uhm recovery status <run-id|last>
```

If replacing the newer state is intentional, use the explicit escape hatch:

```sh
uhm restore <run-id|last> --force
```

Forced restore remains hash- and type-aware but records `forced_restore`; it never claims verified or semantic recovery.

## Resume an interrupted managed commit

```sh
uhm recovery status <run-id>
uhm recovery resume <run-id>
```

Resume requires review and proceeds only when retained preimage, staged output, and already-committed hashes still match.

## Request a best-effort inverse

For work without verified snapshot evidence:

```sh
uhm recover <run-id|last> prefer a local-only inverse
```

This requires full history for the intent and diagnostic/full history for the proposal. `uhm` previews the bounded subset before sending it, and the returned inverse always requires review.

The selected current context is also sent. The full journal, unrelated runs, recovery manifests, snapshot paths, and snapshot bytes are excluded.

## Manage retention

```sh
uhm recovery pin <run-id|last>
uhm recovery unpin <run-id|last>
uhm recovery prune --dry-run
uhm recovery prune
uhm recovery prune --all
uhm recovery off --prune
```

Pin evidence before its age deadline if you need to retain it. The deadline does not move; once an unpinned run reaches it, restore is refused even before the next physical prune. Plain prune enforces age and byte limits. Use `--all` to retire every inactive, unpinned recovery run, including runs still within those limits.

Use backups or version control for directories, repositories, databases, remote systems, unsnapshotted deletions, and anything outside managed-file recovery. See the [recovery reference](../reference/recovery.md) for eligibility and states.
