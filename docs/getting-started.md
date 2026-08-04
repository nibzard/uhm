<!-- diataxis: tutorial -->

# Quickstart

A first result in under five minutes. This uses the default OpenAI provider and assumes you have already [installed](install.md) `uhm` and exported your key:

```sh
export OPENAI_API_KEY="sk-..."
```

## 1. Sanity-check the setup

```sh
uhm doctor            # local configuration and terminal checks
uhm doctor network    # confirm the selected provider is reachable and the key authenticates
```

The first time you run `uhm`, it prints a one-time disclosure to stderr and persists an owner-only notice marker before any request is sent. See [Privacy & telemetry](privacy.md) for exactly what leaves your machine.

## 2. Ask for one bounded job

Say what you need in plain language:

```sh
uhm list the three biggest files in this directory
```

`uhm` turns that into one typed action, shows you the proposal, runs the ordinary work, and prints the result. Result data goes to **stdout**; progress and the review UI go to **stderr**, so piping the result elsewhere just works.

Quote the intent whenever it contains `?`, `'`, `*`, or `!` — the shell expands those characters before `uhm` runs, and zsh (the macOS default) turns an unmatched `?` or `*` into a hard error:

```sh
uhm 'how many paragraphs are in README.md?'
```

zsh users who prefer to skip quoting can add `alias uhm='noglob uhm'` to `~/.zshrc`; an unpaired apostrophe still needs a quoted intent.

## 3. Preview before it runs

`--dry-run` prints the exact proposal without executing anything:

```sh
uhm --dry-run find files modified in the last day larger than 50MB
```

`--review` pauses on every proposal and offers `run`, `revise`, `edit`, `copy`, and `cancel`:

```sh
uhm --review count the markdown files under docs
```

These two flags (and `--force`) are mutually exclusive. See the [behavior table](behavior-contract.md) for the full interaction.

## 4. Pipe data in

Pipe UTF-8 text and ask about it. The bytes travel only with the model request:

```sh
cat NOTES.md | uhm count paragraphs
cat data.csv | uhm ask "how many columns does this file have"
```

A question about a file's content needs the file's bytes on stdin, as above — naming the file in a bare question gives the model nothing to read.

With `--local-input`, the piped body stays on-device and only a presence, byte-count, and UTF-8 status summary (plus any `--input-format` label) is sent, which the generated program reads from a private spool:

```sh
curl -s https://example.com/big.json | uhm --local-input --input-format application/json summarize the top-level keys
```

## 5. Explain and answer

```sh
uhm explain "tar -xzf release.tar.gz"
uhm ask "what does the -I flag mean on python3"
```

`ask` returns an answer for prose-valued work; `explain` returns a typed explanation. Neither executes a command.

## 6. Local shortcuts

Aliases are short triggers expanded **locally** — no API call, no API key. They are empty by default; add them under `aliases` in your config (see [Configuration](configuration.md)):

```yaml
aliases:
  gst: git status -sb
  ll: ls -lAhF
  ports: ss -tulpn
```

Then `uhm gst` runs `git status -sb` directly. The expansion still passes through local effect detection, so consequential effects are still flagged.

## Where to go next

- [CLI reference](cli-reference.md) — every command, flag, and exit code
- [Configure a provider](how-to/configure-providers.md) — switch to Cerebras or make a provider/model pair persistent
- [Configuration](configuration.md) — providers, credentials, model precedence, context, history, and telemetry
- [Model-selection design](explanation/model-selection.md) — fixed selection, fallback, and qualification status
- [Behavior & exit codes](behavior-contract.md) — the invocation/outcome contract
- [Watch the demo ↗](https://nibzard.github.io/uhm/demo/) — six real jobs in under a minute
