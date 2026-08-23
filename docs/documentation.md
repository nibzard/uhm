<!-- diataxis: project -->

# Documentation structure

The documentation follows the Diátaxis framework. Classify a page by the reader's immediate need before adding or restructuring content.

| Type | Reader asks | Writing rule | Location |
|---|---|---|---|
| Tutorial | “Can you teach me?” | Lead the reader through one reliable learning sequence | `docs/tutorials/` or the root Quickstart |
| How-to | “How do I accomplish this?” | Assume a goal and give the shortest safe procedure | `docs/how-to/` plus established operational guides |
| Reference | “What exactly does this do?” | Be complete, neutral, structured, and lookup-oriented | `docs/reference/` plus canonical contracts |
| Explanation | “Why is it designed this way?” | Discuss concepts, constraints, alternatives, and tradeoffs | `docs/explanation/` and `docs/architecture/` |

Project history, release notes, navigation maps, and this maintainer guide live outside the four reader modes but carry explicit `project` or `navigation` metadata.

## Page metadata

Every page linked from `_sidebar.md` starts with one marker:

```html
<!-- diataxis: tutorial -->
```

Allowed values are `tutorial`, `how-to`, `reference`, `explanation`, `navigation`, and `project`. The sidebar check verifies that each page appears under a compatible section. The "Start here" section additionally accepts `explanation` pages, so `concepts.md` can open the visitor path.

## Keep the modes separate

- Tutorials may link to reference but should not enumerate every option or branch into alternative setups.
- How-to guides should solve one named task and link to reference for exhaustive details.
- Reference pages should describe the current contract without a narrative walkthrough.
- Explanation pages should avoid becoming operational runbooks or duplicated configuration tables.

When a subject needs several modes, give each mode its own page. Stable legacy paths such as `program.md`, `recovery.md`, `local-history.md`, and `model-selection.md` are navigation maps that route readers without breaking existing links.

## Canonical copy

Reader-facing pages share wording that is defined once and reused verbatim. The check fails when a page drifts from these forms.

- **Opening block.** The root `README.md`, `docs/README.md`, and `docs/_coverpage.md` open with the same three parts: the problem statement ("You know the result you want…"), the mechanism paragraph ("`uhm` is an AI assistant for the terminal…"), and the wedge line "The result, not the command." Edit the three files together.
- **Provider sentence.** `README.md`, `docs/README.md`, `docs/concepts.md`, `docs/install.md`, and `docs/troubleshooting.md` state it verbatim: "OpenAI is the default provider; Cerebras and DeepSeek are explicit alternatives." Two-provider phrasings are forbidden everywhere the check scans.
- **One term per concept.** Use *intent* for the input, *action* for the unit of work ("typed action" appears once, defined in `concepts.md`), *proposal* only for the pre-approval state under `--review` or `--dry-run`, and *short Python program* in reader copy. Reserve *microprogram*, *preimage*, and *cooked* for reference pages after the plain-English gloss.

## Privacy mirror

Root `PRIVACY.md` is normative. `docs/privacy.md` embeds its body byte for byte between a small preamble and a "See also" tail. To change privacy content, edit `PRIVACY.md`, then regenerate the mirror: copy everything after the `# Privacy` heading into `docs/privacy.md` between the blockquote and `## See also`. The check fails when the embedded body differs by one byte.

## Avoid duplicated authority

Normative facts have one canonical home:

- flags and commands: `cli-reference.md`;
- configuration keys and defaults: `configuration.md`;
- provider capabilities: `reference/providers.md`;
- process and result semantics: `behavior-contract.md`;
- outbound data: root `PRIVACY.md`, mirrored by `docs/privacy.md`;
- domain contracts: the corresponding file under `docs/reference/`.

The root `README.md` is a landing page, not a manual. It states the problem, shows one run, and links to the canonical pages above. New detail belongs in `docs/`, with a one-line link from the README.

Tutorials and how-to guides should link to those references rather than restating complete tables.

## Validate changes

Run:

```sh
python3 scripts/check-docs.py
git diff --check
```

The documentation check validates release versions, CLI-help coverage, provider-neutral language, the canonical provider sentence, empty-manifest status, local links, docsify link resolution, the privacy mirror, Diátaxis metadata, and sidebar classification. CI runs the same check.

Links inside `docs/` must resolve under both GitHub rules (relative to the containing file) and docsify rules (normalized from the site root). Prefer the `../architecture/NNNN-….md` form for links between nested pages; the check explains the failing form when it catches a mismatch.
