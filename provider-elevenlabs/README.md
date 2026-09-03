# provider-elevenlabs

ElevenLabs speech behind [llm-router](https://github.com/iii-hq/workers/tree/main/llm-router): speech to text with
Scribe and text to speech with the Eleven voices. The worker implements the
speech half of the provider protocol, `provider::elevenlabs::transcribe`
and `provider::elevenlabs::speak`, so `router::transcribe` and
`router::speak` reach ElevenLabs without any caller knowing its API. The
voice worker uses them with `stt.backend: router` / `tts.backend: router`.

There is no chat surface: the provider declares only speech models, so
chat routing never lands here and chat pickers never list its models.

## Install

```bash
iii trigger compose::add worker=provider-elevenlabs
```

Then put the key where the router reads it: `ELEVENLABS_API_KEY` in the
router's environment, or paste it into the `elevenlabs` block of the
llm-router entry in the console's Settings. The catalog fills the moment a
key lands.

## Models

| Family | Ids | Notes |
|---|---|---|
| Speech to text | `scribe_v1`, `scribe_v1_experimental` | 99 languages, detected automatically; word timings become sentence segments. |
| Text to speech | every model `GET /v1/models` marks `can_do_text_to_speech` (`eleven_multilingual_v2`, `eleven_flash_v2_5`, `eleven_v3`, ...) | Each carries the language ids it speaks. `eleven_multilingual_v2` is the default. |

`router::models::list` with `modality: "stt"` or `"tts"` lists them;
`router::models::supports` answers `stt`, `tts` and `streaming` (no: this
worker serves batch requests only).

## Surfaces

| Function | Request | Response |
|---|---|---|
| `provider::elevenlabs::transcribe` | `{model?, audio_base64, mime?, language?}` | `{model, text, segments[{text, start_secs, end_secs}], language, duration_secs}` |
| `provider::elevenlabs::speak` | `{model?, text, voice?, format?, language?, speed?}` | `{model, audio_base64, mime, voice}` |
| `provider::elevenlabs::refresh_models` | `{}` | `{ok, count}` |

- `voice` is a voice id or the name of a voice on the account (`George`,
  `Sarah`); names resolve through `GET /v1/voices`. Omitted, George speaks.
- `format` is `mp3` (default, `audio/mpeg`), `wav` (16 kHz PCM wrapped in a
  WAV container, `audio/wav`), `pcm16` (`audio/pcm`) or `opus` (`audio/ogg`).
- `language` on transcribe is an ISO 639-1 or 639-3 hint; on speak it is
  only sent to models that accept one.
- Errors carry stable prefixes: `provider/not_configured`,
  `provider/auth_expired`, `provider/quota_exceeded`, `provider/rate_limited`,
  `provider/invalid_input`, `provider/upstream_transient`.

## Configuration

Nothing in this worker's own config file. The credential and `api_url`
(default `https://api.elevenlabs.io/v1`; the EU, India and Singapore
residency hosts work too) resolve per request through
`router::provider::resolve`.

## Tests

```bash
cargo test                      # unit tests + wire-schema goldens
UPDATE_GOLDENS=1 cargo test     # regenerate tests/golden after a surface change
```

## Running

```bash
cargo run -- --url ws://127.0.0.1:49134
```
