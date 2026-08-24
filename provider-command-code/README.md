# provider-command-code

Command Code models behind
[llm-router](https://github.com/iii-hq/workers/tree/main/llm-router). Install
this worker next to the router, provide one Command Code API key, and its live
model catalog appears behind `router::chat` and `router::complete`. Models are
namespaced as `command-code/<upstream-id>` so they cannot collide with direct
vendor providers.

The upstream model determines the wire protocol. Claude model ids use Command
Code's native Anthropic Messages endpoint; every other model uses its OpenAI
Chat Completions endpoint. The router invokes the provider functions
worker-to-worker, and `iii-permissions.yaml` blocks agents from calling them
directly.

## Install

```bash
iii worker add provider-command-code
iii worker add llm-router
```

`iii worker add` fetches each binary, writes its engine config block, and the
engine starts the workers on its next boot.

## Quickstart

Configure a credential in the `command-code` slice of the engine's
`llm-router` configuration entry, or set `COMMAND_CODE_API_KEY` in the worker
environment:

```json
{ "providers": { "command-code": { "api_key": "cmd_…" } } }
```

The provider discovers the live catalog after registration. Then call the
router with a namespaced model id:

```ts
const result = await iii.trigger('router::complete', {
  model: 'command-code/gpt-5.4',
  messages: [
    {
      role: 'user',
      content: [{ type: 'text', text: 'Explain this function.' }],
      timestamp: Date.now(),
    },
  ],
}, { timeout_ms: 320_000 });
```

For token-by-token output, call `router::chat` with an iii channel as described
in [llm-router's Quickstart](https://github.com/iii-hq/workers/blob/main/llm-router/README.md).

## Configuration

All provider configuration lives in the router's `llm-router` entry:

```jsonc
"command-code": {
  "api_key": "cmd_…",                                      // or COMMAND_CODE_API_KEY
  "api_url": "https://api.commandcode.ai/provider/v1",    // base or endpoint URL
  "max_tokens": 8192                                       // request default
}
```

Worker environment variables:

| Variable | Default | Meaning |
|---|---|---|
| `COMMAND_CODE_API_KEY` | unset | Credential fallback declared to `llm-router` |
| `CMD_ZDR` | disabled | `1` or `true` requires a zero-data-retention upstream; the API fails closed when none is available |
| `PROVIDER_READ_TIMEOUT_SECS` | `120` | Upstream HTTP read timeout |
| `III_URL` | `ws://127.0.0.1:49134` | Engine WebSocket when `--url` is not set |

An invalid `CMD_ZDR` value is rejected instead of silently disabling ZDR. The
binary also accepts the standard `--url`, `--manifest`, and `--config` flags;
provider configuration comes from `llm-router`, so `--config` is ignored with
a warning.

## Models and accounting

`GET /models` is the source of truth. The worker maps the model id, display
name, and context length reported by Command Code. Pricing and feature
capabilities remain absent. Because the router model type requires an output
ceiling while the listing does not publish one, the record uses the provider's
operational default of 8192 tokens. A transient, malformed, or empty refresh
preserves the last known good catalog; missing or explicitly invalid
credentials clear the provider slice.

The provider forwards usage only when the selected endpoint reports it. Cache
reads are kept disjoint from uncached input tokens, and absent usage or cost is
left absent. There is no `count_tokens` function because this multi-vendor API
does not expose a provider tokenizer endpoint.

`thinking_level` is reported as ignored because the catalog does not advertise
a portable reasoning capability. Chat Completions requests forward native
structured-output settings; Messages requests report and ignore them.

## Tests

```bash
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

The unit suite includes local TCP upstreams for both wire protocols and makes
no external API calls. The shared provider contract suite exercises both
dialects through a real iii engine and local stub upstreams.
