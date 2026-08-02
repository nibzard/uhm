# Rebuilding the uhm demo

The committed `uhm-demo.cast` is the canonical recording. The SVG shown in the
project README and the GIF fallback are derived from that cast. Rendering an
existing cast makes no OpenAI request:

```sh
scripts/record-demo.sh --render-only
```

The renderer is pinned to `svg-term-cli` 2.1.1 and `agg` 1.9.0. Install those
versions, then run the command above. It uses `npx` for the pinned SVG renderer
and expects the pinned `agg` binary on `PATH`.

To replace the source recording, also install asciinema 3.2.0, export
`OPENAI_API_KEY` before starting the recorder, and run:

```sh
scripts/record-demo.sh
```

The recording uses real Responses API calls for the hero, piped ask, and
explain examples in a disposable local Git repository with no remote. The
dry-run and cancellation use exact-intent aliases so their bytes and review
behavior stay deterministic. It sets an isolated home and XDG directories, fixes
the terminal at 80x24 with `TERM=xterm-256color`, disables demo telemetry and
history, and sends minimal context. The key is inherited by the child process;
it is never typed or echoed.

Before any cast or render is published, the script rejects recordings that
contain `sk-`, a fragment of the configured key, the real home directory, the
real hostname, or the checkout path. Review the cast manually as well: these
checks are a backstop, not proof that arbitrary private data is absent.

The combined cast, SVG, and GIF budget is 6 MiB. The SVG is the preferred web
asset; the GIF exists for clients that cannot animate SVG. The final removal
example operates only on a seeded `build` directory and is cancelled at the
real consequential-action prompt.

`index.html` hosts the interactive player on GitHub Pages. It loads the pinned
asciinema player 3.6.3 bundle from jsDelivr and reads the committed cast from
the same directory. The README links to `https://nibzard.github.io/uhm/demo/`;
the repository's Pages source must remain the `docs/` directory on `main`.
