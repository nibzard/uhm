# ADR 0001: Core dependencies and MSRV

Status: accepted for Plan 1

Plan 2 transport and process-control decisions are superseded by [ADR 0002](0002-responses-result-loop.md); this record preserves the Plan 1 rationale.

## Decision

The minimum supported Rust version is 1.82.0 and is declared in `Cargo.toml`. Stable Rust is the normal build channel. Linux and macOS are release platforms.

Correctness boundaries use focused maintained crates:

| Boundary | Decision |
|---|---|
| CLI grammar | Keep the small product-specific parser, with an explicit opaque-intent boundary and contract tests. A conventional subcommand parser cannot express this boundary more clearly. |
| JSON | `serde` and `serde_json`; delete the local JSON parser. |
| YAML | `serde_yaml_ng`; it is a maintained Serde parser with a declared Rust 1.64 MSRV. Delete the local YAML parser. |
| Temporary files | `tempfile`; cache writes use private same-directory temporary files and atomic persistence. Plan 1 also removes the raw editor and its hand-rolled temporary file. |
| File locking | `fs2`; JSONL receipt appends take an exclusive cross-process lock. Revisit this if Plan 2 selects SQLite. |
| Unix process/signal behavior | Standard `ExitStatusExt` is sufficient for the Plan 1 child-status contract. Consider `rustix` only when process-group control is implemented. |
| Artifact hashing | `blake3` for cache/provenance identifiers. |
| Display width | `unicode-width` is reserved for any future terminal geometry. Plan 1 removes cursor-positioning UI, so correctness does not currently depend on width calculations. |
| PTY tests | Keep the Plan 1 interaction surface cooked and `/dev/tty` based. Add `portable-pty` only when a raw interactive surface returns. |
| HTTP/TLS | Retain `ureq` 2 with rustls until Plan 2 replaces the Chat Completions transport. |

Direct and transitive dependencies use permissive MIT, Apache-2.0, ISC, BSD, Unicode-3.0, CC0, CDLA-Permissive, or compatible combinations. The lockfile is checked in. BLAKE3 and tempfile are pinned to releases compatible with Cargo 1.82; compatible transitive URL/IDNA, indexmap, and zeroize releases are locked for the same reason. CI builds the MSRV and runs the full stable suite on Linux and macOS, plus a musl release build.

Verification on 2026-08-01:

- `cargo +1.82.0 check --all-targets --locked` passed.
- RustSec `cargo-audit` scanned 95 locked crates with no known vulnerability.
- The optimized x86_64 GNU/Linux binary is 3.7 MiB in this build environment.
- The local musl build reached the native TLS build and then stopped because this environment lacks `x86_64-linux-musl-gcc`; the checked-in CI job installs `musl-tools` before building.

## Consequences

The old “one direct dependency” goal is gone. Parser and serializer maintenance moves to their upstream projects, the binary grows modestly, and configuration failures become actionable. Dependencies that do not yet protect an active boundary are documented but not added merely to satisfy a checklist.
