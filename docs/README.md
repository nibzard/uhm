# uhm

**uhm** is a fast natural-language layer over terminal tools. Ask it to count paragraphs, find a large file, explain a command, or do one local job. It chooses a typed action, runs ordinary work, and gives you the real output.

It is deliberately smaller than a coding agent. One intent goes in. One bounded job comes out. Then `uhm` exits.

[![A terminal demo of uhm turning natural-language requests into results](demo/uhm-demo.svg)](https://nibzard.github.io/uhm/demo/)

## What it does

- **Plain language in, real output out** — counts, lists, summaries, file transforms.
- **One bounded job per invocation** — at most two model calls, then it exits. No open-ended chat loop.
- **Stays legible** — result data on stdout; progress and review on stderr. `--plain` gives cooked ASCII-safe output.
- **You keep control** — `--dry-run` previews exact bytes; `--review` pauses every proposal; warnings flag deletion, broad writes, and privilege changes.
- **Private by default** — sends your intent and bounded context to OpenAI with `store: false`; content-free telemetry is opt-out.

## What it is — and isn't

| uhm is | uhm is not |
|---|---|
| A natural-language shortcut to ordinary terminal work | A coding agent that edits whole projects |
| One-shot: one intent in, one result out | A chatbot or persistent REPL |
| Honest about limits — exit zero only means the process exited zero | A sandbox or a safety guarantee |

## Install

```sh
cargo install --locked --git https://github.com/nibzard/uhm --tag v0.2.2 uhm-cli
```

Prebuilt binaries with SHA256 verification are on the [Install](install.md) page. You bring your own OpenAI API key; requests use the Responses API with `store: false`. See [Getting started](getting-started.md) for the under-five-minute path.

## Next steps

- [Getting started](getting-started.md) — first result in under five minutes
- [CLI reference](cli-reference.md) — every command and flag
- [Configuration](configuration.md) — every key, including `aliases` (local shortcuts with no API call)
- [Behavior & exit codes](behavior-contract.md) — the invocation/outcome contract
- [Privacy & telemetry](privacy.md) — exactly what leaves your machine
