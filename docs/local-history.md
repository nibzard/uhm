# Local history

`uhm` records an inspectable, append-only decision timeline in the platform data directory. `uhm history status` prints the exact location. The authoritative store is `history.v1.jsonl`; optional content lives under `runs/<run-id>/` with a versioned manifest. Directories are owner-only and files are mode `0600` on Unix.

History and telemetry are separate. Telemetry's serializer accepts only a content-free coarse projection. The local journal is never attached to an OpenAI request.

## Detail levels

- `metadata` (default) retains transitions, versions, routes, effect/outcome categories, sizes, hashes, and truncation flags. It does not retain intent, proposals, source, paths, output, or diagnostics.
- `diagnostic` additionally retains the exact typed proposal and bounded failure diagnostics. Set `capture_output: true` to retain available result tails too.
- `full` additionally retains the original intent. Path-looking whitespace-delimited values are redacted by default. This level is required for explicit repair.

Changing levels affects future records only. Exact replay requires `diagnostic` or `full`; repair requires `full`. Replay creates a linked run, gathers current context, validates the stored typed proposal again, and always enters review. It makes no model call before review and never auto-executes. Repair is a new bounded job and discloses the small retained subset it sends.

## Inspection and lifecycle

```sh
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

Exports are redacted by default: run identifiers, relationships, content, paths, proposal references, output, and diagnostics are removed. `--include-content` is an explicit local disclosure. Export writes atomically. Back up the journal and `runs` directory together if preserving replay evidence matters.

Pruning applies record-count, age, journal-byte, and owned-run rules under the validated history root. Symlinks below the run root are rejected. Separately consented recovery snapshots have their own lifecycle and are excluded from normal export and generic retention deletion; see [bounded recovery](recovery.md). History alone offers no undo, rollback, shell surveillance, cloud sync, or proof that an exit-zero process achieved the user's intent.

## Integrity and recovery

Every event has a BLAKE3 checksum for accidental-corruption detection, not tamper evidence. A truncated final JSONL line is reported and ignored; corruption in an earlier line stops writes. Before manual repair, copy the data directory. Export intact lines, restore from backup, or remove only the corrupt journal after inspecting it. The old Plan 2 `history.jsonl` format migrates atomically and is retained as a timestamped `.bak` file.

The initial 500-event, 30-day, 256 MiB defaults preserve the earlier metadata envelope; they are conservative dogfood hypotheses rather than measured universal values. JSONL remains appropriate until real measurements show material query latency, writer contention, or relational requirements.
