<!-- diataxis: navigation -->

# uhm

Say what you need. Get the result. **The result, not the command.**

You know the result you want. The command, the flag, or the one-liner will not come. `uhm` is for that moment — the name is the sound it starts with.

`uhm` is an AI assistant for the terminal. Say the job in plain words. `uhm` picks one way to do it — a shell command or a short Python program — runs it, and prints the real output. Then it exits.

It is deliberately smaller than a coding agent. One or two model calls per job. Nothing loops, nothing stays running.

[![A terminal demo of uhm turning natural-language requests into results](demo/uhm-demo.svg)](https://nibzard.github.io/uhm/demo/)

## What it does

- **Plain language in, real output out** — counts, lists, summaries, file transforms.
- **One job per invocation** — at most two model calls, then `uhm` exits. No open-ended chat loop.
- **Stays legible** — result data on stdout; progress and review on stderr. `--plain` gives plain ASCII output.
- **You keep control** — `--dry-run` previews exact bytes; `--review` pauses every proposal; warnings flag deletion, broad writes, and privilege changes.
- **Explicit provider boundary** — your intent and a bounded context go only to the selected fixed provider. Each job spends a small amount of API credit, so you need a provider API key. Content-free telemetry is separate and opt-out.

## What it is — and isn't

<table class="comparison-table">
  <thead>
    <tr><th>uhm is</th><th>uhm is not</th></tr>
  </thead>
  <tbody>
    <tr>
      <td data-label="uhm is">A natural-language shortcut to ordinary terminal work</td>
      <td data-label="uhm is not">A coding agent that edits whole projects</td>
    </tr>
    <tr>
      <td data-label="uhm is">One-shot: one intent in, one result out</td>
      <td data-label="uhm is not">A chatbot or persistent REPL</td>
    </tr>
    <tr>
      <td data-label="uhm is">Honest about limits — exit zero only means the process exited zero</td>
      <td data-label="uhm is not">A sandbox or a safety guarantee</td>
    </tr>
  </tbody>
</table>

## Install

```sh
curl -fsSL https://nibzard.github.io/uhm/install.sh | sh
```

OpenAI is the default provider; Cerebras and DeepSeek are explicit alternatives. Set your key and verify it:

```sh
export OPENAI_API_KEY="sk-..."
uhm doctor network
```

The installer fetches the latest release archive for your platform, verifies `SHA256SUMS`, and installs to `~/.local/bin` by default. Source builds are still supported:

```sh
cargo install --locked --git https://github.com/nibzard/uhm --tag v0.6.6 uhm-cli
```

Prebuilt binaries, version pinning, and manual verification are on the [Install](install.md) page. See [Getting started](getting-started.md) for the under-five-minute path.

## Choose what you need

- **Learn by doing:** start with the [Quickstart](getting-started.md), then [process local data](tutorials/local-data.md) or [modify and undo a file](tutorials/recover-file.md).
- **Understand the design:** read [What is uhm?](concepts.md), [how uhm compares](comparison.md), or the [trust boundaries](explanation/trust-boundaries.md).
- **Complete a task:** use the [Cookbook](cookbook.md), [provider setup](how-to/configure-providers.md), [history guide](how-to/use-history.md), or [troubleshooting](troubleshooting.md).
- **Look up exact behavior:** open the [CLI](cli-reference.md), [configuration](configuration.md), [provider](reference/providers.md), [behavior](behavior-contract.md), or [privacy](privacy.md) reference.
