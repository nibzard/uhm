# Plan 17 — Make the advertised surface work on a default install

## Purpose and dependency

A black-box session on the shipped `uhm 0.3.5` binary, run against a disposable directory with no repository knowledge, produced correct results for every read-only intent it was given and then failed on almost everything the help text advertises around them.

Four observations frame this plan.

`(sleep 20 | uhm count files here)` took 20.6 seconds. The same intent with `</dev/null` took 2.8 seconds. `uhm` drains stdin to EOF before it does anything else, with no deadline and no indication that it is waiting. In a loop of three intents launched from a non-interactive parent whose stdin stayed open, the session stalled for a full five minutes — matching `execution.timeout_secs: 300`.

Typing `y` at `Run, revise, edit, copy, cancel? [R/v/e/c/q]` cancelled the job. The file was untouched and the message was `uhm: cancelled by user`. The same binary accepts `y`/`yes` as an affirmative at five other prompts.

`uhm undo last` returned `no retained recovery manifest is available`, and `uhm repair last` returned `repair unavailable: the original intent was not retained`. Both are correct statements about a default install and neither is reachable as a working command without changing configuration first. Four of the ten command forms in `--help` — `repair`, `recover`, `undo`, `restore` — cannot complete on a fresh installation, and `history` cannot answer what any past run did.

Separately, `rewrite fresh.csv in place with the rows reversed` failed two of three identical attempts with `A writable resource has no statically visible write_path use.` — internal vocabulary for a real logic defect. The AST checker only recognises a write when `.write_path` hangs directly off a `resource("literal")` call, so binding the handle to a variable first — the natural shape for reading and writing one file — is refused. `uhm`'s own read-write test at `src/program.rs:1525` is written in the rejected style.

Most of this is not a design failure. The proposal block, the effect detector, the `rm` gate, the fail-safe cancel-on-EOF, and the verified-restore machinery all behaved correctly when they were reached; the defects are in the paths that lead to them. Section 7's checker bug is the exception, and it is the one item here that silently costs correct work.

It depends on Plan 10's CLI-truthfulness and recovery-lifecycle work and on Plan 16 §1's review-interaction repairs, and it revises one line of the latter. It requires no change to the conversation boundary, the outbound context, the privacy policy version, or the telemetry schema.

## Product thesis

> A command that appears in `--help` must either work on a default installation or say exactly what to change so that it will.

The North star in `plans/README.md` is that a user receives a resolved job faster than recalling the syntax. Every finding here costs that: a five-minute silent stall, a cancelled job from the most likely keypress, and four commands whose first invocation is a dead end. The remedy is not new capability. It is making the existing capability reachable.

Two boundaries this plan must not cross. It does not weaken any confirmation gate — `y` must become an affirmative at a prompt that is already showing the command, its effects, and its assumptions, not a way to skip that prompt. And it does not turn `metadata` history or disabled recovery into on-by-default; the settled decisions that keep intents and file bytes off disk by default stand. What changes is that the commands depending on them explain themselves.

## Why this is not a new subsystem

Every mechanism this plan needs already ships.

| Need | Existing code |
| --- | --- |
| Read piped stdin under a byte bound | `input::Spool::read`, `src/input.rs:12` |
| Precedent for a bounded deadline on a blocking read | `context::gather(mode, shell, timeout_ms)`, `src/context.rs:72` |
| Review prompt assembled from the live option set | `src/command.rs:1679` |
| Review key dispatch | `src/command.rs:1700` |
| Affirmative parsing that already accepts `y`/`yes` | `src/command.rs:256`, `:575`, `:864`, `:1407`, `:1903` |
| Recovery-enabled predicate | `recovery::effective_enabled`, `src/recovery.rs:377` |
| Manifest state machine with restorable-state set | `src/recovery.rs:972`, `:1069` |
| Full-intent retention under an explicit detail level | `src/history.rs:506` |
| History rows already carrying `mode` and `outcome` | `history::list`, `src/history.rs:1120` |
| An ineligible-recovery reason already rendered on one route | `program_preview`, `src/command.rs:2013` |
| A requested-but-ineligible capture already blocking execution | `src/command.rs:771` |
| Declared and detected effects already kept apart on the receipt | `src/command.rs:1942` |
| A softer validator branch for unprovable writes | `dynamic_write`, `src/program.rs:155` |
| A pty keystroke test harness | `reviewed_with_keystrokes`, `tests/cli_contract.rs:68` |

Section 1 adds one deadline to one existing read. Sections 2–6 change message text, one match arm, one boolean argument, and two render functions.

## The binding constraint

`plans/README.md` records the settled conversation boundary: at most two model calls and two executions per job, with one global second-call slot. **This plan spends no model call.** Nothing in it adds a request, a probe, or a retry. Section 7 is the only section that touches the model's output, and it changes how an existing program is *analysed and reported* — not how many times one is requested. Its validator change should in fact reduce second-turn repair attempts, since it stops refusing valid programs.

Section 2 also has a hard constraint from Plan 16 §1, which shipped in v0.3.5: the prompt string is derived from the live option set so a spent budget slot is never advertised. Section 2 must extend that derivation, not bypass it.

## Settled scope

| Topic | Decision |
| --- | --- |
| stdin | A non-TTY stdin that has produced no bytes gets a bounded wait, then proceeds with empty input. Once a first byte arrives, the read continues to EOF exactly as today, so a real producer is never truncated. |
| stdin bound | New checked constant, defaulted to 1 s, overridable in config; never unbounded. The bound is on the producer's time to first byte, not on total read time. |
| Affirmatives | `y`/`yes` join `""`/`r`/`run` as Run at the review prompt. No other key gains executing power. |
| Cancel key | `q` remains cancel. `c` remains copy. The prompt stops relying on the initial letter alone to disambiguate them. |
| Unknown keys | Re-prompt once with the live option list instead of silently cancelling. |
| Recovery preconditions | `undo`/`restore`/`recover` consult `effective_enabled` before manifest resolution and name the setting when it is off. |
| `last` alias | Resolves to the most recent manifest in a restorable state; a non-restorable newer manifest never shadows it silently. |
| `prune` | `uhm recovery prune` gains `--all`; the `recovery off` and `recovery on` messages name the command that can actually remove retained snapshots. |
| History default | Stays `metadata`. `repair`/`recover` state the required setting, which they already do, and `history list`/`show` become readable without retaining intents. |
| Program validator | Local-name resolution is added to the existing static check. Anything still unprovable becomes the existing warning, never a hard error. |
| Effects rendering | Keeps the declared/detected union; adds a visible distinction. Never gates execution on a mismatch. |
| Comprehension intents | Documentation only: the stdin idiom for asking about a file's content gets a fenced example and a guidance line, and the intent class joins the benchmark corpus. Any prompt or routing change edits the outbound context and is deferred to Plan 13. |
| Defaults changed | None. Every fix is to messages, parsing, alias resolution, rendering, one static-analysis branch, or a new bounded deadline. |
| Non-goals | Making recovery or full history on by default; a REPL; universal rollback; a shell-route recovery class; any new outbound field; changing the program contract schema. |

## Measured behavior before implementation

One black-box session on `uhm 0.3.5`, macOS 25.5.0 / aarch64, `gpt-5.6-terra`, default config, on 2026-08-04. Read-only intents were correct in every case checked (4 CSV data rows, 1110 column sum, 88 README paragraphs, correct three largest files). This is a single-operator session, not a benchmark.

| Probe | Observed |
| --- | --- |
| `(sleep 20 \| uhm count files here)` | 20.6 s wall, then correct answer |
| Same intent with `</dev/null` | 2.8 s |
| Three intents in a loop, parent stdin open | Stalled to a 5-minute timeout |
| `y` at the review prompt, real pty | `uhm: cancelled by user`; target file unchanged |
| `uhm undo last`, fresh install | `no retained recovery manifest is available`, exit 11 |
| `uhm repair last` | `repair unavailable: the original intent was not retained` |
| `uhm history list` | Run ids, raw epochs, route, event counts; no intent, no command |
| `uhm history show <id>` | Raw JSONL event lines |
| `uhm recovery prune` with 2 retained snapshots | `pruned 0 snapshots (0 bytes)`; status still showed 2 / 61 bytes |
| `uhm recovery off --prune` | `pruned 2 snapshots (61 bytes)` |
| `--recoverable` on a shell-route append | 0 snapshots captured, no warning |
| `--recoverable` on a program-route in-place rewrite | 1 snapshot, `Verified restore: available` |
| `uhm undo last` after that rewrite | Plan printed, exit 11, no next step named |
| `uhm undo last --force` | Refused, pointed to `restore --force` |
| `uhm restore last --force` | Restored correctly |
| `rewrite fresh.csv in place with the rows reversed`, 3 identical `--fresh` attempts | 1 success, 2 × `A writable resource has no statically visible write_path use.` |
| `uhm --dry-run delete the logs directory` | `rm -rf -- logs` with no trailing newline |
| `uhm how many paragraphs are in README.md?` (README.md:78) verbatim in zsh | `zsh:1: no matches found: README.md?` |
| Same in bash | `88` |
| Counting CSV rows | Proposal showed `Effects: writes local data, reads local data`; event recorded `detected_effects:["read_local"]` |

Two claims from the session were withdrawn on inspection. Bare `uhm` is not a dead end: `src/main.rs:341` prompts and `src/main.rs:343` reads an intent from `/dev/tty`, and an interactive run of bare `uhm` answered `count the files here` correctly. Exit 11 for `undo` on a fresh install is consistent with the documented meaning at `docs/cli-reference.md:187`; the defect is the message, not the code.

One probe comes from a later session, on the shipped v0.3.6 run inside this repository on 2026-08-04: `uhm what is this plan 17 about` printed a listing of every file in `plans/` followed by `✓ Finished`. The same session resolved read-only shell intents (process listings, file sizes, line counts, git state) correctly. It is recorded as RTE-1 in section 6.

## Finding disposition

| ID | Finding | Section |
| --- | --- | --- |
| IN-1 | Non-TTY stdin is drained to EOF with no deadline before any work begins | 1 |
| IN-2 | The behavior contract says "explicitly supplied" stdin; the implementation drains any non-TTY stdin | 1 |
| REV-1 | `y`/`yes` at the review prompt cancel the job, while the same binary accepts them elsewhere | 2 |
| REV-2 | `c` means copy and cancel is `q`, in a prompt that executes shell commands | 2 |
| REV-3 | An unrecognized key silently cancels with no re-prompt | 2 |
| REC-1 | `undo`/`restore` never consult `recovery.enabled`, so a disabled install and a manifest-less run give one message | 3 |
| REC-2 | The `last` alias resolves by `updated_at` with no state filter, so a restored run shadows a restorable one | 3 |
| REC-3 | `uhm recovery prune` cannot remove retained snapshots, but two messages direct users to it | 3 |
| REC-4 | `--recoverable` is silently inert on the shell route | 3 |
| REC-5 | Non-interactive `undo` prints a plan and exits 11 without naming the command that completes it | 3 |
| HIS-1 | `repair` and `recover` are unreachable under the default history detail | 4 |
| HIS-2 | `history list` drops `mode` and `outcome` it already has, and prints raw epochs | 4 |
| HIS-3 | `history show` has no rendered view, only a raw JSONL dump | 4 |
| CLI-1 | The plain `--dry-run` and review-`copy` branches emit no trailing newline | 5 |
| DOC-1 | The documented first example fails under zsh, and no quoting guidance exists | 6 |
| DOC-2 | `doctor` column alignment breaks on multi-word statuses | 6 |
| RTE-1 | A comprehension question about a local file's content resolves to an unrelated listing presented as a finished job | 6 |
| PRG-1 | The AST checker hard-errors on a resource handle bound to a local name, rejecting the idiomatic in-place rewrite | 7 |
| PRG-2 | Validator reason text and codes reach the terminal unmapped | 7 |
| EFF-1 | The `Effects:` line renders the declared/detected union without distinguishing them | 7 |
| EFF-2 | The program route computes effects and never renders them | 7 |

## 1. Never block a job on stdin that will not arrive

`src/input.rs:12` returns early when stdin is a terminal, and otherwise calls `read_to_end` on a `take(max + 1)`. There is no timeout, poll, or select in the file. The byte cap cannot help, because a producer holding the pipe open sends nothing at all. `src/main.rs:334` calls this before intent resolution (`:338`), route classification, model selection, and key resolution, so the drain gates the whole pipeline and overlaps with nothing.

Any non-interactive caller whose stdin is an open descriptor that never closes hangs: CI runners, agent harnesses, `ssh` without `-n`, cron with inherited descriptors. The user sees no output and no spinner, because nothing has started.

`docs/behavior-contract.md:54` and `:56` describe "explicitly supplied UTF-8 stdin" and "explicitly piped stdin". The implementation has no notion of explicit — it drains whatever is not a tty. IN-2 is that gap.

- Add a bounded first-byte deadline to `input::Spool::read`. If stdin is not a terminal and produces no byte within the deadline, return the empty spool and proceed. Once the first byte arrives, read to EOF under the existing byte cap with no further deadline, so a slow legitimate producer (`git diff | uhm ask ...`) is never truncated mid-stream.
- Make the deadline a checked constant with a config key, following `context_timeout_ms` (`src/config.rs`, surfaced in `uhm config show`). Default it to 1 s. The quantity being bounded is the producer's time to first byte, not the job's wall clock, so the 2.8 s job floor is not the right yardstick: `sort bigfile | uhm dedupe this` emits nothing until sort has consumed its entire input, and a cold `git diff` or a `curl` can sit past 250 ms before their first byte. Expiring on a legitimate slow producer silently substitutes empty input and changes the answer, which is a worse failure than the stall this section removes; 1 s keeps that risk low while still cutting the observed five-minute hang by two orders of magnitude.
- Never let the deadline apply to a spool that has already yielded bytes, and never let it silently truncate. A cap breach must keep its current explicit error.
- When the deadline expires with zero bytes, say so on stderr unconditionally — one line naming the elapsed bound and that the job is proceeding without piped input. Silence is what made this cost five minutes, and the deadline's one false-negative mode (a producer whose first byte arrives after the bound) is only diagnosable if the notice always prints. stderr keeps it out of every pipe; `-v` must not be required.
- Update `docs/behavior-contract.md:54`/`:56` and `docs/troubleshooting.md` to state the actual rule, and document `</dev/null` as the explicit way to declare no input.

## 2. Make the confirm prompt accept what users type

`src/command.rs:1700` dispatches the review key:

```rust
"" | "r" | "run" => ReviewDecision::Run,
"v" | "revise" => ...
"e" | "edit"   => ...
"c" | "copy"   => ReviewDecision::Copy,
_ => ReviewDecision::Cancel,
```

`y` falls to `_` at `src/command.rs:1722` and cancels. This was confirmed on a real pty: the prompt appeared, `y` was delivered, `uhm: cancelled by user` was printed, and the target file was unchanged. The same file accepts `y`/`yes` as affirmatives at `src/command.rs:256`, `:575`, `:864`, `:1407`, and `:1903`, and `src/main.rs:844` does too. A user who has just answered `y` to one `uhm` prompt learns the wrong lesson for this one.

The existing test at `src/command.rs:2574` asserts `n`/`no` cancel and `:2581` asserts an unknown key cancels. Neither covers `y`, which is why this survived.

- Accept `y` and `yes` as Run. This does not weaken the gate: the prompt is already displaying the command, its declared effects, the detected risks, the shell, the cwd, and the assumptions. Confirming it is the point of the prompt.
- REV-3: route unrecognized input to a single re-prompt that restates the live options, rather than to Cancel. Preserve EOF as Cancel — `src/command.rs:1222` and `:686` are the fail-safe path and must stay. Bound the re-prompt so a closed or hostile terminal cannot loop.
- REV-2: `c` for copy and `q` for cancel invert the common reading of both letters. Do not remap the keys — muscle memory from v0.3.x exists and `q`-to-quit is defensible. Instead stop making the letter carry the meaning alone: render the copy option so it cannot be misread as cancel, and keep the full words accepted. Plan 16 §1 currently instructs "Keep `c` and `q` always available"; that line stands for availability and is amended here for presentation.
- Add the missing coverage: `y`, `yes`, `n`, an unknown key, and EOF, each asserted against both the decision and whether the target file changed.

## 3. Make the recovery commands explain why they cannot run

Four defects, all in the path to machinery that works. The verified-restore engine itself behaved correctly: it captured a preimage, detected a genuine postimage conflict with hashes, refused to call a forced operation verified, and restored the file byte-for-byte when explicitly forced.

**REC-1 — a disabled install and a manifest-less run give the same message.** `recovery::resolve_manifest_run` (`src/recovery.rs:829`) emits `no retained recovery manifest is available` from `:838` when the runs directory is absent and from `:860` when no manifest was found. `src/main.rs:624` maps any error to `recovery_unavailable`. Nothing on the undo path consults `recovery::effective_enabled` (`src/recovery.rs:377`), which exists and is used only by `recovery status` and `config show` (`src/main.rs:896`). On a default install the true cause is always `recovery.enabled: false`, and the message never says so.

- Check `effective_enabled` before manifest resolution in `undo`, `restore`, and `recover`. When capture was never enabled, say that, name `uhm recovery on` and `--recoverable`, and return the configuration status (13) rather than the not-executed status (11) — the work was not merely unexecuted, it was never capturable.
- Keep the manifest-absent message distinct for the enabled case.

**REC-2 — `last` can resolve to a run that cannot be undone.** `src/recovery.rs:848` skips only invalid ids and missing manifest files; `:854` then picks the greatest `updated_at`. There is no `RecoveryState` filter, and `updated_at` moves when a run is restored. So a just-restored manifest wins the alias and `undo last` reports `recovery manifest is restored, not restorable` (`src/recovery.rs:979`, `:1076`). Observed twice: once after a restore, and once after a failed run, where `undo last` targeted an older unrelated run and printed a legitimate conflict for it.

- Resolve `last` to the most recent manifest whose state is in the restorable set already defined at `src/recovery.rs:972` and `:1069`.
- When a newer, non-restorable manifest was skipped to get there, say which run was chosen and why. Silently operating on an older run is the hazard; naming it is the fix.

**REC-3 — the advertised prune cannot prune.** `src/main.rs:820` invokes prune with `all = false`. The predicate at `src/recovery.rs:1601`/`:1604` then keeps any manifest that is newer than the age cutoff and within the total cap, so a fresh small snapshot is always skipped. `uhm recovery off --prune` passes `all = true` (`src/main.rs:798`) and does remove them. Meanwhile `src/main.rs:807` tells the user to `use \`uhm recovery prune\` to remove them now`, and the `recovery on` disclosure at `src/main.rs:789` also points at `prune`. Observed: `prune` removed 0 of 2 snapshots; `off --prune` removed both. `recovery status` counts with a third predicate (`src/recovery.rs:1326`) that ignores age and pinning, which is why the two disagreed on screen.

- Add `--all` to `uhm recovery prune` and thread it to the existing parameter. `--dry-run` must honour it.
- Correct `src/main.rs:807` and `:789` to name the form that removes retained snapshots.
- Make the no-op case legible: when prune skips candidates because they are within age and cap, report the skip and the reason instead of only `pruned 0 snapshots`.

**REC-4 — `--recoverable` is inert on the shell route, and the tool already knows it.** A `printf ... >> notes.txt` job accepted `--recoverable`, reported success, and captured nothing; `recovery status` showed 0 manifests. This is structural and correct: the sole manifest writer is `recovery::prepare_with_lease` (`src/recovery.rs:529`), reached only from `src/program.rs:443`, and it requires staged managed outputs (`src/recovery.rs:537`). A shell redirection has none. The narrow managed-file class is a settled decision in `plans/README.md` and this plan does not widen it.

What is wrong is that `uhm` already computes the explanation and throws it away. `src/command.rs:1153` detects the requested-but-impossible case and records a history event whose reason string, at `src/command.rs:1166`, is `shell execution has a receipt but no controlled preimage`. Nothing prints it. The parent-shell route does the same at `src/command.rs:396`, reason at `:409`.

The program route already does this right. Its equivalent reason — `stdout-only programs have no managed artifact preimage`, `src/command.rs:632` — is rendered by `program_preview` at `src/command.rs:2013` as a `Recovery:` line, and a requested-but-ineligible capture blocks execution at `src/command.rs:771`.

- Render the existing reason string on the shell and parent-shell routes, as a `Recovery:` line in the same block that already prints effects and assumptions. The text exists; only the `eprintln!` is missing.
- Match the program route's stronger behavior for an explicit `--recoverable`: when the user asked for a preimage that this route cannot produce, stop rather than proceed silently. `--force` still overrides, consistent with every other gate.
- The stop is a documented-behavior change, not a message fix: a script passing `--recoverable` on a shell-route job exits 0 today and would refuse afterward. Under the roadmap's patch-compatibility rule it must ride a minor release and lead that release's changelog. The `Recovery:` line rendering above carries no such constraint and may ship in a patch with the rest of this section.

**REC-5 — non-interactive `undo` names no next step.** With a valid manifest and no tty, `undo` printed the plan and the concurrency caveat, then exited 11 with no instruction. `uhm undo last --force` then refused — correctly, since force cannot make a conflicted operation verified (`src/main.rs:621`) — and pointed to `restore --force`, which worked. The final message is good; the first one is a dead end.

- When `undo` stops for confirmation, name the exact command that proceeds, distinguishing the verified path from the forced one.

## 4. Make history answer "what did I run?"

**HIS-1.** `src/history.rs:506` retains the intent only when `detail == Full`, and otherwise writes `intent_hash`. `repair_seed` (`src/history.rs:1002`) therefore fails at `:1015`, and `recovery_seed` (`:1033`) at `:1053`. The message already names the fix, which is why this is the least severe of the four unreachable commands. Keeping intents off disk by default is a settled privacy decision and does not change.

- Leave the default. Make `--help` stop presenting `repair` and `recover` as unconditional forms: mark them as requiring retained history, the same way `restore` already shows its required `--force`.
- Have `uhm doctor` report whether `repair`/`recover`/`undo` are currently usable. `doctor` is already the best surface in the tool and is where a user looks before filing a bug.

**HIS-2 / HIS-3.** `history::list` (`src/history.rs:1104`) returns `mode`, `events`, `failed`, and `outcome` per row (`:1120`). The human renderer at `src/main.rs:941` prints only run id, raw epoch, route, and event count — dropping `outcome` and `mode` it already holds. `history show` (`src/main.rs:961`) serialises each event to a JSONL line at `:975`. Neither retains an intent, so neither can be fixed by printing one; both can be made readable from data already present.

- Print `outcome` in `list`, and render the timestamp as a local, human-readable time. An unreadable epoch is why the listing could not be used to pick a run id for `repair` or `undo`.
- Give `show` a rendered default: one block per event with its kind, relative time, and the fields that matter for that kind, with the raw JSONL preserved behind `--json`.
- Neither change may print a redacted or content-bearing field that `--json` export would withhold. The allowlisted export schema from Plan 10 §4.1 is the reference for what is printable.

## 5. Fix the output that runs into the next prompt

**CLI-1.** `write_command` (`src/command.rs:2075`) does `write_all` then `flush` with no newline. The plain `--dry-run` branch at `src/command.rs:2071` uses it, so `uhm --dry-run delete the logs directory` emits `rm -rf -- logs` with no line terminator and the next shell prompt appends to it. The `--json` branch at `:2057` uses `println!` and is correct. The review `copy` action shares the helper (`src/command.rs:1301`, `:754`).

`--dry-run` output is meant to be piped and eyeballed, so the fix must not add a byte that a consumer would have to strip.

- Terminate the plain-text command with a single newline when stdout is a terminal. Keep the exact bytes with no terminator when stdout is not a terminal, so `uhm --dry-run ... | sh` and byte-comparison tests are unaffected.
- Assert the trailing byte in both cases. No test in `src/` or `tests/` currently covers it.

## 6. Fix the documented first run

**DOC-1.** `README.md:78` and `docs/cookbook.md:12` both give `uhm how many paragraphs are in README.md?`. Under zsh — the default shell on macOS, which the install instructions target — this fails with `zsh:1: no matches found: README.md?` before `uhm` is executed at all. It works in bash. `uhm what's the biggest file here` fails with `unmatched '`. A search of `README.md` and `docs/` for quoting, globbing, or `noglob` guidance returns nothing.

This is inherent to unquoted natural language on a CLI and cannot be fixed in the binary: zsh expands before the `shell-init` function is entered, so the wrapper cannot help either. But `?` ends most questions and apostrophes are unavoidable in English, so the first example a user copies must not be the one that breaks.

- Change the documented examples so no copied line depends on shell quoting luck, and verify every fenced `uhm` example in `README.md` and `docs/` under both zsh and bash.
- Add one short paragraph to `docs/getting-started.md` and `docs/troubleshooting.md`: quote the intent when it contains `?`, `'`, `*`, or `!`, and show the quoted form. Mention `noglob uhm` for zsh users who want an alias.
- Add a documentation test that extracts fenced `uhm` invocations from `README.md` and `docs/` and asserts each survives both shells without expansion errors. The failure happens at expansion, not parse — `zsh -n` accepts `README.md?` and only `zsh -c` fails on it — so the test must place a stub `uhm` on `PATH` that records its argv and run each example live under `zsh -c` and `bash -c`. This class of defect is otherwise invisible to CI.

**DOC-2.** `uhm doctor` aligns its status column by assuming a single-word status; `child environment warning` and `provider network skipped` push their detail text out of alignment. Cosmetic, but `doctor` is the tool's best surface and the misalignment reads as broken output. Pad from the rendered width.

**RTE-1 — a comprehension question resolves to a confident non-answer.** `uhm what is this plan 17 about`, on v0.3.6 inside this repository, printed a listing of every file in `plans/` followed by `✓ Finished`. The mechanics are in the routing rules of `DEVELOPER_INSTRUCTIONS` (`src/prompt.rs:8`): run routes may not return prose, and ask/explain routes may only analyze text supplied on stdin. A question about a named file's *content* therefore has no reachable correct output on the run route — no single command summarizes a document — so the model emitted one that pattern-matched the words instead, and the receipt called the job finished. This is the same confidently-wrong shape the North star exists to prevent: an unresolved job presented as a resolved one.

The intent already has a supported spelling. `cat plans/17-*.md | uhm what is this about` puts the text on stdin, where the routing rules direct the ask route to analyze it directly and answer in prose. This is a contract claim, not a measured one — the piped form was not exercised in either session — so the documentation change below must verify it live before shipping the example. No fenced example in `README.md` or `docs/` shows a file piped into a question, so the working form is undiscoverable exactly where the broken form is natural.

- Document the stdin idiom in the same pass as DOC-1: one fenced example in `README.md` and `docs/cookbook.md` piping a file into a question, and a line in `docs/getting-started.md` stating that a question about a file's content needs the file's bytes on stdin. These examples fall under section 6's two-shell documentation test like every other.
- The in-binary remediations — steering the model toward `head`/`sed -n` as the best one-command resolution of "what is X about", or letting an ask route read a named file — both edit `DEVELOPER_INSTRUCTIONS` and therefore the outbound context, which this plan does not change. Add the comprehension-intent class to the benchmark corpus so its resolution rate is measured; the prompt revision itself belongs to Plan 13's model-policy track and requalifies under Plan 14.

## 7. The in-place rewrite rejection, and effect honesty

**PRG-1 — the validator rejects the idiomatic in-place rewrite.** Three identical `--fresh` attempts at `rewrite fresh.csv in place with the rows reversed`, same directory, same file, produced one success and two failures reading exactly:

```text
uhm: A writable resource has no statically visible write_path use.
```

This is not primarily a message defect. The AST checker embedded at `src/program.rs:105`, run as `python3 -I -S -c AST_CHECKER` from `preflight` (`src/program.rs:259`), computes `missing_writes` at `src/program.rs:153` and hard-errors at `:154`. A declared writable resource is only counted as written if the source contains `.write_path` whose receiver is *syntactically* a `resource("<literal>")` call — `src/program.rs:132`.

So:

```python
resource("target").write_path.write_text(...)      # passes
w = resource("target").write_path; w.write_text(…) # passes — attribute is still on the Call
r = resource("target"); r.write_path.write_text(…) # HARD ERROR — owner is ast.Name, not ast.Call
```

The third form is the natural way to write an in-place `read_write` job, because the same handle is read and then written. The nondeterminism I measured is the model choosing between two correct programs, one of which the checker cannot see through.

The suite does not catch this, and the reason is instructive. `read_write_requires_existing_regular_file_and_commits_replacement` (`src/program.rs:1525`) builds its proposal as `r=resource('document')` then `r.write_path.write_text(...)` — precisely the rejected form — at `src/program.rs:1532`. It stays green because it calls `execute()` directly and never runs `preflight()`. So the project's canonical example of a read-write program is one its own validator would refuse in production, and no test connects the two.

The `dynamic_write` escape at `src/program.rs:155` only downgrades to a warning when `resource(...)` receives a non-literal argument. Binding the handle to a variable sets neither `write_resource_calls` nor `dynamic_write`, so the most ordinary form lands in the hard-error branch rather than the branch that exists precisely for writes that cannot be proven statically.

- Resolve a resource handle bound to a local name before deciding a declared write is missing. Single-assignment local tracking is sufficient for the observed form and stays a static check.
- Any residual case the checker still cannot prove must fall to the existing `dynamic_write` warning at `src/program.rs:155`, not to a hard error. A checker that cannot see a write should say so and continue, not refuse a valid program.
- Close the gap that hid this: at least one test must drive a read-write program through `preflight()` and not only `execute()`. The existing fixture at `src/program.rs:1525` becomes that test once its style is validated rather than assumed.
- Then measure: run the in-place single-file rewrite class across `tests/program_corpus.rs`, `evaluation/`, and `benchmark/`, recording rejection rate per reason code. If a materially worse rate survives this fix, the residue belongs to Plan 12's contract work, not here.

**PRG-2 — validator vocabulary reaches the terminal unmapped.** `src/command.rs:564` prints `Program contract error: {message}` and `src/command.rs:2143` prints `uhm: {message}`, both straight from the diagnostic. Warnings are worse: `src/command.rs:543` prints the internal code alongside the text. The only consumer that ever adds context is the model-facing repair payload at `src/command.rs:1726`; no code maps a reason to human remediation anywhere, and history keeps only code and severity (`src/history.rs:163`).

- Map each reason code to a user-facing sentence naming what was wrong and what to do. Keep the code under `-v` and in history, where it belongs.
- Do not print an internal code in the default warning path.

**EFF-1 — the `Effects:` line renders a union as if it were observation.** Counting rows in a CSV — a shell-route `python3 -c '…'` command — showed `Effects: writes local data, reads local data`, while the receipt for that run held `declared_effects:["read_local","write_local"]` and `detected_effects:["read_local"]`. Both routes union the two sets through `merged_effects` (`src/command.rs:1973`), a plain dedup-append: the shell route as `merged_effects(&classification.effects, &metadata.effects)` at `src/command.rs:1151`, the program route as `merged_effects(&proposal.effects, &detected)` at `src/command.rs:522`. `src/render/card.rs:13` then prints the result. A `write_local` present only in the model's declaration renders identically to one local classification actually found. The two sets are kept apart on the receipt at `src/command.rs:1942` and nothing ever compares them; telemetry concatenates them too (`src/telemetry.rs:697`).

The detector is a substring scan over lowered source (`src/program.rs:310`) whose `WriteLocal` needles include `'w'` and `"w"` (`:325`–`:335`). It is deliberately crude, so it will disagree with a declaration in both directions. That is a reason to show the disagreement, not to hide it inside a union.

- Keep showing the union — under-reporting an effect is the dangerous direction — but distinguish a declared-only effect from a detected one in the rendering, so the line stops implying observation it does not have.
- Do not gate execution on a mismatch. Detection is advisory by settled decision, and over-declaration is the safe error.
- Telemetry stays untouched. The event schema is a fixed set of allowlisted enum fields — `src/telemetry.rs:781` asserts an event contains nothing else — so even a coarse disagreement counter is a new field, and this plan claims no telemetry schema change. Whether over-declaration is systemic is worth measuring, but that field belongs to a plan that changes the schema deliberately.

**EFF-2 — the program route never shows effects at all.** `program_preview` (`src/command.rs:1982`) prints Runtime, Contract, Resources, Recovery, and Limits, but no `Effects:` line, even though `src/command.rs:521` has already computed the merged set. The route that runs generated Python — the one with the broadest reach — is the one that does not tell the user what it will touch. Render it, using the same distinction EFF-1 introduces.

## 8. Tests and validation

Required focused tests, each of which must fail before its fix:

- `input.rs`: a non-TTY stdin held open with no bytes returns an empty spool within the deadline and the unconditional stderr notice is asserted, not just the spool; a producer whose first byte arrives inside the deadline and then streams is read to completion without truncation; a producer whose first byte arrives after the deadline is pinned as the accepted false-negative — empty spool plus notice — so the trade-off is a tested decision rather than an accident; the byte-cap error is unchanged. An end-to-end test asserting that `(sleep N | uhm <intent>)` completes in well under N seconds is the regression that matters, since that is the observed failure.
- Review keys, via `reviewed_with_keystrokes` (`tests/cli_contract.rs:68` — note the harness must hold the writing end open, because `tty::read_line_cooked` reads `/dev/tty`, not stdin): `y` and `yes` execute; `n`, `no`, and an unknown key re-prompt once and then cancel; EOF cancels without execution; each assertion checks the target file, not only the message.
- Recovery preconditions: with `recovery.enabled: false`, `undo`/`restore`/`recover` name the setting and return 13; with recovery enabled and no manifest, the message is the distinct manifest-absent one.
- `last` resolution: a restorable manifest plus a newer restored manifest resolves to the restorable one and names the choice; a run whose only manifest is non-restorable still reports the state clearly.
- `prune`: `--all` removes retained in-cap snapshots and plain `prune` does not, with `recovery status` agreeing with what was removed in both cases; `--dry-run --all` removes nothing.
- `--recoverable` on a shell-route and parent-shell action renders the existing reason string and, without `--force`, stops before execution.
- `program.rs` AST checker: all three resource-write forms in section 7 pass, including `r = resource("target")` followed by `r.write_path.write_text(...)`; a genuinely unwritten declared resource still hard-errors; an unprovable write produces the `dynamic_write` warning rather than a hard error.
- No default-path stderr line contains a validator reason code; every hard-error reason maps to a mapped sentence, asserted per code so a new code cannot ship unmapped.
- The proposal block distinguishes a declared-only effect from a detected one, and the program route renders an `Effects:` line at all.
- `history list` prints outcome and a human timestamp; `history show` renders events by default and emits raw JSONL under `--json`; neither prints a field the allowlisted export withholds.
- `--dry-run` and review `copy`: exact bytes with no terminator when stdout is not a tty, one trailing newline when it is.
- Documentation: every fenced `uhm` example in `README.md` and `docs/` runs against a stub `uhm` under both `zsh -c` and `bash -c` without expansion errors, per section 6.

Full verification is the existing quality gate — `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, and the `tests/cli_contract.rs` pty suite on macOS and Linux. Section 1 changes a blocking read, so the CLI contract suite must be run with stdin variously a tty, a closed pipe, an open idle pipe, and a streaming pipe.

## Delivery sequence

1. Sections 1 and 2. The silent hang and the cancelling `y` are the two defects that cost a user real work, and neither touches recovery, history, or documentation. Independently shippable as a patch release.
2. Section 3, REC-1 through REC-3, REC-5, and REC-4's `Recovery:` line. Message and resolution changes only; no state-machine change. REC-4's stop-without-`--force` is the one behavior change in this plan and rides the next minor release, per section 3.
3. Section 7 PRG-1. It is a self-contained static-analysis fix with a clear reproducer, and it removes a two-in-three failure rate from the most common program-route mutation. It can proceed in parallel with step 2; only the corpus measurement that follows it needs a benchmark slot.
4. Sections 4, 5, and 6. Rendering and documentation. Section 6's example audit should land with whatever release changes the README.
5. Section 7 PRG-2, EFF-1, and EFF-2 together — one pass over proposal-block rendering.

## Completion criteria

- No invocation of `uhm` waits on stdin that will never arrive, and no legitimate piped producer is truncated.
- The most likely affirmative keystroke at a confirmation prompt executes the reviewed command; no advertised or unrecognized key silently cancels; EOF still cancels.
- Every command in `--help` either works on a default installation or, on its first invocation, names the exact setting or flag that makes it work.
- `undo`, `restore`, and `recover` distinguish "capture was never enabled" from "this run has no manifest", and return a status that reflects the difference.
- `undo last` never silently operates on an older run than the one the user meant.
- Every message that tells a user to run a command names a command that can do the thing.
- `--recoverable` never reports success having captured nothing.
- `uhm history list` and `uhm history show` are sufficient to choose a run id and see what happened, without retaining intents by default.
- No documented `uhm` example fails in the shell the install instructions target.
- The documented way to ask about a file's content works as copied, and the bare comprehension form is a measured corpus class rather than an unrecorded wrong answer.
- An in-place single-file rewrite is not rejected for binding its resource handle to a variable, and a write the checker cannot prove produces a warning rather than a refusal.
- No internal validator vocabulary or reason code reaches the terminal as the whole of a default-path message.
- Every route that executes something renders its effects, and a declared-only effect is visibly distinct from a detected one.
- No default changes, no new outbound field, no new telemetry content, no additional model call, and no change to the conversation boundary.

## Non-goals

- Enabling recovery capture or full history retention by default.
- Widening the managed-file recovery class to shell-route effects.
- A REPL, a chat loop, or any cross-job memory.
- Remapping the review keys wholesale, or adding a `--yes` flag that skips review entirely.
- Fixing shell quoting inside the binary; zsh expands before `uhm` runs.
- Changing the program contract schema, its resource model, or its prompt text. Section 7 corrects one static-analysis branch inside the existing checker; contract evolution belongs to Plan 12.
- Making a bare comprehension question about a named file resolve in-binary. That requires revising the routing rules in `DEVELOPER_INSTRUCTIONS`, which changes the outbound context; RTE-1 documents the working stdin form here and defers the prompt revision to Plan 13.
