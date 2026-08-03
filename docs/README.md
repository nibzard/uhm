<!-- diataxis: navigation -->

# uhm

**uhm** is a fast natural-language layer over terminal tools. Ask it to count paragraphs, find a large file, explain a command, or do one local job. It chooses a typed action, runs ordinary work, and gives you the real output.

It is deliberately smaller than a coding agent. One intent goes in. One bounded job comes out. Then `uhm` exits.

[![A terminal demo of uhm turning natural-language requests into results](demo/uhm-demo.svg)](https://nibzard.github.io/uhm/demo/)

## What it does

- **Plain language in, real output out** — counts, lists, summaries, file transforms.
- **One bounded job per invocation** — at most two model calls, then it exits. No open-ended chat loop.
- **Stays legible** — result data on stdout; progress and review on stderr. `--plain` gives cooked ASCII-safe output.
- **You keep control** — `--dry-run` previews exact bytes; `--review` pauses every proposal; warnings flag deletion, broad writes, and privilege changes.
- **Explicit provider boundary** — sends your intent and bounded context only to the selected fixed provider; content-free telemetry is separate and opt-out.

## What it is — and isn't

| uhm is | uhm is not |
|---|---|
| A natural-language shortcut to ordinary terminal work | A coding agent that edits whole projects |
| One-shot: one intent in, one result out | A chatbot or persistent REPL |
| Honest about limits — exit zero only means the process exited zero | A sandbox or a safety guarantee |

## Install

```sh
cargo install --locked --git https://github.com/nibzard/uhm --tag v0.3.0 uhm-cli
```

Prebuilt binaries with SHA256 verification are on the [Install](install.md) page. OpenAI is the default provider; Cerebras is an explicit fixed alternative. See [Getting started](getting-started.md) for the under-five-minute path.

## Choose what you need

- **Learn by doing:** start with the [Quickstart](getting-started.md), then [process local data](tutorials/local-data.md) or [modify and undo a file](tutorials/recover-file.md).
- **Complete a task:** use the [Cookbook](cookbook.md), [provider setup](how-to/configure-providers.md), [history guide](how-to/use-history.md), or [troubleshooting](troubleshooting.md).
- **Look up exact behavior:** open the [CLI](cli-reference.md), [configuration](configuration.md), [provider](reference/providers.md), [behavior](behavior-contract.md), or [privacy](privacy.md) reference.
- **Understand the design:** read [Core concepts](concepts.md), [model-selection design](explanation/model-selection.md), [trust boundaries](explanation/trust-boundaries.md), or the architecture decisions.
