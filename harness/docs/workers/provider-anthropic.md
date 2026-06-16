# provider-anthropic

Anthropic Messages API streaming provider; exposes
`provider::anthropic::stream` and `provider::anthropic::complete` on the
iii bus.

## Purpose

This worker is the iii-side bridge to Anthropic's Messages API. It resolves
its credential + runtime settings from the harness provider registry
(`harness::provider::resolve`, provider `anthropic`), issues a streaming
Messages API request, parses the SSE response into
`AssistantMessageEvent` frames, and forwards each frame to the
caller-supplied channel. The terminal event is either `Done` or `Error`,
followed by a `close` on the writer.

The orchestrator triggers via `provider::anthropic::stream` (the modern
channel-writer surface); `provider::anthropic::complete` is the legacy
non-streaming shim that internally drains the stream and returns the
final `AssistantMessage`.

Prompt-cache control blocks, tool definitions, and thinking-budget knobs
are translated by
[src/provider-anthropic/wire-messages.ts](harness/src/provider-anthropic/wire-messages.ts)
and
[src/provider-anthropic/wire-tools.ts](harness/src/provider-anthropic/wire-tools.ts)
to match the API contract.

## Registered functions

- `provider::anthropic::stream` — Stream a single assistant turn from Anthropic into the caller-supplied channel. Each `AssistantMessageEvent` is sent as a JSON text message; the terminal event is `Done` or `Error` followed by close.
- `provider::anthropic::complete` — Legacy: drain a streamed Anthropic completion and return the final `AssistantMessage`.

## Triggers

None.

## State keys

None. The worker is stateless beyond the in-process credential cache used
by [src/provider-anthropic/cache.ts](harness/src/provider-anthropic/cache.ts).

## Configuration

From the `provider_anthropic` section of
[config.yaml](harness/config.yaml):

- `default_max_tokens` (default `8192`) — fallback for the request's
  `max_tokens` field when the model is not in the catalog.
- `default_api_url` (default `https://api.anthropic.com/v1/messages`) —
  endpoint for outbound calls.

### Max output tokens

Per request, `max_tokens` resolves as (see
[src/runtime/output-tokens.ts](harness/src/runtime/output-tokens.ts)):

1. A registry override (`providers.anthropic.max_tokens` in the `harness`
   config entry) wins, clamped down to the model's catalog
   `max_output_tokens` when known.
2. Otherwise `min(model.max_output_tokens, 32_000)` — the catalog limit
   comes from [models.dev](https://models.dev) at discovery time; the 32k
   cap is overridable via the `HARNESS_OUTPUT_TOKEN_MAX` env var.
3. Unknown model → `default_max_tokens`.

### Extended thinking

When the run request carries a `thinking_level`
(`minimal|low|medium|high|xhigh`), the request body gains
`thinking: { type: "enabled", budget_tokens }` plus the
`anthropic-beta: interleaved-thinking-2025-05-14` header. Budgets come
from the catalog's `thinking_budgets`, falling back to a formula on the
model's output ceiling (`high` → `min(16000, output/2−1)`, `xhigh` →
`min(31999, output−1)`). The budget always stays below `max_tokens`;
when fewer than 1024 tokens of room remain, thinking is dropped instead
of sending a request the API would reject. Thinking/redacted-thinking
SSE blocks stream as `thinking_*` events and persist as a `thinking`
content block.

## Dependencies

The worker self-declares to the harness provider registry at startup
(`harness::provider::register`) and resolves credentials/settings per
request (`harness::provider::resolve`). It also calls the SDK-provided
`ChannelWriter` injected into the `writer_ref` field of the stream input.

## Source layout

| File | Purpose |
|---|---|
| [src/provider-anthropic/main.ts](harness/src/provider-anthropic/main.ts) | Binary entry point (`iii-provider-anthropic`). |
| [src/provider-anthropic/register.ts](harness/src/provider-anthropic/register.ts) | Registers both functions. |
| [src/provider-anthropic/config.ts](harness/src/provider-anthropic/config.ts) | Loads the `provider_anthropic` section. |
| [src/provider-anthropic/types.ts](harness/src/provider-anthropic/types.ts) | `AnthropicConfig` + `configWithCredential` builder. |
| [src/provider-anthropic/auth.ts](harness/src/provider-anthropic/auth.ts) | `buildConfig` (calls `harness::provider::resolve`). |
| [src/provider-anthropic/discover.ts](harness/src/provider-anthropic/discover.ts) + [refresh-fn.ts](harness/src/provider-anthropic/refresh-fn.ts) | `GET /v1/models` discovery → `models::reconcile` (`provider::anthropic::refresh_models`). |
| [src/provider-anthropic/cache.ts](harness/src/provider-anthropic/cache.ts) | In-process credential / config cache. |
| [src/provider-anthropic/stream.ts](harness/src/provider-anthropic/stream.ts) | `streamAnthropic` async generator: builds request body, fetches SSE, yields `AssistantMessageEvent`s. |
| [src/provider-anthropic/sse.ts](harness/src/provider-anthropic/sse.ts) | SSE parser. |
| [src/provider-anthropic/wire-messages.ts](harness/src/provider-anthropic/wire-messages.ts) | `AgentMessage[]` → Anthropic `messages` translation (text, tool calls, tool results, thinking blocks, cache markers). |
| [src/provider-anthropic/wire-tools.ts](harness/src/provider-anthropic/wire-tools.ts) | `AgentFunction[]` → Anthropic `tools` translation. |
| [src/provider-anthropic/stream-fn.ts](harness/src/provider-anthropic/stream-fn.ts) | `provider::anthropic::stream` handler. |
| [src/provider-anthropic/complete.ts](harness/src/provider-anthropic/complete.ts) | `provider::anthropic::complete` handler (legacy drain-and-return). |
| [src/provider-anthropic/iii.worker.yaml](harness/src/provider-anthropic/iii.worker.yaml) | Worker manifest. |
