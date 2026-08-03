<!-- diataxis: navigation -->

# Model selection

The v0.3.0 default is the explicit fixed pair `openai:gpt-5.6-terra`. No provider/model pair is currently qualified for automatic selection because the shipped qualification manifest and holdout commitment are intentionally unavailable.

Choose the page that matches what you need:

- [Configure a provider](how-to/configure-providers.md) — select OpenAI or Cerebras explicitly.
- [Configure a fallback provider](how-to/configure-fallback.md) — opt into one alternate for typed pre-proposal failures.
- [Provider and model reference](reference/providers.md) — look up endpoints, capabilities, precedence, modes, and fallback classes.
- [Provider benchmark reference](reference/benchmark.md) — look up corpus structure, worker isolation, metrics, artifacts, and historical evidence.
- [Why model selection is evidence-gated](explanation/model-selection.md) — understand fixed selection, identity binding, fallback, and qualification.
- [Run the provider benchmark](how-to/run-provider-benchmark.md) — execute the development corpus.
- [Provider qualification runbook](qualification.md) — produce reviewed evidence from a sealed holdout.

Historical v0.1 default-selection results remain recorded in [the v0.1 release candidate report](rc-v0.1.0.md). They are historical release evidence, not the current qualification mechanism.
