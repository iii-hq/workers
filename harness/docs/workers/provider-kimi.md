# provider-kimi

Kimi (Moonshot) Chat Completions streaming provider; exposes
`provider::kimi::stream` and `provider::kimi::complete` on the iii bus.

## Purpose

The iii-side bridge to Moonshot's Chat Completions API. It resolves its
credential + runtime settings from the harness provider registry
(`harness::provider::resolve`, provider `kimi`), issues a streaming
`chat/completions` request, parses the SSE response
into `AssistantMessageEvent` frames, and forwards each frame to the
caller-supplied channel. Mirrors the shape of
[provider-openai](harness/docs/workers/provider-openai.md) so the
orchestrator can swap providers without touching the FSM.

`provider::kimi::stream` is the modern channel-writer surface;
`provider::kimi::complete` is the legacy drain-and-return helper.

Tool calls are translated to/from the OpenAI function/tool wire format in
[src/provider-kimi/wire-tools.ts](harness/src/provider-kimi/wire-tools.ts);
message translation (text, tool-call, tool-result, system) lives in
[src/provider-kimi/wire-messages.ts](harness/src/provider-kimi/wire-messages.ts).

## Registered functions

- `provider::kimi::stream` — Stream a single assistant turn from Kimi
  Chat Completions into the caller-supplied channel.
- `provider::kimi::complete` — Legacy: drain a streamed Kimi
  chat-completion and return the final `AssistantMessage`.

## Triggers

None.

## State keys

None — the worker is stateless.

## Configuration

From the `provider_kimi` section of
[config.yaml](harness/config.yaml):

- `default_max_tokens` (default `8192`) — fallback for the request's
  `max_completion_tokens` field when the model is not in the catalog.
  When the catalog knows the model (kimi maps to models.dev `moonshotai`),
  the per-request value resolves as: registry override (clamped to the
  model ceiling) → `min(catalog max_output_tokens, 32_000)` (env override
  `HARNESS_OUTPUT_TOKEN_MAX`) → this fallback. See
  [src/runtime/output-tokens.ts](harness/src/runtime/output-tokens.ts).
- `default_api_url` (default
  `https://api.moonshot.ai/v1/chat/completions`) — endpoint for outbound
  calls. Override to target Moonshot's China endpoint
  (`https://api.moonshot.cn/v1/chat/completions`) or any Chat-Completions
  compatible gateway; the harness registry looks up the credential by the
  `provider` field, so adjust the gateway and the credential together.

## Dependencies

The worker self-declares to the harness provider registry at startup
(`harness::provider::register`) and resolves credentials/settings per
request (`harness::provider::resolve`). The credential falls back to
`MOONSHOT_API_KEY`. It also calls the SDK-provided `ChannelWriter` injected
into the `writer_ref` field of the stream input.

## Source layout

| File | Purpose |
|---|---|
| [src/provider-kimi/main.ts](harness/src/provider-kimi/main.ts) | Binary entry point (`iii-provider-kimi`). |
| [src/provider-kimi/register.ts](harness/src/provider-kimi/register.ts) | Registers both functions. |
| [src/provider-kimi/config.ts](harness/src/provider-kimi/config.ts) | Loads the `provider_kimi` section. |
| [src/provider-kimi/types.ts](harness/src/provider-kimi/types.ts) | `ChatCompletionsConfig` + `configFromCredential` builder. |
| [src/provider-kimi/auth.ts](harness/src/provider-kimi/auth.ts) | `buildConfig` (calls `harness::provider::resolve`). |
| [src/provider-kimi/discover.ts](harness/src/provider-kimi/discover.ts) + [refresh-fn.ts](harness/src/provider-kimi/refresh-fn.ts) | `GET /v1/models` discovery → `models::reconcile` (`provider::kimi::refresh_models`). |
| [src/provider-kimi/stream.ts](harness/src/provider-kimi/stream.ts) | `streamKimi` async generator: builds the request body, fetches SSE, yields `AssistantMessageEvent`s. |
| [src/provider-kimi/sse.ts](harness/src/provider-kimi/sse.ts) | SSE parser. |
| [src/provider-kimi/wire-messages.ts](harness/src/provider-kimi/wire-messages.ts) | `AgentMessage[]` → Kimi `messages` translation. |
| [src/provider-kimi/wire-tools.ts](harness/src/provider-kimi/wire-tools.ts) | `AgentFunction[]` → Kimi `tools` translation. |
| [src/provider-kimi/stream-fn.ts](harness/src/provider-kimi/stream-fn.ts) | `provider::kimi::stream` handler. |
| [src/provider-kimi/complete.ts](harness/src/provider-kimi/complete.ts) | `provider::kimi::complete` handler (legacy drain-and-return). |
| [src/provider-kimi/iii.worker.yaml](harness/src/provider-kimi/iii.worker.yaml) | Worker manifest. |
