<!-- diataxis: explanation -->

# Core concepts

You know the result you want. The command, the flag, or the one-liner will not come. `uhm` is for that moment.

`uhm` is an AI assistant for the terminal — a natural-language layer over the terminal tools you already have. You describe a small job in plain words; `uhm` picks one concrete way to do it, runs that one thing, prints the real result, and exits.

It is deliberately not a coding agent. There is no open-ended loop, no background process, no daemon, and no autonomous editing session. One intent goes in. One bounded job comes out. Then `uhm` is done. A job is bounded: one child process, one time limit, one result.

## The shape of a request

Every invocation has the same shape:

1. **Intent** — the words after the options. Everything after the first intent word is opaque user text, so a phrase that happens to contain `-y`, `--help`, or `--system` cannot raise its own authority.
2. **A typed action** — `uhm` asks the model to choose one of a small, fixed set of action types (run a shell command, run a bounded Python program, answer a question, ask one clarifying question, or decline). It does not run free-form generated shell blindly; the action is chosen from a closed list. The chosen type is called the route.
3. **A bounded execution** — one child process, with timeouts, native terminal streams, and result bytes on stdout.
4. **The real output** — what the tool actually printed, not a paraphrase. Then `uhm` exits.

## What an intent is, and is not

An intent is a single, self-contained job: "count the paragraphs in README.md", "find the three biggest files", "summarize this diff".

It is not a conversation. If one essential detail is missing, `uhm` can ask exactly one clarifying question and revise once. A failed command can get one bounded repair attempt in an interactive terminal. After that, `uhm` exits with a status code. There is no chat.

## The first word is a boundary

The optional verb — `run`, `ask`, `explain` — sets the mode. After it, every argument is treated as your text, not as options. This is why a dictated instruction cannot smuggle in flags. Put `--` before an intent that itself starts with a hyphen:

```sh
uhm -- --weirdly-named-file
```

## Where the work happens

By default, the chosen command or program runs in your current working directory with your user permissions. `uhm` does not install a sandbox around it. It applies convenience limits — timeouts, output caps, a stripped environment for generated programs — but those reduce accidents, not hostile code. Review (`--review`, `--dry-run`) and the consequential-action warning exist to slow you down, not to contain execution.

## What can leave your machine

Three things can leave your machine on a normal request:

- **Your intent** (the words).
- **Explicitly piped input**, unless you use `--local-input`.
- **The selected context** — bounded environment facts. The default `standard` mode is OS, architecture, target shell, installed-tool booleans, a normalized working directory, bounded Git state, and up to 40 entry names. It does not include file contents, diffs, remotes, environment values, or history.

Requests go only to the selected fixed provider. OpenAI is the default provider; Cerebras and DeepSeek are explicit alternatives. OpenAI and DeepSeek use the Responses API with `store: false`; Cerebras uses its fixed Chat Completions endpoint. Content-free telemetry is separate and opt-out. See [Privacy & telemetry](privacy.md) for the exact boundary.

## What stays on your machine

Private, append-only metadata history stays on your machine by default — state transitions, route, effects, hashes, and timing categories, never the intent, proposal, paths, input, or output. See the [history reference](reference/history.md).

## Next

- [CLI reference](cli-reference.md) — every command and flag
- [Configuration](configuration.md) — providers, credentials, and selection policy
- [Model-selection design](explanation/model-selection.md) — fixed selection, fallback, and qualification
- [Program execution model](explanation/program-execution.md) — when and how `uhm` generates a program
- [Behavior & exit codes](behavior-contract.md) — what each exit status means
