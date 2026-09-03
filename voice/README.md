# voice

Speak to the console. The voice worker turns microphone audio into text as
you talk, transcribes recordings to timestamped segments, and reads replies
aloud. Speech-to-text runs on the machine running the worker with a small
streaming model it downloads once; nothing leaves the machine unless you point
it at an OpenAI-compatible audio endpoint.

Recognition is two passes. A small streaming model produces the words as
you speak and decides where utterances end; a large second-pass model
(Parakeet TDT 0.6B v2) then re-decodes each finished utterance for the final
text, with punctuation and casing and the accuracy of hosted dictation
tools. The second pass downloads in the background the first time it is
needed (about 660 MB); until it lands, the streaming text stands.

Three surfaces, one worker:

1. **A mic in every chat.** A `Dictate` button joins the composer toolbar
   beside attach (the chat header on consoles without that slot). Click to
   toggle, or hold to talk. Words appear as you speak in a header pill and
   land in the composer when you stop, ready to edit before sending.
2. **Read aloud.** Every finished turn gets a `Read aloud` action above the
   composer that speaks the last reply.
3. **A voice page.** `#/ext/voice` shows the engine state, downloads the
   model, transcribes a WAV file with per-segment timestamps, and runs a
   dictation test.

Agents get the same through `voice::*` functions and the `voice::transcript`
trigger.

## Install

```bash
iii trigger compose::add worker=voice
```

The first dictation or transcription downloads the streaming model (about
44 MB) into `data/voice/models` under the project directory and loads it in
well under a second; the second-pass model (about 660 MB, CC-BY-4.0) follows
in the background. `voice::models::download id=parakeet-tdt-0.6b-v2` fetches
it ahead of time.

## Quickstart

Transcribe a recording:

```bash
iii trigger voice::transcribe path=./meeting.wav
```

```json
{
  "text": "The quick brown fox jumps over the lazy dog.",
  "segments": [
    { "segment": 0, "text": "The quick brown fox jumps over the lazy dog.", "start_secs": 0.3, "end_secs": 2.9 }
  ],
  "duration_secs": 5.5,
  "model": "zipformer-en-20m",
  "backend": "local"
}
```

Read a reply aloud, then stop it:

```bash
iii trigger voice::speak text="Build finished, three tests failed."
iii trigger voice::speak::stop
```

Check what the worker can do right now:

```bash
iii trigger voice::doctor
```

## Dictation over the bus

A dictation session is a live recognizer stream. The caller names a function
that receives transcript events, then pushes 16 kHz mono 16-bit PCM in ~100 ms
chunks:

```bash
iii trigger voice::dictation::start output_function_id=my::transcript
iii trigger voice::dictation::push session_id=d_… seq=1 pcm16_base64=…
iii trigger voice::dictation::stop session_id=d_…
```

Events carry `kind` (`partial` replaces the in-progress text, `final` commits
a segment, `closed` ends the session), `seq`, `segment`, and `text`. The same
events fan out on the `voice::transcript` trigger, filterable by `session_id`.
Sessions idle for `session_idle_secs` are closed by the worker.

## Configuration

Stored in the `configuration` worker under `voice` and editable in the
console's Settings; every field takes effect on the next call, and a model or
endpointing change reloads the recognizer on next use.

| Field | Default | Meaning |
| --- | --- | --- |
| `models_dir` | `data/voice/models` | Where models live (relative to the project directory). |
| `stt.backend` | `local` | `local` (bundled recognizer) or `openai` (any `/v1/audio/transcriptions` server). |
| `stt.model` | `zipformer-en-20m` | Streaming model id from `voice::models::list` (live partial text). |
| `stt.final_model` | `parakeet-tdt-0.6b-v2` | Second-pass model that re-decodes each utterance for the final text. Empty disables the second pass. |
| `stt.num_threads` | `2` | Decoder threads. |
| `stt.silence_after_speech_secs` | `0.8` | Trailing silence that commits an utterance. |
| `stt.silence_without_speech_secs` | `2.4` | Trailing silence that ends an empty segment. |
| `stt.max_utterance_secs` | `20` | Longest utterance before a forced commit. |
| `stt.openai.base_url`, `api_key`, `model`, `language` | OpenAI defaults | The remote transcription endpoint. `api_key` accepts `${OPENAI_API_KEY}`. |
| `tts.backend` | `host` | `host` (`say` on macOS, `espeak-ng` on Linux), `openai` (`/v1/audio/speech`, audio returned to the caller), or `off`. |
| `tts.voice`, `tts.rate_wpm` | system default | Host voice and speaking rate. |
| `tts.max_speak_chars` | `4000` | Longest text one `voice::speak` call reads. |
| `tts.openai.base_url`, `api_key`, `model`, `voice` | OpenAI defaults | The remote speech endpoint. |
| `max_audio_bytes` | `10485760` | Largest inline (`audio_base64`) file for `voice::transcribe`. |
| `max_sessions` | `8` | Open dictation sessions across all callers. |
| `session_idle_secs` | `120` | Idle time before a session is closed. |

A local whisper server (whisper.cpp `server`, speaches, and similar) works as
the `openai` backend with an empty `api_key`.

## Functions

| Function | What it does |
| --- | --- |
| `voice::transcribe` | A WAV file (path or base64) to text with segments. |
| `voice::dictation::start` / `push` / `stop` / `list` | Live sessions. |
| `voice::speak` / `voice::speak::stop` | Read text aloud, stop playback. |
| `voice::models::list` / `voice::models::download` | Local model catalog and install. |
| `voice::doctor` | Backends, model state, open sessions. |

Triggers: `voice::transcript`, `voice::session-started`,
`voice::session-stopped`, `voice::model-progress`.

## Limits

- The bundled model is English only; other languages need the `openai`
  backend.
- WAV is the only container decoded. Convert other formats first, for
  example `ffmpeg -i in.m4a -ac 1 -ar 16000 out.wav`.
- Read-aloud on the `host` backend plays on the worker's machine. In a remote
  deployment use the `openai` backend so the audio reaches the browser.
- Prebuilt binaries cover macOS (Intel and Apple silicon) and glibc Linux
  (x86_64 and aarch64); the speech engine ships no static libraries for musl
  or 32-bit ARM.

## Development

```bash
cd voice
cargo build                 # builds ui/ with pnpm first
cargo test
III_VOICE_UI_WATCH=1 cargo run   # hot-reloads the console UI from ui/dist
```

Run the recognizer against real audio with the model on disk:

```bash
VOICE_TEST_DOWNLOAD=1 VOICE_TEST_WAV=./clip.wav cargo test --test engine_live -- --nocapture
```
