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

Allowed values are `tutorial`, `how-to`, `reference`, `explanation`, `navigation`, and `project`. The sidebar check verifies that each page appears under a compatible section.

## Keep the modes separate

- Tutorials may link to reference but should not enumerate every option or branch into alternative setups.
- How-to guides should solve one named task and link to reference for exhaustive details.
- Reference pages should describe the current contract without a narrative walkthrough.
- Explanation pages should avoid becoming operational runbooks or duplicated configuration tables.

When a subject needs several modes, give each mode its own page. Stable legacy paths such as `program.md`, `recovery.md`, `local-history.md`, and `model-selection.md` are navigation maps that route readers without breaking existing links.

## Avoid duplicated authority

Normative facts have one canonical home:

- flags and commands: `cli-reference.md`;
- configuration keys and defaults: `configuration.md`;
- provider capabilities: `reference/providers.md`;
- process and result semantics: `behavior-contract.md`;
- outbound data: root `PRIVACY.md`, mirrored by `docs/privacy.md`;
- domain contracts: the corresponding file under `docs/reference/`.

Tutorials and how-to guides should link to those references rather than restating complete tables.

## Validate changes

Run:

```sh
python3 scripts/check-docs.py
git diff --check
```

The documentation check validates release versions, CLI-help coverage, provider-neutral language, empty-manifest status, local links, Diátaxis metadata, and sidebar classification. CI runs the same check.
