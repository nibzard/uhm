# Plan 3 — Prepare and publish the first public release

## Purpose and dependency

This plan turns the feature-complete core from Plans 1–2 into a product someone can discover, install, understand, trust, measure, and use on Linux or macOS without cloning the repository. Completion of this plan is the first public release milestone.

Release v0.1 should make one narrow promise well: natural language goes in, one bounded local job completes, and the useful result comes out. Just-in-time standalone programs, generalized undo, background work, and broader agent behavior do not block this release.

## Full implementation description

### 1. Build a compact first-run and diagnostics experience

On the first invocation, print one concise disclosure to stderr before any outbound model or telemetry request. It should state:

- OpenAI receives the prompt, explicitly piped input, and the selected context mode; `standard` context is the default.
- Bounded metadata-only execution receipts are enabled by default and stored privately on this device; name their limit and the clear command.
- Content-free aggregate telemetry is enabled by default, name the coarse categories and provider-level connection processing, and give the exact opt-out command.
- The user is responsible for actions; warnings are convenience signals, not a safety guarantee.

Persist one versioned disclosure marker only after the notice is rendered. A material change to outbound context or telemetry increments the revision and shows the revised notice once. Do not force the user through a wizard before doing useful work. If `OPENAI_API_KEY` is missing, provide the shortest secure setup path and the exact resolved secrets/config locations. Accept the key through the environment or a private secrets file; never echo it, put it in command history, or forward it to generated child processes.

Add `uhm doctor`, with human and `--json` output, to check:

- Supported host/architecture and terminal capability.
- API key presence without revealing it.
- OpenAI reachability through a cheap explicit check only when requested.
- Config validity and resolved paths/permissions.
- Context and telemetry settings.
- Shell detection and the child-versus-parent-shell limitation.
- Required clipboard mechanisms and, later, optional program interpreters.

Diagnostics must distinguish app configuration, network/TLS, OpenAI authentication/rate limit, structured-response, local process, and terminal failures. Each error says what happened and the next useful action.

### 2. Give `uhm` a memorable terminal-native personality

Use [PRODUCT.md](../PRODUCT.md) as the strategic source. The voice is quick, playful, and capable—closer to the energy and polish of Charm tools than a traditional austere Unix utility.

Personality rules:

- Put delight in short progress verbs, responsive color, crisp spacing, and occasional microcopy; do not add a mascot speech on every run.
- Keep the successful result visually primary and let the interface disappear immediately afterward.
- Be literal around deletion, privilege elevation, remote mutation, failed execution, billing, privacy, and data loss. No jokes or coy wording at consequential moments.
- Avoid anthropomorphic claims such as “I understood” or “I safely did.” Prefer observable state: “running,” “finished,” “needs one detail,” “command exited 2.”
- Remove ornamental confidence percentages and “safe” badges. Show effects, targets, assumptions, and concrete uncertainty.

Define semantic terminal tokens rather than hard-coded colors: primary, muted, success, warning, critical, info, focus. Verify important copy without color or dim text. Use the terminal's existing palette where possible and never make essential content depend on a specific dark/light theme.

### 3. Finish accessibility and terminal compatibility

Make `--plain`/`UHM_PLAIN=1` a first-class product mode and auto-enable it for `TERM=dumb`. Plain mode uses cooked line input, ASCII-safe labels, no spinner or animation, no dim styling, and no cursor/OSC/DECSET sequences.

Independently honor `NO_COLOR` and a no-motion setting so a user can retain Unicode/raw editing without animation. Test control layout at 40, 80, and 160 columns. Use display-cell width for CJK, emoji, and combining input. Ctrl-C cancels the current prompt/action and returns control predictably; Ctrl-D exits an input session.

The no-argument experience should collect one intent, complete it, and exit. Remove or reframe the current indefinite REPL so the product does not imply background presence, a persistent shell, or general conversation.

Test on:

- Bash, Zsh, and Fish on supported hosts.
- macOS arm64 and x86_64.
- Linux glibc and static musl artifacts where practical.
- Direct TTY, redirected stdin/stdout/stderr, tmux, and an SSH PTY.
- `NO_COLOR`, plain mode, screen-reader-oriented cooked input, narrow terminals, and Unicode prompts.

### 4. Implement privacy-preserving default-on telemetry

Use this architecture:

```text
uhm CLI
  → https://telemetry.uhm.dev/v1/events
  → strict Cloudflare Worker validation and rate limiting
  → Workers Analytics Engine dataset uhm_cli_v1
```

Workers Analytics Engine is the best initial fit. As verified on 2026-08-01, its official page says usage is currently unbilled while publishing future pricing; the documented Free allowance is 100,000 data points and 10,000 read queries per day, and raw data is retained for three months. The Worker gateway separately includes 100,000 requests per day on the Free plan. Tinybird remains a credible alternative: its Free query/API allowance is 1,000 per day while its Events API ingestion has a separate 100 requests/second limit. ClickHouse Cloud adds unnecessary operating surface for this scale. Treat every quota and price as time-sensitive and recheck them before deployment. Sources: [Analytics Engine pricing](https://developers.cloudflare.com/analytics/analytics-engine/pricing/), [Analytics Engine limits](https://developers.cloudflare.com/analytics/analytics-engine/limits/), [Workers limits](https://developers.cloudflare.com/workers/platform/limits/), and [Tinybird Free plan](https://www.tinybird.co/docs/forward/pricing/free).

The CLI knows only the first-party HTTPS endpoint and a versioned JSON schema. It never contains a Cloudflare token, dataset layout, or query credential. The Worker:

- Accepts only POST with the correct content type and a body under 2 KiB per event.
- Rejects unknown keys, arbitrary strings, oversized values, and unknown enum variants.
- Does not copy IP, geolocation, User-Agent, headers, or request logs into the dataset.
- Disables payload/body logging and configures Worker observability so request IPs, headers, and event bodies are not retained in application logs.
- Returns `202` after validation and a non-blocking `writeDataPoint()` call.
- Has a server-side kill switch and coarse abuse rate limiting.

Emit at most one `interaction_summary` per completed interaction. Keep proposal transport, local execution, and human outcome evidence distinct. Suggested v1 fields:

```json
{
  "v": 1,
  "event": "interaction_summary",
  "release": "0.1",
  "os": "linux",
  "arch": "x86_64",
  "shell": "bash",
  "mode": "auto",
  "route": "shell",
  "decision": "ran",
  "effects": "network_read",
  "proposal_outcome": "valid",
  "execution_outcome": "exit_zero",
  "user_feedback": "unknown",
  "latency": "2s_5s",
  "cache": "miss",
  "interactive": true,
  "notice_revision": 1
}
```

Every string is a short server-maintained enum. `proposal_outcome` describes whether a usable structured action was returned; `execution_outcome` describes run/not-run, exit, signal, cancellation, or application failure; `user_feedback` is `good`, `bad`, or `unknown`. Exit zero must never be labeled job success. Use only major/minor app version and coarse latency buckets. Do not include a user, installation, device, session, repository, or stable pseudonymous ID. This deliberately supports aggregate usage and outcome analysis but not DAU, retention, or cross-invocation funnels.

Add `uhm feedback good|bad` for explicit content-free feedback on the most recent local metadata receipt. If its interaction summary is still queued, update that local event. If it was already sent, create a separate `feedback_summary` carrying the feedback enum and the receipt's permitted coarse route/effect/proposal/execution enums, with no join key. Dashboards must not count that separate event as another interaction. The server never receives a receipt, run, or installation identifier, and telemetry never accepts feedback text.

Never collect:

- Prompts, questions, clarifications, feedback text, generated/edited/executed commands, or hashes of them.
- Answers, explanations, stdin, stdout, stderr, clipboard data, panic text, stack traces, or raw error messages.
- cwd, paths, filenames, directory entries, repository/remotes/branches/diffs, aliases, history, or cached responses.
- Username, hostname, locale, timezone, exact client/local timestamp, User-Agent, environment/config values, API keys, tokens, or model endpoint.

The privacy notice and policy must be precise about infrastructure: Cloudflare necessarily processes the connection IP to serve the request, and Analytics Engine records its own server ingestion timestamp. `uhm` does not place the IP or an exact client timestamp in its payload, and the Worker must not persist the IP in application telemetry or logs. Do not claim that a network provider never processes connection metadata.

Because telemetry is enabled by default per the product decision, show the notice before the first event in both interactive and noninteractive use. Provide `uhm telemetry status|preview|on|off`, `--no-telemetry`, `UHM_TELEMETRY=off`, and honor `DO_NOT_TRACK=1`. Evaluate every opt-out before creating or claiming an event. `preview` renders the current invocation's candidate event, never an ambiguous queued batch.

Telemetry is deliberately lossy and must never materially slow or alter the job:

- After the useful result has been written, synchronously attempt the current summary once with a cancellable hard 100 ms end-to-end deadline. This may delay return to the shell by at most that documented budget; it must never delay result bytes or run on local alias/cache-result paths. Enqueue only when no request was attempted or failure is definitely pre-send; an ambiguous post-send outcome follows the loss rule below.
- Store each queued event as its own atomically created private file. Under one dedicated exclusive queue lock, claim events by atomic rename and prune. A `202` deletes a claim; a definite pre-send failure restores it if still within bounds. If the request may have reached the server but the response is lost, restoring the claim can produce a rare duplicate; deleting it can lose an event. Prefer loss in that ambiguous case, document best-effort/lossy semantics, and make aggregate queries tolerant of residual duplicates rather than adding a stable identifier.
- At the next network-bound invocation, flush up to ten older summaries within a separate hard 200 ms batch deadline. There is no daemon and no long-running background task.
- Use a separate cross-process send lock from the final opt-out check through request completion/timeout. Normal invocations use a nonblocking acquisition and enqueue instead of waiting behind another sender. `telemetry off` writes the disabled configuration, then acquires the send and queue locks, rechecks state, and deletes all local events before returning. It cannot retract an event already accepted by the server, but after the command returns no local send remains in flight.
- Cap the queue at 20 summaries and seven days; drop oldest events beyond either limit.
- Do not retry a send within an invocation. DNS, TLS, timeout, validation, quota, crash, or ambiguous acknowledgement may lose an event; rare duplicates remain possible only where transport outcome cannot be known. This is aggregate best-effort telemetry, not an exactly-once channel.
- Telemetry failure never changes stdout, child execution, or exit status.

The CLI release gate ends at validated ingestion plus a few aggregate queries. Optional D1 cohorts, dual-write migration, and long-term analytics belong in an operational runbook and must not expand v0.1. If added later, only thresholded aggregates may outlive Analytics Engine's raw retention; raw events must never be copied into a durable identity store.

### 5. Build release automation and artifacts

Create two GitHub Actions layers.

Pull-request CI:

- Stable Rust formatting, Clippy with warnings denied, all unit/integration/PTY tests, and release build.
- Linux and macOS runners.
- Mock OpenAI Responses tests; no production API key in CI.
- Plain/non-TTY control-byte checks and packaged-binary smoke tests.

Tag release workflow:

- Trigger only from an intentional semantic-version tag matching `Cargo.toml`.
- Build and smoke-test at least `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `x86_64-apple-darwin`, and `aarch64-apple-darwin`. If a target cannot be built reproducibly, omit it explicitly rather than relabeling a dynamic binary as static.
- Produce consistently named `.tar.gz` archives containing the binary, license, and readme.
- Generate SHA-256 checksums and build-provenance attestations.
- Create a draft GitHub Release, attach every asset, and publish only after validation. Enable immutable releases when the repository setting is available.
- Smoke-test the actual archived artifact, not only `cargo run` or the build-tree binary.

GitHub documents draft/immutable releases and artifact attestations here: [release management](https://docs.github.com/en/repositories/releasing-projects-on-github/managing-releases-in-a-repository), [immutable releases](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases), and [artifact attestations](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations).

macOS signing/notarization is desirable when a Developer ID is available, but it should not quietly block the first open-source release. Test the real download/quarantine journey and document any limitation precisely.

### 6. Publish the crate as a secondary channel

Publishing `uhm-cli` makes sense because it already contains a binary target and gives Rust users the familiar `cargo install uhm-cli` path. It is not the beginner installation path and must not block GitHub binaries.

Before publishing:

- Verify crate-name availability and ownership.
- Complete repository, homepage, documentation, keywords, categories, rust-version/MSRV, include/exclude, license, and binary metadata.
- Run `cargo package` and `cargo publish --dry-run` from a clean release tree.
- Verify the packaged crate builds without repository-only files and installs the `uhm` binary.
- Document that Cargo installation compiles locally and is secondary to release binaries.

Do not add Homebrew, apt, npm, an auto-updater, or a broad package-manager matrix until the first artifact cycle is stable and actual demand is visible.

### 7. Complete release documentation and an RC gate

Rewrite the README around outcomes and installation rather than architecture. Include:

- Direct Linux/macOS binary installation with checksum verification.
- API key setup and `uhm doctor`.
- Three realistic result-first examples plus review, dry-run, clarification, repair, and piping.
- Exactly what context and telemetry leave the device, with opt-out commands near first use.
- Local receipt location/retention and how to clear it.
- Child-shell behavior, the copy/run fallback for parent-state actions, and that automatic parent-shell integration is deferred to Plan 6.
- Explicit limitations: model mistakes, advisory warnings, no sandbox/safety promise, no universal undo, no background agent, no native Windows.

Run a release-candidate dogfood set across ordinary reads, file writes, package/network operations, detected deletion/sudo, failed command repair, answers, piped input, parent-shell actions, SSH/tmux, and plain mode. Freeze features during RC.

## Expected outcomes

- A new user can download a known binary, verify it, configure a key, understand outbound data, and complete a first job without Rust or a repository checkout.
- The CLI feels distinctive and enjoyable without compromising clarity, accessibility, or pipe behavior.
- Typed and locally recognized parent-shell requests never produce false success: v0.1 returns the exact action and explains that the user must apply it in the current shell; documentation states that arbitrary shell syntax cannot be classified completely.
- Maintainers receive cheap aggregate usage/outcome information without collecting the content of terminal work or creating stable user identities.
- Every public artifact is reproducible enough to test, checksummed, attributable to a release workflow, and documented for its actual supported platform.

## Definition of done

- A fresh Linux machine and both Apple architectures pass the documented install → doctor → first result journey from downloaded archives.
- The first-use disclosure precedes the first OpenAI and telemetry requests and accurately matches captured outbound request bodies.
- `uhm telemetry preview` exactly matches the current invocation's validated candidate event, and tests prove prohibited strings/data cannot enter it.
- Telemetry defaults on; configuration, environment, flag, and `DO_NOT_TRACK` opt-outs are evaluated before queue access or transmission. Concurrency/crash/ambiguous-response tests prove the queue is private, bounded, non-corrupting, and follows its documented loss/rare-duplicate semantics. `telemetry off` returns with no local send in flight; an unreachable endpoint neither adds more than the explicit post-result deadline nor changes the job outcome.
- Cloudflare Worker tests cover body size, schema version, unknown fields/enums, rate limits, 202 behavior, kill switch, and WAE field ordering. Dashboard queries use `SUM(_sample_interval)` when Analytics Engine sampling applies.
- Plain-mode snapshots contain no terminal controls; TTY snapshots work at 40/80/160 columns and under tmux/SSH.
- A copy/state matrix covers first progress, ordinary success, clarification, no-result, command failure, consequential warning, and the privacy notice. Review confirms ordinary states feel quick, playful, and capable; errors and consequential states remain literal; personality never changes result stdout.
- End-to-end release tests prove ordinary requests run without a review card or confirmation, detected consequential actions pause, `--review` always pauses, `--dry-run` never executes, and `--force` warns without prompting.
- `uhm feedback good|bad` records only the allowed enum and telemetry dashboards keep proposal, execution, and explicit feedback outcomes separate.
- Release CI builds, archives, smoke-tests, checksums, attests, and attaches all declared targets to a draft release.
- `cargo publish --dry-run` passes; crates.io publication is completed if ownership is ready, or explicitly deferred without blocking GitHub Release.
- Release notes state context default, telemetry default/fields/retention, receipt default, execution policy, parent-shell semantics, supported platforms, and absence of sandbox/undo guarantees.
- The RC task corpus has no open P0/P1 defect in parsing, outbound data, proposal validation, execution, terminal rendering, receipts, or warnings.
- The public v0.1 GitHub Release is published from a tag only after every required artifact and document is present.

## Anti-goals

- Do not ship generated standalone programs, Monty, Cloudflare Code Mode, a container runtime, or an arbitrary-code sandbox in v0.1.
- Do not add hosted accounts, login, cloud history, cross-device sync, or stable telemetry identifiers.
- Do not collect raw prompts, commands, output, paths, repositories, error strings, or persist IP addresses in product analytics or application logs.
- Do not add a background daemon, telemetry service process, automatic updater, or persistent agent.
- Do not claim that warnings make execution safe or that exit code zero proves intent success.
- Do not block the user's explicit `--force` authority because a heuristic labels an action consequential.
- Do not make Windows, Homebrew, apt, npm, shell completion, or every Linux distribution a first-release requirement.
- Do not let personality add extra confirmation, jokes during consequential actions, animation in plain mode, or noise on stdout.

## Primary code and infrastructure areas

`src/main.rs`, `src/render/*`, `src/lineedit.rs`, `src/tty.rs`, `src/dirs.rs`, `src/config.rs`, `src/secret.rs`, `src/history.rs`, a new telemetry module, `README.md`, `SECURITY.md`, `AI_POLICY.md`, `config.example.yaml`, `Cargo.toml`, and `.github/workflows/*`, plus a small independently deployable telemetry Worker directory.
