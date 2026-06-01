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

- `default_max_tokens` (default `8192`) — upper bound for the request's
  `max_tokens` field when the caller omits it.
- `default_api_url` (default `https://api.openai.com/v1/chat/completions`)
  — endpoint for outbound calls. Override this to target
  `azure-openai`, `groq`, `openrouter`, or any other Chat-Completions
  compatible gateway; the harness registry looks up the credential by the
  `provider` field, so adjust the gateway and the credential together.

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
| [src/provider-openai/discover.ts](harness/src/provider-openai/discover.ts) + [refresh-fn.ts](harness/src/provider-openai/refresh-fn.ts) | `GET /v1/models` discovery (chat-capable subset) → `models::register` (`provider::openai::refresh_models`). |
| [src/provider-openai/stream.ts](harness/src/provider-openai/stream.ts) | `streamOpenai` async generator: builds the request body, fetches SSE, yields `AssistantMessageEvent`s. |
| [src/provider-openai/sse.ts](harness/src/provider-openai/sse.ts) | SSE parser. |
| [src/provider-openai/wire-messages.ts](harness/src/provider-openai/wire-messages.ts) | `AgentMessage[]` → OpenAI `messages` translation. |
| [src/provider-openai/wire-tools.ts](harness/src/provider-openai/wire-tools.ts) | `AgentFunction[]` → OpenAI `tools` translation. |
| [src/provider-openai/stream-fn.ts](harness/src/provider-openai/stream-fn.ts) | `provider::openai::stream` handler. |
| [src/provider-openai/complete.ts](harness/src/provider-openai/complete.ts) | `provider::openai::complete` handler (legacy drain-and-return). |
| [src/provider-openai/iii.worker.yaml](harness/src/provider-openai/iii.worker.yaml) | Worker manifest. |
