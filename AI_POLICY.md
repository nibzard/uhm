# AI-assisted contribution policy

AI tools are welcome in this project. Use them for code, tests, documentation,
review, research, or repetitive work if they help. What matters is that the
submitted change is understood, disclosed, and verified by a person.

## The short version

If AI materially helped produce the contribution:

1. Say so in the pull request.
2. Describe what the tool did.
3. Describe what you checked yourself.
4. Take responsibility for the result.

Autocomplete that only completes a token or familiar line does not need a
formal disclosure. Generated functions, tests, prose, designs, reviews, or
large edits do.

## A good disclosure

Put a short section in the pull request description:

```markdown
## AI assistance

- Tool: Codex
- Used for: tracing the safety path, drafting the patch, and suggesting tests
- Human review: inspected every diff, changed the allowlist design, and ran
  fmt, Clippy, all tests, and a release CLI smoke test
```

The tool or model version is useful context but not required if you do not know
it. We care more about the scope of assistance and the verification performed.

## Contributor responsibilities

You must:

- understand the submitted behavior well enough to maintain and explain it;
- review the complete diff, including generated tests and documentation;
- verify claims against the current code and actual command output;
- keep secrets, private code, customer data, and credentials out of prompts
  unless you are authorized to share them with that provider;
- check generated material for licensing or provenance problems;
- preserve unrelated changes and avoid destructive operations outside the
  stated task;
- disclose uncertainty instead of inventing test results, citations, issue
  context, or platform support.

Passing tests are necessary, but they do not transfer responsibility to the
tool. A generated test can encode the same misunderstanding as generated code.

## Repository-specific expectations

`uhm` sits between model output and a real shell. AI-assisted changes to these
areas need extra scrutiny:

- command classification and auto-run decisions;
- quoting, shell segmentation, and redirection parsing;
- terminal control sequences and clipboard behavior;
- secret, cache, and history storage;
- API streaming and structured response parsing.

For those changes, include a regression test and explain the failure mode in
plain language. Prefer conservative behavior when the code cannot classify a
command confidently.

## Commit authorship

The human contributor is the author of the commit and the accountable party.
Do not add an AI tool as a fake person, signatory, or copyright holder. If you
want the repository history to mention AI assistance, put it in the commit body
or pull request disclosure. Conventional Commit formatting still applies.

## Maintainer use of AI

Maintainers may use AI tools to summarize a pull request, reproduce a bug,
draft review comments, or inspect a patch. Merge and rejection decisions remain
the maintainers' responsibility. AI-generated review feedback should be checked
before it is posted.

## When disclosure is missing

Missing disclosure is usually fixed by asking for it. Deliberately hiding
material AI use, fabricating verification, or submitting code the contributor
cannot explain may lead to the pull request being closed.

We would rather review an openly AI-assisted, well-tested patch than pretend
these tools are not part of the work.
