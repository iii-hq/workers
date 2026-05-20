# provider-anthropic

Anthropic Messages API streaming provider; exposes
`provider::anthropic::stream` and `provider::anthropic::complete` on the
iii bus.

## Purpose

This worker is the iii-side bridge to Anthropic's Messages API. It pulls a
credential from the auth worker (`auth::get_token`, provider `anthropic`),
issues a streaming Messages API request, parses the SSE response into
`AssistantMessageEvent` frames, and forwards each frame to the
caller-supplied channel. The terminal event is either `Done` or `Error`,
followed by a `close` on the writer.

The orchestrator dispatches via `provider::anthropic::stream` (the modern
channel-writer surface); `provider::anthropic::complete` is the legacy
non-streaming shim that internally drains the stream and returns the
final `AssistantMessage`.

Prompt-cache control blocks, tool definitions, and thinking-budget knobs
are translated by
[src/provider-anthropic/wire-messages.ts](harness-node/src/provider-anthropic/wire-messages.ts)
and
[src/provider-anthropic/wire-tools.ts](harness-node/src/provider-anthropic/wire-tools.ts)
to match the API contract.

## Registered functions

- `provider::anthropic::stream` — Stream a single assistant turn from Anthropic into the caller-supplied channel. Each `AssistantMessageEvent` is sent as a JSON text message; the terminal event is `Done` or `Error` followed by close.
- `provider::anthropic::complete` — Legacy: drain a streamed Anthropic completion and return the final `AssistantMessage`.

## Triggers

None.

## State keys

None. The worker is stateless beyond the in-process credential cache used
by [src/provider-anthropic/cache.ts](harness-node/src/provider-anthropic/cache.ts).

## Configuration

From the `provider_anthropic` section of
[config.yaml](harness-node/config.yaml):

- `default_max_tokens` (default `8192`) — upper bound for the request's
  `max_tokens` field when the caller omits it.
- `default_api_url` (default `https://api.anthropic.com/v1/messages`) —
  endpoint for outbound calls.

## Dependencies

From
[src/provider-anthropic/iii.worker.yaml](harness-node/src/provider-anthropic/iii.worker.yaml):
`auth-credentials ^0.2.0`. The worker also calls the SDK-provided
`ChannelWriter` injected into the `writer_ref` field of the stream input.

## Source layout

| File | Purpose |
|---|---|
| [src/provider-anthropic/main.ts](harness-node/src/provider-anthropic/main.ts) | Binary entry point (`iii-provider-anthropic`). |
| [src/provider-anthropic/register.ts](harness-node/src/provider-anthropic/register.ts) | Registers both functions. |
| [src/provider-anthropic/config.ts](harness-node/src/provider-anthropic/config.ts) | Loads the `provider_anthropic` section. |
| [src/provider-anthropic/types.ts](harness-node/src/provider-anthropic/types.ts) | `AnthropicConfig` + `configWithCredential` builder. |
| [src/provider-anthropic/auth.ts](harness-node/src/provider-anthropic/auth.ts) | `fetchCredential` (calls `auth::get_token`) + `buildConfig`. |
| [src/provider-anthropic/cache.ts](harness-node/src/provider-anthropic/cache.ts) | In-process credential / config cache. |
| [src/provider-anthropic/stream.ts](harness-node/src/provider-anthropic/stream.ts) | `streamAnthropic` async generator: builds request body, fetches SSE, yields `AssistantMessageEvent`s. |
| [src/provider-anthropic/sse.ts](harness-node/src/provider-anthropic/sse.ts) | SSE parser. |
| [src/provider-anthropic/wire-messages.ts](harness-node/src/provider-anthropic/wire-messages.ts) | `AgentMessage[]` → Anthropic `messages` translation (text, tool calls, tool results, thinking blocks, cache markers). |
| [src/provider-anthropic/wire-tools.ts](harness-node/src/provider-anthropic/wire-tools.ts) | `AgentFunction[]` → Anthropic `tools` translation. |
| [src/provider-anthropic/stream-fn.ts](harness-node/src/provider-anthropic/stream-fn.ts) | `provider::anthropic::stream` handler. |
| [src/provider-anthropic/complete.ts](harness-node/src/provider-anthropic/complete.ts) | `provider::anthropic::complete` handler (legacy drain-and-return). |
| [src/provider-anthropic/iii.worker.yaml](harness-node/src/provider-anthropic/iii.worker.yaml) | Worker manifest. |
