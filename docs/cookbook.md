<!-- diataxis: how-to -->

# Cookbook

Short recipes for common goals. Each assumes `uhm` is installed and your API key is configured — see [Install](install.md) and [Quickstart](getting-started.md).

## Quick answers

Get a fact produced by a local tool:

```sh
uhm 'how many paragraphs are in README.md?'
uhm find the three biggest files in this directory
```

Quote any intent that contains `?`, `'`, `*`, or `!` — otherwise the shell expands those characters before `uhm` runs. See [Troubleshooting](troubleshooting.md#the-shell-rejects-the-intent-before-uhm-runs).

## File transforms

Run a change and keep the result pipeable:

```sh
uhm run concatenate the markdown files in docs and write combined.md
```

## Piped input

Feed exact bytes as request data:

```sh
git diff | uhm ask summarize this for a commit message
```

A question about a file's content needs the file's bytes on stdin — the ask route analyzes what is piped to it, not files named in the question:

```sh
cat meeting-notes.md | uhm 'what is this document about'
```

## Privacy-preserving input

Keep piped content on your machine while a generated program still processes it:

```sh
cat private_report.csv | uhm --local-input --input-format text/csv total the amount column
```

The model receives the intent, a byte count, UTF-8 status, and the format label — not the bytes. If it chooses the bounded Python route, the program receives a private local input path.

## Inspect before running

See the exact proposal without executing:

```sh
uhm run --dry-run count every occurrence of the word world in report.txt
uhm run --review remove old build artifacts
```

`--dry-run` prints exact command bytes and runs nothing. `--review` pauses every proposal at a prompt. `--force` skips the advisory prompt for a detected consequential action while still showing the warning.

## Explain only

Ask for prose without allowing execution:

```sh
uhm explain git log --first-parent --oneline
```

## Work with history

```sh
uhm history status
uhm history list --limit 20
uhm history show last
uhm history search -- failure
uhm history replay <run-id> --review
```

## Undo a change

With recovery enabled:

```sh
uhm recovery on
uhm run --recoverable rewrite report.txt as compact JSON
uhm undo last                 # hash-verified restore
uhm restore last --force      # reapply retained evidence
```

## Plain output for scripts

```sh
uhm --plain 'count the lines in *.md'   # ASCII-safe, no controls
uhm --json 'count the lines in *.md'    # machine-readable where supported
```

## Next

- [Configure a provider](how-to/configure-providers.md) — choose OpenAI or Cerebras
- [Configure fallback](how-to/configure-fallback.md) — add one typed-error alternate
- [Use history](how-to/use-history.md) — inspect, replay, export, and prune
- [Recover prior work](how-to/recover-work.md) — undo, force restore, resume, or recover
- [CLI reference](cli-reference.md) — the full surface
- [Program reference](reference/program.md) — exact generated-program contract
- [Troubleshooting](troubleshooting.md) — when something goes wrong
