# Product

## Register

product

## Users

`uhm` is for individual Linux and macOS terminal users who know the result they want but do not reliably remember the command, flag, tool, or short program that will produce it. Its primary audience spans capable beginners attempting more advanced terminal work and experienced users whose command recall has atrophied after extensive use of coding agents. They are usually focused or rushed and value speed, convenience, power, and flexibility more than instruction for its own sake.

## Product Purpose

`uhm` is a fast, open-source, result-first interface between natural language and local terminal work. It translates an intent—typed normally or entered through voice dictation—into one bounded job, executes the proposed action under the user's authority, and returns the useful result. A job may spend one global second turn on a clarification or a user-triggered replacement action, but it does not plan projects, chat indefinitely, run in the background, or become a coding agent.

Success means a user can go from “I know what I need” to a correct local result without recalling a particular tool, flag, shell dialect, or programming language, and without leaving the terminal.

## Brand Personality

Quick, playful, capable. `uhm` should feel upbeat, modern, and slightly quirky in the spirit of the best Charm tools, while remaining literal and calm around errors, elevated privileges, deletion, and other consequential actions. Personality belongs in pacing, copy, color, and small moments of delight—not in extra ceremony.

## Anti-references

- A general-purpose coding agent that explores a repository, creates plans, edits projects, or keeps working autonomously.
- A chatbot or long-running assistant waiting for conversation.
- A background job daemon, autonomous task queue, or terminal surveillance layer.
- A dry, bureaucratic Unix interface whose ceremony becomes slower than remembering the command.
- “AI magic” that hides what ran, claims safety it cannot prove, or presents model confidence as fact.
- A sandbox or rollback product making guarantees the implementation cannot uphold.

## Design Principles

1. **Return the result, not the incantation.** The command or microprogram is an implementation detail unless the user asks to review it or the action deserves a warning.
2. **Fast enough to become a reflex.** Ordinary work should take one invocation, with negligible local overhead and no unnecessary confirmation.
3. **One intent, one bounded job.** Start with one proposal; allow at most one clarification, model revision, or post-execution replacement. A local edit before first execution still belongs to the initial proposal. Autonomous loops, broad plans, and open-ended conversation are not.
4. **Power without pretending.** Trust the model enough to be useful, disclose material effects and assumptions, and never describe a heuristic as a safety boundary.
5. **Context should earn its trip.** Send bounded terminal context that materially improves the result, disclose it on first use, and make prompt-only or fuller modes easy to select.
6. **The terminal is the interface.** Preserve exact bytes, exit codes, pipes, narrow terminals, SSH, tmux, accessibility modes, and parent-shell semantics.
7. **Measure use without collecting terminal work.** Coarse telemetry is on by default after a concise notice and is immediately opt-out; it never contains prompts, commands, paths, input, output, credentials, or stable identifiers.
8. **Keep a local receipt, not cloud memory.** Decisions and outcomes should be inspectable on the user's device under explicit retention/detail controls and must never become hidden context for unrelated jobs.

## Accessibility & Inclusion

The product must remain usable without color, animation, Unicode decoration, raw-mode editing, or a wide terminal. Important state is never encoded by color alone. A plain/cooked mode must work with screen readers and `TERM=dumb`; output intended for pipes must contain no terminal control sequences. Text and controls must remain understandable at narrow widths, and behavior should be tested with Unicode input, SSH, and tmux.
