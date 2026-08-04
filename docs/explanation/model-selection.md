<!-- diataxis: explanation -->

# Why model selection is evidence-gated

Provider selection looks like a configuration problem, but automatic selection is a product-behavior decision. A candidate must do more than return valid JSON: it must preserve UHM's action contract, choose appropriate routes, survive runtime preflight, and complete representative work without expanding authority.

## Fixed selection and qualification answer different questions

Fixed mode answers, “Which provider and model did the operator explicitly choose?” It is allowed even when no qualification evidence exists because the choice is direct and inspectable.

Evidence mode answers, “Which candidate has reviewed evidence for this exact request class and contract?” It therefore fails closed when evidence is missing or stale. The shipped v0.3.5 manifest is empty, so no pair is currently selected automatically.

## Model names are not identities

A model alias can move without a client release. Qualification binds the provider, fixed endpoint, requested model, stable provider-returned identity, request class, permitted action types, and fingerprints for the prompt, schemas, context policy, adapter, selection policy, corpus, runner, and evaluation rules. A changed component invalidates the old conclusion rather than silently inheriting it.

## Fallback is not quality selection

Fallback handles a narrow set of pre-proposal availability failures. It is sequential, explicitly configured, and consumes the only second provider-call slot. It never compares two proposals or promotes an alternate because it “looks better.” Authentication and policy failures stop immediately.

Keeping fallback mechanical prevents an availability feature from becoming an unreviewed runtime evaluator.

## Development evidence is not release evidence

The development corpus helps improve adapters, prompts, validators, and benchmark mechanics. Once those artifacts have influenced development, they cannot provide untouched confirmation of the same system. Qualification therefore uses a separately authored sealed holdout, frozen policy, blinded judging, and independent audit.

The cost is deliberate friction. The benefit is that an automatic-selection claim remains reproducible and tied to the behavior users actually run.

For procedures, see [Run the provider benchmark](../how-to/run-provider-benchmark.md) and the [qualification runbook](../qualification.md). For exact runtime behavior, see the [provider reference](../reference/providers.md).
