# provider-sarvam

Sarvam AI behind [llm-router](https://github.com/iii-hq/workers/tree/main/llm-router), as one worker for every
Sarvam model family:

- **Chat**: `provider::sarvam::stream` runs `sarvam-105b`, `sarvam-30b` and
  `sarvam-m` on Sarvam's OpenAI-compatible Chat Completions API (SSE
  streaming, tool calling, `reasoning_effort`), so `router::chat` and the
  harness use them like any other provider's models.
- **Speech to text**: `provider::sarvam::transcribe` runs Saaras
  (`saaras:v3` default, `saaras:v4`, `saarika:v2.5`) behind
  `router::transcribe`: 22 Indian languages and English, detected when no
  hint is given, word timings folded into sentence segments.
- **Text to speech**: `provider::sarvam::speak` runs Bulbul (`bulbul:v3`
  default, `bulbul:v2`) behind `router::speak`: 11 languages, 35+ speakers,
  mp3, wav, pcm16 or opus back.
- `provider::sarvam::count_tokens` estimates prompt tokens locally with
  tiktoken (Sarvam publishes no tokenizer for the 105B and 30B models).

The voice worker reaches the speech half with `stt.backend: router` /
`tts.backend: router` and a `sarvam::…` model.

## Install

```bash
iii trigger compose::add worker=provider-sarvam
```

Put the key where the router reads it: `SARVAM_API_KEY` in the router's
environment, or paste it into the `sarvam` block of the llm-router entry in
the console's Settings. The catalog fills the moment a key lands.

## Behavior

- Auth: the same key goes out as `Authorization: Bearer` (what the chat
  endpoint documents) and as `api-subscription-key` (what every other
  endpoint takes), so a gateway in front of either shape is satisfied.
- Endpoints: chat at the configured `api_url` (default
  `https://api.sarvam.ai/v1/chat/completions`); the speech endpoints at that
  URL's origin (`/speech-to-text`, `/text-to-speech`), so a residency host
  override moves all three.
- Reasoning: every Sarvam chat model reasons by default; the router's
  `thinking_level` folds onto `reasoning_effort` `low | medium | high`.
  `response_format` maps to `json_object` without schema validation and the
  catalog says so (`supports_structured_output: false`).
- Languages: hints such as `hi`, `hi-IN` or `en-US` become Sarvam's
  `xx-IN` codes; Odia's ISO code `or` becomes `od-IN`. Unknown hints let
  Saaras detect the language and make Bulbul speak English.
- Speakers: `shubh` is the Bulbul v3 default (`anushka` for v2). Pass any
  speaker name Sarvam lists; v2 voices do not work on v3 and vice versa.
- Limits: Bulbul takes up to 2500 characters per request; the REST
  speech-to-text endpoint is for recordings, not hour-long files (use
  Sarvam's batch API for those).

## Surfaces

| Function | Request | Response |
|---|---|---|
| `provider::sarvam::stream` | router chat stream contract | `AssistantMessageEvent` frames |
| `provider::sarvam::transcribe` | `{model?, audio_base64, mime?, language?}` | `{model, text, segments[{text, start_secs, end_secs}], language, duration_secs}` |
| `provider::sarvam::speak` | `{model?, text, voice?, format?, language?, speed?}` | `{model, audio_base64, mime, voice}` |
| `provider::sarvam::count_tokens` | `{model, system_prompt?, tools?, messages}` | `{model, tokens, estimator}` |
| `provider::sarvam::refresh_models` | `{}` | `{ok, count}` |

Errors carry stable prefixes: `provider/not_configured`,
`provider/auth_expired`, `provider/quota_exceeded`, `provider/rate_limited`,
`provider/invalid_input`, `provider/upstream_transient`; chat failures use
the shared `ErrorKind` taxonomy on their error frames.

## Configuration

Nothing in this worker's own config file. The credential, `api_url` and
`max_tokens` resolve per request through `router::provider::resolve`.

## Tests

```bash
cargo test                      # unit tests + wire-schema goldens (+ engine-backed suite when `iii` is on PATH)
UPDATE_GOLDENS=1 cargo test     # regenerate tests/golden after a surface change
```

## Running

```bash
cargo run -- --url ws://127.0.0.1:49134
```
