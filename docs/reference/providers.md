<!-- diataxis: reference -->

# Provider and model reference

`uhm` supports three fixed built-in provider adapters. Arbitrary base URLs and provider inference from model names are rejected.

| Provider | ID | Endpoint | API family | Credential | Streaming | Reasoning effort | Strict bounds on wire |
|---|---|---|---|---|---:|---:|---:|
| OpenAI | `openai` | `https://api.openai.com/v1/responses` | Responses | `OPENAI_API_KEY` | yes | yes | yes |
| Cerebras | `cerebras` | `https://api.cerebras.ai/v1/chat/completions` | Chat Completions | `CEREBRAS_API_KEY` | no | no | adapted locally |
| DeepSeek | `deepseek` | `https://api.deepseek.com/v1/responses` | Responses | `DEEPSEEK_API_KEY` | yes | yes | yes |

All three adapters require exactly one strict tool call and reject plain prose, refusals, incomplete responses, unknown tools, and multiple choices or calls. Every accepted wire result passes through the same canonical local action decoder and semantic validator.

For Cerebras, the wire adapter removes unsupported `maxLength`, `maxItems`, and `pattern` keywords from tool schemas. The complete canonical bounds are still enforced locally before an action is accepted.

DeepSeek speaks the same Responses format as OpenAI and echoes the resolved strict tool set, so no schema adaptation is needed. `deepseek-v4-flash` runs in thinking mode, which rejects a forced tool choice, so the adapter sends `tool_choice: "auto"`; the one-call requirement is still enforced locally when the response is parsed.

## Resolution

Provider and model resolve independently in this order:

1. built-in defaults;
2. `config.yaml`;
3. `UHM_PROVIDER` and `UHM_MODEL`;
4. `--provider` and `--model` / `-m`.

`OPENAI_MODEL` and `DEEPSEEK_MODEL` are compatibility aliases only when the matching provider is selected and `UHM_MODEL` is absent.

## Selection modes

| Mode | Behavior |
|---|---|
| `fixed` | Use the resolved explicit provider/model pair; optionally try one configured alternate for an allowed pre-proposal failure |
| `evidence` | Resolve only from exact reviewed qualification-manifest evidence; otherwise return unavailable |

The v0.5.0 checked-in manifest has no entries, so evidence mode currently selects no pair.

## Fallback error classes

Configurable triggers are `rate_limited`, `transient`, `timeout`, `incomplete`, and `malformed`. Credential, authentication, request rejection, refusal, unsupported capability, and policy failures are not configurable fallback triggers.

Fallback can occur only before an accepted proposal, is sequential, and consumes the global second provider-call slot.

## Diagnostics

```sh
uhm doctor
uhm doctor network
uhm doctor all
uhm doctor all network
```

`network` checks the selected provider. `all` reports all three adapters; `all network` includes reachability and authentication for each. Doctor and live provider calls use the same proxy resolver, native/custom trust configuration, TLS client, and error classifier. A doctor request uses the provider's models route to avoid a billable generation.

Transport failures are reported by stage: trust configuration, proxy configuration, proxy/CONNECT, DNS, TCP, TLS certificate, TLS handshake, or HTTP. Live JSON errors use distinct `trust`, `proxy`, `dns`, `tls`, and `network` error kinds; none are eligible fallback triggers by default.
