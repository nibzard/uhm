<!-- diataxis: how-to -->

# Inspect, replay, and maintain history

Use this guide for common local-history operations. Metadata history is enabled by default and remains separate from telemetry.

## Inspect recent work

```sh
uhm history status
uhm history list --limit 20
uhm history list --failed
uhm history show last
uhm history search -- failure
```

## Retain enough detail for replay or repair

Set the detail level before the run you may need later:

```yaml
history:
  enabled: true
  detail: diagnostic   # replay
  capture_output: false
```

Use `full` when you also need the original intent for explicit repair. Changing the level affects only future records.

## Replay a retained proposal

```sh
uhm history replay <run-id> --review
```

Replay validates the retained proposal against the current contract and always enters review. It does not call a provider before review and never auto-executes.

## Repair a prior run

```sh
uhm repair <run-id|last> explain the desired correction
```

Repair requires full history. It previews the bounded retained subset before sending it to the selected provider.

## Export safely

```sh
uhm history export --output /absolute/path/history.jsonl
```

The default export removes identifiers, relationships, content, paths, proposal references, output, and diagnostics. `--include-content` is an explicit local disclosure.

## Prune or clear records

```sh
uhm history prune --dry-run
uhm history prune
uhm history clear --before 2026-08-01
uhm history clear --all
```

History clearing reports and preserves separately consented recovery-owned run directories. See the [history reference](../reference/history.md) for storage and integrity details.
