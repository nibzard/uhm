<!-- diataxis: how-to -->

# Configure a fallback provider

Use fallback only when you want one disclosed alternate provider/model for specific pre-proposal failures. It is off by default.

## 1. Configure both credentials

Set `OPENAI_API_KEY` and `CEREBRAS_API_KEY` in the environment or private secrets file. Missing alternate credentials do not trigger or defer fallback; the request fails closed.

## 2. Add an alternate and allowlist

```yaml
provider: openai
model: gpt-5.6-terra

selection:
  mode: fixed
  alternate:
    provider: cerebras
    model: gpt-oss-120b
  fallback_on:
    - rate_limited
    - transient
    - timeout
```

Allowed values are `rate_limited`, `transient`, `timeout`, `incomplete`, and `malformed`.

## 3. Validate the configuration

```sh
uhm config check
uhm config show
uhm doctor all
uhm doctor all network
```

Changing the authorized endpoint set causes the first-use disclosure to appear again before outbound work.

## Operational limits

Fallback is sequential and can occur only before a proposal is accepted. It consumes the second and final provider-call slot, leaving no call for clarification or repair. Authentication failures, missing credentials, and policy rejection never trigger it. Fallback is a transport-availability mechanism, not a runtime quality comparison.

See the [provider reference](../reference/providers.md) for the exact matrix and [model-selection explanation](../explanation/model-selection.md) for why fallback and qualification are separate.
