# Plan 8 — Record an asciinema demo and embed it in the README

## Purpose and dependency

A first-time visitor should see `uhm`'s value within a few seconds, without installing Rust, cloning the repository, or reading the command table. This plan records one short, reproducible terminal demo with asciinema, renders it into an asset GitHub displays inline, and places it near the top of the README with a link to the interactive player.

It depends on nothing and can ship before v0.1. It also feeds Plan 3's README rewrite — Plan 3 §7 rewrites the README around outcomes and installation — so the demo asset, caption, and link should be designed to drop straight into that rewrite rather than arrive as a bolt-on.

Verified toolchain facts (2026-08-01): GitHub's markdown sanitizer strips `<script>`, so asciinema's official JS `<script>` player cannot be embedded in README markdown. The supported path is a render (animated SVG or GIF) committed as a repo asset and referenced as an image, plus an external link to the interactive player. `asciinema rec` records v3 casts by default; the official `agg` tool renders GIF from v1/v2/v3 casts; the widely used `svg-term-cli` renders animated SVG but only from v2 casts, so an SVG target must be recorded with `asciinema rec -f asciicast-v2`. SVG gives the best size-to-quality ratio and is preferred; GIF via `agg` is the universal fallback. Sources: asciinema [embedding docs](https://docs.asciinema.org/manual/server/embedding/), [agg docs](https://docs.asciinema.org/manual/agg/), [svg-term-cli](https://github.com/marionebl/svg-term-cli), and the asciinema [README-embedding discussion](https://github.com/asciinema/discussions/issues/283).

## Full implementation description

### 1. Choose the render format and where assets live

Decide: animated SVG is the in-README render; the `.cast` powers the interactive link; GIF via `agg` is the fallback. Record in v2 format (`-f asciicast-v2`) so both the SVG and GIF paths stay open from one source recording.

Keep every demo artifact under `docs/demo/`:

- `docs/demo/uhm-demo.cast` — the committed recording (small text file).
- `docs/demo/uhm-demo.svg` — the committed render referenced by the README.
- `docs/demo/uhm-demo.gif` — the `agg` fallback for viewers where SVG animation is disabled.
- `docs/demo/README.md` — toolchain prerequisites, versions, the privacy rules in §3, and the single rebuild command.

Reference the SVG in the README with ordinary markdown image syntax. Do not attempt a `<script>` embed — GitHub strips it. GitHub renders SMIL-animated SVG that is referenced as an image, so the inline animation plays; SVG also stays crisp at any width and is typically well under ~100 KiB. Cap total committed demo assets at a stated budget (for example 250 KiB) and let the rebuild script fail if a render exceeds it.

### 2. Pick demos that show result-first value in seconds

Keep the cast 30–60 seconds. Open with the hero and order by impact, using intents already documented in the README so the demo and docs agree:

1. Hero (read): `uhm list the three biggest files` — the answer appears with no separator, route, or context flag.
2. Ask over a pipe: `git diff | uhm ask write a one-line commit message`.
3. Multifile work: `uhm count the words, paragraphs, and headings in the markdown files in docs` — show that one natural-language request can select a suitable shell pipeline or bounded program.
4. Explain: `uhm explain what git log -p does`.
5. Dry run: `uhm run --dry-run concatenate the markdown files` — exact bytes shown, nothing executes.
6. Explicit yolo mode: `uhm --force remove build artifacts` — show the warning, skip confirmation under explicit user authority, and return a literal result. The disposable sandbox makes this demonstration harmless; do not imply that `--force` makes an action safe.

Every step runs inside a sandbox so no real repository, remote, or path is shown. Drive the steps from a committed script so the demo is reproducible rather than a one-off live take.

### 3. Make the recording reproducible and private

Add `scripts/demo/demo-script.sh` (the deterministic driver the recording runs) and `scripts/record-demo.sh` (build → sandbox → record → render → place assets → verify).

Reproducibility:

- Run from a `mktemp` sandbox populated with a few seeded files and a tiny local git repo with no remote, so output is stable across takes.
- Drive commands from the script with small, deliberate delays so the cast reads like typing; trim dead air with `asciinema rec --idle-time-limit` and `agg`/post-edit.
- Use a fixed terminal size (for example 80×24) and a real color `TERM` (for example `xterm-256color`) so colors and layout match the render. The demo deliberately shows styling and personality, so do not use `--plain` for the recorded run.
- Use normal `standard` context implicitly. Do not expose a context flag in the recording: the hero must look like the product's default path, while the throwaway sandbox keeps the bounded context impersonal.

Privacy and safety (hard rules):

- Never type or echo `OPENAI_API_KEY`. Inject it into the recording shell's environment before `asciinema rec` starts; do not export it inside the recorded session.
- After recording, `scripts/record-demo.sh` greps the `.cast` for forbidden patterns — `sk-`, real home paths, any configured key fragment, and real hostnames — and exits nonzero if any appear.
- The sandbox contains only throwaway seeded content; no real source, secrets, or remotes appear.
- The recording makes real OpenAI API calls; that is the only external effect and is expected. Privacy is handled by sandboxing and masking, not by mocking the model.

### 4. Embed in the README and link the interactive player

Add a concise **Demo** block near the top of the README, after the one-line intro and before *Quick start*:

- The animated SVG at a readable width: `![uhm demo](docs/demo/uhm-demo.svg)`.
- A one-line caption and a "▶ open the interactive player" link to the asciinema-hosted cast, or to a GitHub Pages page running the official player if self-hosting is preferred over asciinema.org.
- A short note that the demo makes a real model call and that styling is part of the product (not a mock).

Coordinate with Plan 3 §7 so the demo lands inside the README rewrite rather than alongside it.

### 5. Keep the demo from rotting

- One command rebuilds end to end: `scripts/record-demo.sh`.
- Optional CI check: regenerate the SVG from the committed `.cast` and diff against the committed copy, so a renderer or tool bump that changes output is caught. CI gates the committed asset from the committed cast, using no API key.
- Document the toolchain (asciinema, `agg`, and either `svg-term-cli` or the chosen SVG renderer) in `docs/demo/README.md` with versions noted, and recheck them before re-recording — matching the repo's dated-fact convention.

## Expected outcomes

- A visitor sees a working `uhm` session within seconds of opening the README, before installing anything.
- The demo is reproducible from committed scripts and rebuilds the same committed asset; no live, fumbly take.
- No secret, real path, remote, or personal data appears in the committed cast or render; a verification step enforces this.
- The render displays inline on GitHub (animated SVG) with a GIF fallback and an interactive-player link.
- The asset is shaped to drop into Plan 3's README rewrite.

## Definition of done

- `scripts/demo/demo-script.sh` and `scripts/record-demo.sh` exist; running the latter from a clean checkout rebuilds `docs/demo/uhm-demo.svg` byte-stably from `docs/demo/uhm-demo.cast` (modulo renderer version).
- The cast runs ≤ ~60 seconds, opens with a bare natural-language hero intent, and includes a read, an ask-over-pipe, multifile statistics, an explain, a `--dry-run`, and an explicit `--force` execution with literal wording.
- The record script builds `uhm`, runs in a throwaway sandbox at a fixed size and `TERM` with implicit standard context, injects `OPENAI_API_KEY` outside the recorded session, and exits nonzero if the cast contains `sk-`, real home paths, configured key fragments, or real hostnames.
- The README contains a Demo block near the top: the committed SVG renders inline on GitHub, plus a caption and a working link to the interactive player; a GIF fallback asset exists.
- `docs/demo/README.md` records the toolchain, versions, privacy rules, and the single rebuild command.
- If the optional CI check is added, CI regenerates the SVG from the committed cast and fails on drift, using no API key.

## Anti-goals

- Do not embed asciinema's `<script>` player in README.md — GitHub strips it.
- Do not type, echo, or commit an API key, real path, remote, or personal data.
- Do not record a live, ad-hoc take that cannot be reproduced; the demo must rebuild from scripts.
- Do not mock the model output — the demo shows a real model call, with privacy handled by sandboxing and masking rather than faking results.
- Do not claim safety in the consequential step; keep its wording literal and advisory.
- Do not make the demo depend on asciinema.org hosting as the only path — keep a committed render that works offline and on GitHub.

## Primary code and infrastructure areas

`README.md`, new `docs/demo/{uhm-demo.cast,uhm-demo.svg,uhm-demo.gif,README.md}`, new `scripts/demo/demo-script.sh`, new `scripts/record-demo.sh`, optional `.github/workflows/*` CI step, and `.gitignore` for sandbox intermediates.
