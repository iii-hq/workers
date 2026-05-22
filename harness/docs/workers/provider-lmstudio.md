# provider-lmstudio

LM Studio (localhost) Chat Completions streaming provider; exposes
`provider::lmstudio::stream` and `provider::lmstudio::complete` on the iii bus.

## Purpose

The iii-side bridge to a local [LM Studio](https://lmstudio.ai) server. LM
Studio runs any GGUF model the user has downloaded behind an
OpenAI-compatible REST endpoint (default
`http://localhost:1234/v1/chat/completions`); this worker turns that into
the same `AssistantMessageEvent` stream every other provider emits, so the
orchestrator can swap LM Studio in for a hosted provider without touching
the FSM.

`provider::lmstudio::stream` is the modern channel-writer surface;
`provider::lmstudio::complete` is the legacy drain-and-return helper.

Tool calls are translated to/from the OpenAI function/tool wire format in
[src/provider-lmstudio/wire-tools.ts](harness-node/src/provider-lmstudio/wire-tools.ts);
message translation (text, tool-call, tool-result, system) lives in
[src/provider-lmstudio/wire-messages.ts](harness-node/src/provider-lmstudio/wire-messages.ts).
Whether a *specific* GGUF model actually honours `tool_calls` depends on
how it was trained — modern Qwen/Llama instruct GGUFs work, generic base
models do not. The catalog's `supports_tools: true` flag is a default;
override it for individual models if needed.

## Local-first setup

1. Install LM Studio (UI app) and download a model — e.g. via the in-app
   browser or `lms get qwen/qwen3-4b-2507`.
2. Start the local server. Either click "Start Server" in the LM Studio
   UI, or run:

   ```bash
   lms server start
   lms load qwen/qwen3-4b-2507 -y
   lms ps        # confirm the model is loaded
   ```

3. Trigger a turn against the harness with `provider: 'lmstudio'` and the
   loaded model id:

   ```ts
   await iii.trigger({
     function_id: 'run::start',
     payload: {
       provider: 'lmstudio',
       model: 'qwen/qwen3-4b-2507',
       // …rest of the run::start payload
     },
   });
   ```

LM Studio model IDs are user-controlled (whatever you have loaded), so the
catalog only ships a single `lmstudio-local` capability placeholder.
[src/models-catalog/catalog.ts](harness-node/src/models-catalog/catalog.ts)'s
`syncGet` falls back to that placeholder for any unknown `(lmstudio, *)`
lookup so capability gating still works.

### Optional API key

The default localhost setup runs without authentication; the worker
detects a missing credential and falls back to the literal token
`lm-studio` so the `Authorization` header is always present (some
corporate proxies require it). If you run an authenticated LM Studio
deployment, set `LMSTUDIO_API_KEY` so `auth::get_token` returns it and
the worker will use the real key instead.

## Registered functions

- `provider::lmstudio::stream` — Stream a single assistant turn from a
  local LM Studio Chat Completions server into the caller-supplied
  channel.
- `provider::lmstudio::complete` — Legacy: drain a streamed LM Studio
  chat-completion and return the final `AssistantMessage`.

## Triggers

None.

## State keys

None — the worker is stateless.

## Configuration

From the `provider_lmstudio` section of
[config.yaml](harness-node/config.yaml):

- `default_max_tokens` (default `8192`) — upper bound for the request's
  `max_completion_tokens` field.
- `default_api_url` (default
  `http://localhost:1234/v1/chat/completions`) — endpoint for outbound
  calls. Override if LM Studio is running on a non-default port or behind
  a reverse proxy.

## Routing

The provider is NOT auto-detected by model name; users must pass
`provider: 'lmstudio'` explicitly. See
[src/turn-orchestrator/provider-router.ts](harness-node/src/turn-orchestrator/provider-router.ts) — LM Studio model IDs (e.g. `qwen/...`,
`lmstudio-community/...`) overlap with HF-style identifiers used by other
services, so any regex heuristic would cause false positives.

## Dependencies

From
[src/provider-lmstudio/iii.worker.yaml](harness-node/src/provider-lmstudio/iii.worker.yaml):
`auth-credentials ^0.2.0` (used opportunistically — the worker proceeds
with a synthetic key when no credential is present). The worker also
calls the SDK-provided `ChannelWriter` injected into the `writer_ref`
field of the stream input.

## Source layout

| File | Purpose |
|---|---|
| [src/provider-lmstudio/main.ts](harness-node/src/provider-lmstudio/main.ts) | Binary entry point (`iii-provider-lmstudio`). |
| [src/provider-lmstudio/register.ts](harness-node/src/provider-lmstudio/register.ts) | Registers both functions. |
| [src/provider-lmstudio/config.ts](harness-node/src/provider-lmstudio/config.ts) | Loads the `provider_lmstudio` section. |
| [src/provider-lmstudio/types.ts](harness-node/src/provider-lmstudio/types.ts) | `ChatCompletionsConfig` + `configFromCredential` builder. |
| [src/provider-lmstudio/auth.ts](harness-node/src/provider-lmstudio/auth.ts) | `fetchCredential` (calls `auth::get_token`) + `buildConfig`, with `lm-studio` fallback for no-auth localhost setups. |
| [src/provider-lmstudio/stream.ts](harness-node/src/provider-lmstudio/stream.ts) | `streamLmstudio` async generator: builds the request body, fetches SSE, yields `AssistantMessageEvent`s. |
| [src/provider-lmstudio/sse.ts](harness-node/src/provider-lmstudio/sse.ts) | SSE parser, including LM Studio-specific "no model loaded" error classification. |
| [src/provider-lmstudio/wire-messages.ts](harness-node/src/provider-lmstudio/wire-messages.ts) | `AgentMessage[]` → LM Studio (OpenAI) `messages` translation. |
| [src/provider-lmstudio/wire-tools.ts](harness-node/src/provider-lmstudio/wire-tools.ts) | `AgentFunction[]` → LM Studio (OpenAI) `tools` translation. |
| [src/provider-lmstudio/stream-fn.ts](harness-node/src/provider-lmstudio/stream-fn.ts) | `provider::lmstudio::stream` handler. |
| [src/provider-lmstudio/complete.ts](harness-node/src/provider-lmstudio/complete.ts) | `provider::lmstudio::complete` handler (legacy drain-and-return). |
| [src/provider-lmstudio/iii.worker.yaml](harness-node/src/provider-lmstudio/iii.worker.yaml) | Worker manifest. |
