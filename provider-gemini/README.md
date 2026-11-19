# provider-gemini

Google Gemini models behind
[llm-router](https://github.com/iii-hq/workers/tree/main/llm-router), on the
Gemini Interactions API. Install this worker next to the router, provide a
Gemini API key, and the live Gemini chat catalog appears behind `router::chat`
and `router::complete`: streaming text, thought summaries, function calls
(with images in function results), structured JSON output and implicit prompt
caching. The router invokes the provider functions worker-to-worker, and
`iii-permissions.yaml` blocks agents from calling them directly.

## Install

```bash
iii call compose::add worker=provider-gemini worker=llm-router
```

`iii call compose::add` resolves the workers and their dependencies, writes
exact declarations to `worker-compose.yaml`, and reconciles the Compose project.

## Quickstart

Create a key at [Google AI Studio](https://aistudio.google.com/apikey) and put
it in the `gemini` slice of the engine's `llm-router` configuration entry, or
export `GEMINI_API_KEY` where the router runs:

```json
{ "providers": { "gemini": { "api_key": "AIza…" } } }
```

The provider discovers the catalog as soon as a key is configured. Then call
the router with a Gemini model id:

```ts
const result = await iii.call({
  function_id: 'router::complete',
  payload: {
    model: 'gemini-3.8-flash',
    thinking_level: 'low',
    messages: [
      {
        role: 'user',
        content: [{ type: 'text', text: 'Summarise this diff in one line.' }],
        timestamp: Date.now(),
      },
    ],
  },
  timeoutMs: 320_000,
});
```

For token-by-token output, call `router::chat` with an iii channel as described
in [llm-router's Quickstart](https://github.com/iii-hq/workers/blob/main/llm-router/README.md).
`router::provider::list` reports a `default_model` for gemini: the newest
catalog model of the best-ranked variant, `flash` first, then `hydrogen`.

## Configuration

All provider configuration lives in the router's `llm-router` entry:

```jsonc
"gemini": {
  "api_key": "AIza…",                                                        // or GEMINI_API_KEY
  "api_url": "https://generativelanguage.googleapis.com/v1beta/interactions", // full endpoint URL
  "max_tokens": 65536                                                        // thinking + output; capped at 65536
}
```

The models listing is read from the `/models` sibling of `api_url`, so a proxy
that serves both routes works with one override, as long as `api_url` ends in
`/interactions`. Any other path gets no model listing, and the key is never
sent to the public host in its place. The default is `v1beta`
because `v1` does not serve the preview models or the `-latest` aliases.

Worker environment variables:

| Variable | Default | Meaning |
|---|---|---|
| `GEMINI_API_KEY` | unset | Credential fallback declared to `llm-router` |
| `PROVIDER_READ_TIMEOUT_SECS` | `900` | Upstream read timeout and the deadline for generation to start; Google asks for at least 15 minutes on hard Gemini 4 prompts |
| `III_URL` | `ws://127.0.0.1:49134` | Engine WebSocket when `--url` is not set |

The binary also accepts the standard `--url`, `--manifest`, and `--config`
flags; `--config` is ignored with a warning.

## Models, thinking and accounting

`GET /models` is the source of truth for ids, context windows, output limits
and display names. Non-chat models (speech, image, embedding, live, robotics
and similar) are filtered out. Prices come from Google's pricing page and are
kept in [`src/curated.rs`](https://github.com/iii-hq/workers/blob/main/provider-gemini/src/curated.rs);
the `-latest` aliases have no price.

Every listed model thinks. `thinking_level` maps one to one onto Gemini's
levels, clamped to what the model accepts, with a warning when it changes:
`gemini-4-neon` and `gemini-4-hydrogen` take `low` to `xhigh`;
`gemini-3.6-flash`, `gemini-3.5-flash`, `gemini-3.5-flash-lite`,
`gemini-3.1-flash-lite` and `gemini-3-flash-preview` take `minimal` to `high`;
everything else takes `low` to `high`. With no level the model's own default applies. Thought summaries are
always requested, so the console shows the thinking as it streams.

Requests are stateless (`store: false`): the router's history is sent in full
every turn, and Google's implicit cache bills the repeated prefix at the cached
rate. Usage maps `total_cached_tokens` to `cache_read`, the rest of the input
to `input`, and output plus thought tokens to `output` (Google bills thinking
at the output rate); thought tokens are also reported as `reasoning`.

## Known limits

- **API key only.** OAuth credentials are refused with a clear error; the
  Gemini API needs a project header the router's OAuth credential does not
  carry.
- **No token counter.** `router::count_tokens` answers `no_token_counter` for
  Gemini models and callers use their own estimate.
- **Thinking cannot be switched off.** `thinking_level: off` runs at the
  model's lowest level and says so in a warning.
- **No long decode.** `max_output_tokens` is capped at 65,536; Gemini's
  continuation loop for longer outputs is not implemented.
- **`gemini-2.5-*` is not listed.** Those ids answer "no longer available to
  new users" for new keys.
- **Messages from another provider** are replayed behind a placeholder
  thought step, because Gemini rejects a function call in the current turn that
  has no thought at the start of its model run. Gemini's own thought signatures are replayed as
  received; thinking from other providers is dropped.
- The router ends any provider stream after its `stream_timeout_ms` (600 s by
  default); raise it for prompts that need Gemini 4's full 15 minutes.

## Tests

```bash
cargo test
GEMINI_API_KEY=… cargo test --test live_smoke -- --ignored --nocapture
```

The unit suite replays trimmed captures of the live API from
`tests/fixtures/` and makes no external calls. The integration suite runs a
real engine, the real router and this provider against a local stub upstream.
The live suite (ignored by default, five short requests) checks a text turn,
an `agent_call` tool loop, a tool call made by another provider and a
history that opens on a tool call.
