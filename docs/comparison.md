<!-- diataxis: explanation -->

# How uhm compares

`uhm` takes one natural-language intent, picks one bounded shell action or a generated Python microprogram, runs it, prints the real result, and exits. It is deliberately smaller than a coding agent and deliberately quieter than a chatbot. One intent goes in. One bounded job comes out. Then `uhm` exits.

That places it in a narrow band most "AI terminal" tools do not occupy. This page is a point-in-time public scan (August 2026) — comprehensive, but not mathematically exhaustive. Small natural-language shell wrappers appear on GitHub often; what follows is the set worth reasoning about.

## What counts as a competitor

Three tiers, ordered by how directly each overlaps:

| Tier | What it does | Overlap with `uhm` |
|---|---|---|
| Direct alternatives | Natural language → terminal command or action | The same job: turn words into a run, then stop |
| Broader terminal and coding agents | Multi-step loops that inspect, edit, and execute | Compete for the same terminal-AI usage and budget, not a like-for-like replacement |
| Non-AI substitutes | Command discovery, history, correction | Often the better answer when the real problem is recall, not expression |

`uhm` is result-first rather than command-first, and one-job-and-exit rather than an agent. The command or microprogram is an implementation detail; what you receive is the result, and you can inspect the implementation when necessary. That framing governs every comparison below.

If you already know what you are looking for:

| If you want... | Consider |
|---|---|
| The finished result, not the command | `uhm` |
| To see and edit the command before it runs | [llm-cmd](https://github.com/simonw/llm-cmd), hai, [uwu](https://github.com/context-labs/uwu) |
| A mature, broad command assistant | [ShellGPT](https://github.com/TheR1D/shell_gpt), [AIChat](https://github.com/sigoden/aichat) |
| Local-first or private inference | [cmd-ai](https://github.com/BrodaNoel/cmd-ai), [cmdh](https://github.com/pgibler/cmdh), [osh](https://github.com/charyan/osh), Spren |
| Shell-native, hotkey-driven insertion | [Termax](https://github.com/huangyz0918/termax), [hi-shell](https://github.com/longyijdos/hi-shell), [whai](https://github.com/gael-vanderlee/whai), [clai](https://github.com/domzilla/clai) |
| Safe confirm-then-execute with risk signals | [llm-term](https://github.com/dh1011/llm-term), [cmd-ai](https://github.com/BrodaNoel/cmd-ai), Spren, [nlsh](https://github.com/abakermi/nlsh) |
| Several suggested commands to choose from | [Shell-AI (shai)](https://github.com/ricklamers/shell-ai) |
| An autonomous multi-step agent | [Warp](https://www.warp.dev/ai), [GitHub Copilot CLI](https://github.com/features/copilot/cli), [Claude Code](https://github.com/anthropics/claude-code), [OpenAI Codex CLI](https://github.com/openai/codex), [Gemini CLI](https://github.com/google-gemini/gemini-cli), [OpenCode](https://github.com/sst/opencode) |
| To remember or correct commands without an LLM | [Atuin](https://atuin.sh), [navi](https://github.com/denisidoro/navi), [The Fuck](https://github.com/nvbn/thefuck), [tldr pages](https://github.com/tldr-pages/tldr) |

Names link to verified sources. hai and Spren are listed plain where no canonical source was confirmed.

## Closest direct alternative: llm-cmd

[llm-cmd](https://github.com/simonw/llm-cmd) is the closest command-first minimalist competitor. The workflow is `llm cmd undo last git commit` — the plugin asks the configured LLM for a shell command, drops that command into an editable prompt, and you press Enter to execute or Ctrl+C to cancel. It takes `-m` for a model and `-s` for a custom system prompt, and draws on the broader LLM CLI plugin ecosystem for cloud and local models.

The cleanest way to state the difference:

- llm-cmd: describe the command you need, inspect it, then execute it.
- `uhm`: describe the result you need, receive that result, and inspect the implementation when necessary.

| Dimension | llm-cmd | `uhm` |
|---|---|---|
| Core model | Natural language → editable shell command | Natural language → typed action → useful result |
| Default approval | Always exposes the command for editing before execution | Ordinary actions run immediately; review can be requested or triggered for consequential actions |
| Primary interface | Command-first | Result-first |
| Execution types | Shell commands only | Shell commands, non-executing answers, or bounded Python microprograms |
| Model selection | Inherits the cloud and local model ecosystem of the LLM CLI | Fixed OpenAI or Cerebras adapters; automatic selection requires reviewed qualification evidence |
| Context | Joins the command-line words into a prompt | Bounded OS, shell, working-directory, Git and installed-tool context |
| Piped input | No dedicated stdin-data workflow | Explicit piped input, including a local-only input mode |
| Failure handling | Prints the command's captured error output | One bounded repair attempt in interactive use |
| History | Its custom command currently bypasses LLM's normal SQLite logging | Local metadata receipts, replay, search and optional detailed records |
| Recovery | None documented | Optional bounded file recovery for managed Python artifacts |
| Output handling | Captures output, decodes it and prints it after completion | Designed to preserve result bytes, exit codes and pipeability |

The core is roughly 49 lines: it calls `subprocess.check_output(..., shell=True)` with no visible timeout or command-risk classifier, and it relies principally on mandatory human review. Simon introduced it on March 26, 2024 as an alpha and "very dangerous." That warning is still in the repository, so it reads as a deliberately tiny experimental utility rather than an execution framework.

Where llm-cmd wins: an extremely small and understandable implementation; mandatory edit-before-execute as a strong, simple safety mechanism; excellent model flexibility through the larger ecosystem, including local-model plugins; a customizable system prompt; an easy install for existing LLM users.

Where `uhm` wins: it returns the result rather than centering the command; less confirmation friction for routine read-only work; structured Python processing, not just shell generation; better handling of stdin, private piped data and pipeable output; bounded clarification and repair; effect warnings, local receipts, replay and limited recovery; more explicit runtime, privacy and terminal-behavior contracts.

## Other close direct alternatives

The next nine, in rough order of overlap with `uhm`.

| Tool | Shape | How it differs from `uhm` |
|---|---|---|
| hai | Suggest, confirm, execute; supports piped input | Closest minimalist Unix-style interaction; produces shell commands rather than `uhm`'s result-first shell or Python jobs |
| [cmd-ai](https://github.com/BrodaNoel/cmd-ai) | Show, confirm, execute | Strong local-first option — Ollama by default, with OpenAI, Gemini, Claude; includes safety, history, completion; command-focused, confirmation-dependent |
| [ShellGPT](https://github.com/TheR1D/shell_gpt) | Generate commands, code, explanations; optional execution | Broadest established assistant — Bash, Zsh, PowerShell, CMD across Linux, macOS, Windows; more general copilot, less deliberately bounded |
| [llm-term](https://github.com/dh1011/llm-term) | Generate, display, confirm with `y`, execute | Small Rust tool with explicit execution safety and a portable Windows build; shell-command generation only, no documented Python path |
| [uwu](https://github.com/context-labs/uwu) | Generate a command, place it into an editable line | Focused and low-friction; deliberately rejects agent complexity; you edit and run rather than receive a result |
| [Termax](https://github.com/huangyz0918/termax) | Ask, generate, optionally auto-execute; hotkey-driven | Broad model support, Bash/Zsh/Fish plugins, Windows, RAG from past commands; collects more workspace context, behaves more like a personalized assistant |
| Spren | Preview and safely execute | Cross-platform Bash, PowerShell, CMD with dangerous-command detection and error analysis; shell translation rather than bounded shell-or-Python execution |
| [llm.fish](https://github.com/avafloww/llm.fish) | Generate, refine, execute, or immediate "yolo" mode; can repair failed commands | Lightweight for Fish users; requires Claude Code and Fish; narrower portability and safety envelope |
| [nlsh](https://github.com/abakermi/nlsh) | Generate and execute with confirmation and command filters | Straightforward, safety-oriented execution loop; conventional NL-to-shell rather than result-first orchestration |

## Wider direct and near-direct alternatives

Grouped by approach. These overlap with `uhm` but did not make the shortlist above.

**Command generation and execution.**

- [AIChat](https://github.com/sigoden/aichat) — broad multi-provider LLM CLI with an OS-aware Shell Assistant; useful far beyond shell commands; conversational, not one-job-and-exit.
- [Shell-AI (shai)](https://github.com/ricklamers/shell-ai) — returns several one-line command suggestions; choice-oriented rather than auto-executing.
- [AI Shell (Builder.io)](https://github.com/BuilderIO/ai-shell) — recognizable NL-to-shell command generator with explanations.
- [LazyShell](https://github.com/bernoussama/lazyshell) — generates and executes commands across multiple providers.
- [gpt-cli (gustawdaniel)](https://github.com/gustawdaniel/gpt-cli) — Rust NL-to-Linux-command, then confirm and execute.
- [AI Shell Command Generator](https://github.com/codingthefuturewithai/ai-shell-command-generator) — OpenAI, Claude or Ollama; color-coded risk assessment; teaching mode.
- [Open Codex (codingmoh)](https://github.com/codingmoh/open-codex) — local Ollama generation, optional execution after confirmation. Unrelated to OpenAI's Codex CLI.
- `VCMD` — Cerebras-powered generation with safety classification and failure correction. No confirmed canonical source.
- Ask AI — Rust CLI with dangerous-command detection and dry runs. No confirmed canonical source.
- [HyperShell](https://github.com/kirabase/hyper-shell) — generation and execution via OpenAI or Claude.
- `ai.go` — AWS Bedrock/Anthropic, with safety checks and command-history context. No confirmed canonical source.
- [doum-cli](https://github.com/junhyungL/doum-cli) — Rust ask/suggest/auto modes, OpenAI or Anthropic.
- [larpshell](https://github.com/uwuclxdy/larpshell) — in-terminal generation, execution, and explanation.
- [Nexterm](https://github.com/DevAdvancer/Nexterm) — Rust terminal emulator with an `ai:` mode, explanations, and error assistance.
- [Console2Ai](https://github.com/MaxITService/Console2Ai) — PowerShell hotkeys that send your prompt plus recent screen history to an AI.
- [Natural Language Shell Interface](https://github.com/Natural-Language-Shell/Natural-Language-Shell) — custom NL shell with real-time execution and optional voice input.
- [Syniq](https://github.com/Vptsh/syniq) — Linux TUI using a free public model; SAFE/RISKY/BLOCKED classification; no API key.
- [PromptShell](https://github.com/Kirti-Rathi/PromptShell) — early-stage, cross-platform, privacy-oriented; cloud and local (Ollama) models.

**Suggestion-first and shell-insertion tools** — these insert or print a command rather than treating the completed result as the primary output.

- [nl-sh](https://github.com/mikecvet/nl-sh) — Rust NL overlay that also takes ordinary shell commands; proof of concept.
- [cmdh](https://github.com/pgibler/cmdh) — Ollama/OpenAI NL command generator.
- [tlm](https://github.com/yusufcanb/tlm) — local Ollama copilot with suggest, explain, and ask modes.
- [ShellOracle](https://github.com/djcopley/ShellOracle) — Ollama, OpenAI, DeepSeek, LocalAI, Grok.
- [Zev](https://github.com/dtnewman/zev) — OpenAI, Gemini, Ollama, Azure OpenAI.
- [DeveloperGPT](https://github.com/luo-anthony/DeveloperGPT) — command generation plus terminal chat; cloud and a local offline model.
- `apesh` — lightweight contextual helper; can repair the previous failed command. No confirmed canonical source.
- [hi-shell](https://github.com/longyijdos/hi-shell) — Zsh ghost-text suggestions with safe/warn/blocked scoring; accept before execution.
- [whai](https://github.com/gael-vanderlee/whai) — keybinding replaces the current line with a generated command.
- [zsh-ai](https://github.com/matheusml/zsh-ai) — minimal Zsh NL plugin; no auto-execution.
- [clai](https://github.com/domzilla/clai) — Bash/Zsh/Fish/PowerShell via a keyboard shortcut; multiple providers.
- [clai (merefield)](https://github.com/merefield/clai) — pure-Bash helper with green/amber/red traffic-light risk indicators.
- [pls](https://github.com/colus001/pls) — multilingual NL-to-shell translator.
- [Pal](https://github.com/scottyeager/Pal) — command suggestions plus an `/ask` question mode.
- [osh](https://github.com/charyan/osh) — local English-to-Unix via Ollama; optional execution.
- [gpt-cli (alex-ello)](https://github.com/alex-ello/gpt-cli) — suggestions and chat for Bash, Zsh, Sh across platforms.
- [reTermAI](https://github.com/pie0902/reTermAI) — grounded in your shell history; OpenAI or Gemini.
- [llm-term (juftin)](https://github.com/juftin/llm-term) — multi-provider terminal chat; a different project from the Rust execution tool above.

**More agentic shell-specific projects** — they reach toward tools, filesystem, network, and task orchestration, sitting between the generators and the full agents.

- [Shell-AI (nishant9083)](https://github.com/nishant9083/shell-ai) — local Ollama ReAct agent with file, shell, web, memory and MCP tools.
- [Luminos](https://github.com/benbaptist/luminos) — NL assistant with filesystem, network and shell tools under permission prompts.
- [RealConsole](https://github.com/hongxin/RealConsole) — Rust "smart shell" with planning and task orchestration, not just command translation.

## Broader terminal and coding agents

These compete for the same terminal-AI usage and budget, but they are not direct replacements. They run in multi-step loops, inspect repositories, modify files, and continue until a larger objective is complete. If your task is genuinely multi-step, these are the better fit; `uhm` is the wrong shape for it.

**Commercial and platform-backed.**

- [Warp](https://www.warp.dev/ai) — plain-English multi-step agent built into Warp.
- [GitHub Copilot CLI](https://github.com/features/copilot/cli) — agentic terminal assistant; reads files, modifies projects, runs commands in trusted directories.
- [Amazon Q Developer CLI](https://github.com/aws/amazon-q-developer-cli) — agentic, chat-driven CLI coding assistant.
- [Claude Code](https://github.com/anthropics/claude-code) — terminal coding agent that understands a codebase, edits files, runs commands.
- [OpenAI Codex CLI](https://github.com/openai/codex) — local coding agent; inspects, changes, executes code.
- [Cursor CLI](https://cursor.com/cli) — terminal and headless Cursor agent for shell, scripts, CI/CD.
- [Amp](https://ampcode.com/) — Sourcegraph coding agent in terminal and editors.
- [Factory Droid CLI](https://docs.factory.ai/droid-cli/quickstart) — terminal interface for Factory's autonomous Droids.

**Open-source or provider-flexible.**

- [Gemini CLI](https://github.com/google-gemini/gemini-cli) — open-source ReAct-style agent; built-in tools and MCP.
- [OpenCode](https://github.com/sst/opencode) — open-source terminal coding agent; multiple providers, persistent sessions.
- [Qwen Code](https://github.com/QwenLM/qwen-code) — open-source terminal agent; multiple providers, local models, headless, MCP, sandboxing.
- [Kimi CLI](https://github.com/MoonshotAI/kimi-cli) — open-source agent; planning, code edits, shell, web.
- [Crush](https://github.com/charmbracelet/crush) — open-source agentic coding TUI; many providers, mid-session model switching.
- [Goose](https://github.com/block/goose) — Block's open-source local agent; desktop, CLI, and API.
- [Aider](https://github.com/Aider-AI/aider) — terminal pair programmer; many cloud and local models.
- [Plandex](https://github.com/plandex-ai/plandex) — plans and executes long multi-step coding tasks.
- [Open Interpreter](https://github.com/OpenInterpreter/open-interpreter) — NL interface to run code and shell locally.
- [Microsoft Intelligent Terminal](https://github.com/microsoft/intelligent-terminal) — Windows Terminal fork with native agent integration over ACP.

## Non-AI substitutes

Often the better choice when the real problem is remembering or recovering a command rather than expressing an arbitrary task in natural language. None of these need a model, a network call, or a credit balance.

- [Atuin](https://atuin.sh) — searchable, optionally synced, end-to-end-encrypted shell history.
- [navi](https://github.com/denisidoro/navi) — interactive cheatsheets with executable examples.
- [The Fuck](https://github.com/nvbn/thefuck) — corrects the previous failed or mistyped command against built-in rules.
- [tldr pages](https://github.com/tldr-pages/tldr) — concise, community-maintained examples for command-line tools.

## Which to try first

For benchmarking, install the sourceable shortlist first: [llm-cmd](https://github.com/simonw/llm-cmd), [ShellGPT](https://github.com/TheR1D/shell_gpt), [cmd-ai](https://github.com/BrodaNoel/cmd-ai), [llm-term](https://github.com/dh1011/llm-term), [uwu](https://github.com/context-labs/uwu), [llm.fish](https://github.com/avafloww/llm.fish), [Termax](https://github.com/huangyz0918/termax), [nlsh](https://github.com/abakermi/nlsh). By dimension:

- Minimalism — llm-cmd, hai, uwu.
- Command-assistant maturity and breadth — ShellGPT, AIChat.
- Local or private inference — cmd-ai, cmdh, osh, Spren.
- Shell-native, hotkey-driven interaction — Termax, hi-shell, whai, clai.
- Safe, confirmation-based execution — llm-term, cmd-ai, Spren, nlsh.
- Automatic or result-oriented execution — llm.fish, Termax, and several smaller executors.
- Full-agent displacement — Warp, GitHub Copilot CLI, Claude Code, OpenAI Codex CLI, Gemini CLI, OpenCode.

No surveyed alternative documents the same complete combination as `uhm`: the actual result as the default output rather than a proposed command; a strict one-intent, one-bounded-job lifecycle; a choice between shell actions and generated Python microprograms; explicit handling of piped-data privacy; bounded clarification and repair rather than open-ended loops; and local receipts, review and bounded recovery. Several tools cover any one of these; none cover the set.

## A point-in-time note

This page is a public scan dated August 2026. It is comprehensive but not mathematically exhaustive — small natural-language shell wrappers appear on GitHub frequently, and existing tools change features, licensing, and reach between scans. Treat the tables as a starting set, not a census. Where a tool is described as "command-first" or "confirm-first," confirm against its current repository before relying on the distinction.

## Next

- [Getting started](getting-started.md) — install `uhm` and run your first intent.
- [Core concepts](concepts.md) — the one-intent, one-bounded-job lifecycle.
- [Program execution model](explanation/program-execution.md) — when and why `uhm` generates a program instead of a command.
- [Behavior & exit codes](behavior-contract.md) — the approval, review, and exit-status contract.
- [Privacy & telemetry](privacy.md) — what leaves your machine, and the local-only input mode.
