<!-- diataxis: how-to -->

# Configure a provider

Use this guide to configure OpenAI or Cerebras as a fixed provider/model pair. Explicit fixed use is supported independently of evidence qualification.

## Use OpenAI

OpenAI is the default. Set its key and verify the selected endpoint:

```sh
export OPENAI_API_KEY="sk-..."
uhm doctor network
```

For persistent explicit configuration:

```yaml
provider: openai
model: gpt-5.6-terra
```

OpenAI requests use the fixed Responses endpoint with `store: false`.

## Use Cerebras

Set the Cerebras key and choose both provider and model:

```sh
export CEREBRAS_API_KEY="csk-..."
uhm --provider cerebras --model gpt-oss-120b doctor network
```

For persistent configuration:

```yaml
provider: cerebras
model: gpt-oss-120b
```

Cerebras requests use its fixed Chat Completions endpoint. A model name never implies a provider.

## Store keys in the private secrets file

`uhm doctor` prints the validated secrets path. Create it with mode `0600`, then edit it with a private editor:

```sh
install -m 600 /dev/null "$(uhm doctor 2>/dev/null | grep -o '/[^ ]*secrets[^ ]*')"
```

The file may contain either or both assignments:

```text
OPENAI_API_KEY=sk-...
CEREBRAS_API_KEY=csk-...
```

Environment variables take precedence over the matching file assignment.

## Override one invocation

```sh
UHM_PROVIDER=cerebras UHM_MODEL=gpt-oss-120b uhm ask "show a jq example"
uhm --provider openai --model gpt-5.6-terra ask "show a jq example"
```

Run `uhm config show` to inspect each resolved value and its source. See the [provider reference](../reference/providers.md) for endpoints and capabilities.
