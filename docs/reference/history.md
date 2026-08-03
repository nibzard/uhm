<!-- diataxis: reference -->

# History reference

The authoritative journal is private `history.v1.jsonl` in the platform data directory. Optional per-run content is stored beneath `runs/<run-id>/`. `uhm history status` prints the resolved paths and limits.

Directories created by UHM are owner-only and files are mode `0600` on Unix. Retained proposals use versioned envelopes and append-only `proposal-1.json`, `proposal-2.json` names; a replacement never overwrites its predecessor.

## Detail levels

| Level | Retained data | Supports |
|---|---|---|
| `metadata` | transitions, versions, route, effects/outcomes, sizes, hashes, truncation flags | inspection and aggregate local history |
| `diagnostic` | metadata plus exact typed proposals and bounded failure diagnostics; optional result tails | exact replay |
| `full` | diagnostic plus bounded original intent | replay and explicit repair |

Changing level affects future records only. Path-looking values in full intent are redacted by default.

## Commands

```text
uhm history list [--limit N] [--failed] [--route ROUTE]
uhm history show <run-id|last>
uhm history search -- <substring>
uhm history replay <run-id> --review
uhm history export [--output /absolute/path] [--include-content]
uhm history prune [--dry-run]
uhm history clear --before YYYY-MM-DD
uhm history clear --all
uhm history status
uhm feedback good|bad [run-id]
```

Replay recognizes retained versioned proposals, validates them against the current action contract, creates a linked run, and always enters review. Bare schema-v3 proposal history is read only through its dedicated legacy normalizer.

## Defaults

| Limit | Default |
|---|---:|
| Records | 500 |
| Age | 30 days |
| Total bytes | 268,435,456 |
| Per-run artifact bytes | 1,048,576 |

## Integrity and export

Events use BLAKE3 checksums for accidental-corruption detection, not tamper evidence. An interrupted final line is reported and ignored; earlier corruption stops writes.

Default export removes run IDs, relationships, content, paths, proposal references, output, and diagnostics. `--include-content` is an explicit local operation. Export writes atomically.

History pruning does not delete separately consented recovery snapshots. `history clear --all` reports and preserves recovery-owned run directories.
