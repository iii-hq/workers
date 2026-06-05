# provider-openai

OpenAI Chat Completions streaming provider; exposes
`provider::openai::stream` and `provider::openai::complete` on the iii
bus.

## Purpose

The iii-side bridge to OpenAI's Chat Completions API. It resolves its
credential + runtime settings from the harness provider registry
(`harness::provider::resolve`, provider `openai`), issues a streaming
`chat/completions` request, parses the SSE response
into `AssistantMessageEvent` frames, and forwards each frame to the
caller-supplied channel. Mirrors the shape of
[provider-anthropic](harness/docs/workers/provider-anthropic.md) so
the orchestrator can swap providers without touching the FSM.

`provider::openai::stream` is the modern channel-writer surface;
`provider::openai::complete` is the legacy drain-and-return helper.

Tool calls are translated to/from the OpenAI function/tool wire format in
[src/provider-openai/wire-tools.ts](harness/src/provider-openai/wire-tools.ts);
message translation (text, tool-call, tool-result, system) lives in
[src/provider-openai/wire-messages.ts](harness/src/provider-openai/wire-messages.ts).

## Registered functions

- `provider::openai::stream` — Stream a single assistant turn from OpenAI Chat Completions into the caller-supplied channel.
- `provider::openai::complete` — Legacy: drain a streamed OpenAI chat-completion and return the final `AssistantMessage`.

## Triggers

None.

## State keys

None — the worker is stateless.

## Configuration

From the `provider_openai` section of
[config.yaml](harness/config.yaml):

- `default_max_tokens` (default `8192`) — fallback for the request's
  `max_completion_tokens` field when the model is not in the catalog.
- `default_api_url` (default `https://api.openai.com/v1/chat/completions`)
  — endpoint for outbound calls. Override this to target
  `azure-openai`, `groq`, `openrouter`, or any other Chat-Completions
  compatible gateway; the harness registry looks up the credential by the
  `provider` field, so adjust the gateway and the credential together.

### Max output tokens

Per request, `max_completion_tokens` resolves as (see
[src/runtime/output-tokens.ts](harness/src/runtime/output-tokens.ts)):

1. A registry override (`providers.openai.max_tokens` in the `harness`
   config entry) wins, clamped down to the model's catalog
   `max_output_tokens` when known.
2. Otherwise `min(model.max_output_tokens, 32_000)` — the catalog limit
   comes from [models.dev](https://models.dev) at discovery time; the 32k
   cap is overridable via the `HARNESS_OUTPUT_TOKEN_MAX` env var.
3. Unknown model → `default_max_tokens`.

Reasoning tokens count toward `max_completion_tokens`; the clamped
default leaves ample room, but a very low registry override can starve
output on reasoning models.

### Reasoning effort

Reasoning models (catalog `supports_thinking`, or ids matching
`gpt-5*`/`o3*`/`o4*` — the o1 family is excluded, it rejects the param)
get `reasoning_effort: "medium"` by default. Note for gateway routing
(`default_api_url` overridden): the default applies to gpt-5-style ids
there too; a gateway that rejects `reasoning_effort` can be opted out per
model by setting `supports_thinking: false` in the catalog.
A `thinking_level` on the run request maps onto the effort ladder for
the model family (gpt-5.1: none–high; gpt-5.2+: adds xhigh; gpt-5-pro:
high only; o-series: low–high; chat-tuned variants take no effort
param). See [src/provider-openai/reasoning.ts](harness/src/provider-openai/reasoning.ts).

## Dependencies

The worker self-declares to the harness provider registry at startup
(`harness::provider::register`) and resolves credentials/settings per
request (`harness::provider::resolve`). It also calls the SDK-provided
`ChannelWriter` injected into the `writer_ref` field of the stream input.

## Source layout

| File | Purpose |
|---|---|
| [src/provider-openai/main.ts](harness/src/provider-openai/main.ts) | Binary entry point (`iii-provider-openai`). |
| [src/provider-openai/register.ts](harness/src/provider-openai/register.ts) | Registers both functions. |
| [src/provider-openai/config.ts](harness/src/provider-openai/config.ts) | Loads the `provider_openai` section. |
| [src/provider-openai/types.ts](harness/src/provider-openai/types.ts) | `ChatCompletionsConfig` + `configFromCredential` builder. |
| [src/provider-openai/auth.ts](harness/src/provider-openai/auth.ts) | `buildConfig` (calls `harness::provider::resolve`). |
| [src/provider-openai/discover.ts](harness/src/provider-openai/discover.ts) + [refresh-fn.ts](harness/src/provider-openai/refresh-fn.ts) | `GET /v1/models` discovery (chat-capable subset) → `models::reconcile` (`provider::openai::refresh_models`). |
| [src/provider-openai/stream.ts](harness/src/provider-openai/stream.ts) | `streamOpenai` async generator: builds the request body, fetches SSE, yields `AssistantMessageEvent`s. |
| [src/provider-openai/sse.ts](harness/src/provider-openai/sse.ts) | SSE parser. |
| [src/provider-openai/wire-messages.ts](harness/src/provider-openai/wire-messages.ts) | `AgentMessage[]` → OpenAI `messages` translation. |
| [src/provider-openai/wire-tools.ts](harness/src/provider-openai/wire-tools.ts) | `AgentFunction[]` → OpenAI `tools` translation. |
| [src/provider-openai/stream-fn.ts](harness/src/provider-openai/stream-fn.ts) | `provider::openai::stream` handler. |
| [src/provider-openai/complete.ts](harness/src/provider-openai/complete.ts) | `provider::openai::complete` handler (legacy drain-and-return). |
| [src/provider-openai/iii.worker.yaml](harness/src/provider-openai/iii.worker.yaml) | Worker manifest. |
