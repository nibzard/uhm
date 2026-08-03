<!-- diataxis: navigation -->

# Local history

UHM keeps a private append-only decision journal that is separate from telemetry. Metadata history is enabled by default; richer diagnostic and full retention are explicit local choices.

Choose the page that matches what you need:

- [Inspect, replay, and maintain history](how-to/use-history.md) — perform common history operations.
- [History reference](reference/history.md) — look up detail levels, commands, defaults, integrity, and export behavior.
- [Recovery](recovery.md) — understand the separately consented snapshot lifecycle.
- [Trust boundaries](explanation/trust-boundaries.md) — understand why history, recovery, cache, and telemetry remain separate.

Use `uhm history status` to inspect the current location, detail level, and limits without making a provider request.
