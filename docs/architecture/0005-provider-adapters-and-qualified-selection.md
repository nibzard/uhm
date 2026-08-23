<!-- diataxis: explanation -->

# ADR 0005: Provider adapters and qualified selection

- Status: accepted
- Date: 2026-08-03
- Supersedes: the OpenAI-only transport portion of [ADR 0002](../architecture/0002-responses-result-loop.md)

## Context

ADR 0002 intentionally coupled the bounded result loop to one OpenAI Responses transport. Supporting another provider without weakening action validation requires a provider-neutral acceptance boundary, fixed endpoints, and evidence that is tied to the exact production contract. A model name alone is not a provider identity or a qualification claim.

## Decision

OpenAI Responses, Cerebras Chat Completions, and DeepSeek Responses are fixed built-in adapters behind one provider interface. Each adapter owns only its wire request and response parsing. Every returned action passes through the same canonical local decoder, schema, semantic validation, runtime preflight, and bounded result loop before it can be accepted.

Provider and model are selected independently. Fixed mode permits an explicit provider/model pair and an optional alternate for a typed allowlist of pre-proposal failures. Fallback is sequential and shares the global two-call ceiling. Authentication failure, missing credentials, and policy rejection never trigger fallback. Arbitrary compatible endpoints are not supported.

Evidence mode trusts only reviewed entries in the checked-in qualification manifest. An entry must match the fixed endpoint, stable provider-returned model identity, request class, permitted actions, and frozen prompt, schema, context, adapter, selection-policy, corpus, runner, and evaluation fingerprints. Qualification uses a sealed holdout and independent audit; development benchmarks cannot authorize runtime selection. With the shipped empty manifest and unavailable holdout commitment, evidence mode fails closed.

## Consequences

Provider wire differences cannot relax the product action contract. Adding a provider requires a new fixed adapter, privacy disclosure, canonical conformance coverage, and qualification evidence rather than a configurable base URL. Explicit fixed use can precede qualification, but the CLI reports that status and never presents it as evidence-selected.

The three adapters have different provider-side retention behavior. Documentation names the selected endpoint set before outbound work and defers provider-side retention claims to each provider's current terms.
