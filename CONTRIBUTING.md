# Contributing to uhm

Thanks for taking the time to work on `uhm`. Small fixes are useful here. A
clear bug report, a focused documentation edit, or one missing safety test can
be more valuable than a large rewrite.

Participation is covered by the [code of conduct](./CODE_OF_CONDUCT.md). Report
security problems through the private process in [`SECURITY.md`](./SECURITY.md),
not a public bug report.

## Before you start

For a typo or an obvious bug, open a pull request. For a new command mode, a new
dependency, or a change to the safety model, start with an issue so the design
can be discussed before much code is written.

Please keep these constraints in mind:

- Correctness boundaries may use focused, maintained dependencies; document
  the MSRV, license, target support, and binary impact.
- Generated commands are untrusted input, even when the model returns valid
  structured data.
- Local effect detection is advisory, not an authorization or safety proof.
  Never turn a missed match into a claim that a command is safe.
- Prompts, commands, API responses, and terminal output may contain secrets or
  control characters.
- Preserve unrelated work in the tree. Keep the patch scoped to the problem.

## Development setup

Install a current stable Rust toolchain, then run:

```sh
cargo build
cargo test --all-targets
```

No API key is needed for the unit tests. You only need one for manual checks
that call the configured API.

Before submitting a change, run the full local gate:

```sh
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --release
```

Add focused regression tests for bug fixes. A test should demonstrate the old
failure and the intended behavior, especially for safety classification,
parsing, file permissions, and terminal handling.

## Making a change

1. Read the code path before editing it. The opaque-intent grammar and output
   channels have deliberately narrow contracts.
2. Keep behavior changes separate from unrelated cleanup.
3. Avoid destructive test commands. Use temporary directories and explicit
   paths, and clean up only files the test created.
4. Update the README or example config when user-visible behavior changes.
5. For documentation edits, follow the
   [documentation structure guide](docs/documentation.md). Canonical wording
   and the privacy mirror are check-enforced.
6. Record what you actually verified. Do not claim a manual check or platform
   test that you did not run.

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/) in the form:

```text
type(optional-scope): short imperative summary
```

Common types are `feat`, `fix`, `docs`, `test`, `refactor`, `perf`, `build`,
`ci`, and `chore`.

Examples:

```text
fix(policy): detect command substitutions as unknown effects
docs: explain API-free aliases
test(sse): cover streams that end before done
```

Keep the first line concise. Use the body for the reason behind the change,
tradeoffs, or migration notes. If a change breaks user-facing behavior, add a
`BREAKING CHANGE:` footer.

## Pull requests

A useful pull request explains:

- the user problem;
- the chosen fix and any tradeoffs;
- the tests or manual checks that were run;
- whether documentation or configuration changed;
- whether AI tools materially contributed to the patch.

Reviewers may ask for a smaller patch or another regression test. A focused,
well-tested change is simply easier to trust, whoever wrote the first draft.

## AI-assisted contributions

AI-assisted work is welcome. The contributor remains responsible for every
line submitted and must be able to explain the change. Material AI use belongs
in the pull request disclosure described in [`AI_POLICY.md`](./AI_POLICY.md).

## License

By contributing, you agree that your contribution is licensed under the
project's [MIT License](./LICENSE).
