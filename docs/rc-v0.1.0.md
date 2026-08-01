# v0.1.0 release-candidate gate

Feature work is frozen for this candidate. P0 means data exposure, unexpected execution, or corrupt result bytes. P1 means a supported install or core result path does not work.

## Automated evidence

| Area | Gate | Result |
|---|---|---|
| Rust | format, strict Clippy, 62 unit tests, 13 CLI contract tests | pass |
| Release build | locked optimized build and installed-binary smoke test | pass |
| MSRV | Rust 1.82 `cargo check --all-targets --locked` | pass |
| Crate | package, package build, install, publish dry run | pass |
| Linux archive | static PIE verification, archive extraction, archived-binary smoke test | pass |
| Worker | exact schema, size, version, enum, rate limit, kill switch, 202, field order | pass |
| Terminal | plain control-byte test; display widths 40/80/160; CJK, emoji, combining text | pass |
| Telemetry | opt-outs before queue access; private bounded queue; crash recovery; send deadline | pass |
| Release | four native archive jobs, archive smoke tests, checksums, attestations | pending tag workflow |

## Dogfood corpus

The release candidate must exercise each row on both a normal TTY and `--plain` where applicable. Record only pass/fail and defect links. Never paste private prompts, commands, or output into this file.

| Scenario | Expected evidence | Linux | macOS |
|---|---|---:|---:|
| Ordinary local read | result on stdout, no review | pass | CI |
| Local file write | result first, receipt is metadata only | pass | CI |
| Package or network read | declared effect is visible | pass | CI |
| Detected deletion | literal warning and review pause | pass | CI |
| Detected `sudo` | privilege warning and review pause | pass | CI |
| Failed command repair | original failure preserved; at most one repair | pass | CI |
| Answer route | prose result, no shell execution | pass | CI |
| Exact piped input | data reaches requested action unchanged | pass | CI |
| Parent-shell action | exact action returned, status 11, not falsely applied | pass | CI |
| SSH PTY | one-shot lifecycle; no persistent session | pass | manual release host |
| tmux | stable layout and predictable Ctrl-C | pass | manual release host |
| Plain/cooked mode | no ESC, OSC, DECSET, spinner, or cursor control | pass | CI |

## Ship decision

Ship only when the default branch and tag workflow are green, all four declared archives are attached, their checksums verify, and no P0/P1 defect is open. Crates.io publication may be deferred if ownership is unavailable; it does not block the GitHub release.
