---
name: voice
description: Speech in and out for iii — transcribe audio files to timestamped text, run live dictation sessions fed with microphone audio, and read text aloud.
---

# voice

Local speech-to-text and read-aloud on the iii bus. Nothing leaves the machine by default: a small streaming recognizer produces live text and a large second-pass model re-decodes each finished utterance with punctuation and casing, both with models the worker downloads once. An OpenAI-compatible audio endpoint can replace either half through configuration.

## When to Use

- A user attached or named a WAV recording and wants its text: `voice::transcribe`.
- A surface streams microphone audio and wants live text: `voice::dictation::*`.
- A reply should be spoken: `voice::speak`.

## Boundaries

- Audio in is 16 kHz mono 16-bit PCM for dictation; WAV files of any rate for `voice::transcribe`. No other container is decoded.
- The bundled model is English. Other languages need the `openai` backend.
- `voice::speak` on the host backend plays on the machine running the worker, not in the caller's browser; the `openai` backend returns audio for the caller to play.
- Dictation sessions idle past `session_idle_secs` are closed by the worker.

## Functions

- `voice::transcribe` — a WAV file (path or base64) to text with timestamped segments.
- `voice::dictation::start` — open a session; transcript events go to `output_function_id`.
- `voice::dictation::push` — feed one base64 PCM chunk (rising `seq`).
- `voice::dictation::stop` — close a session and return its transcript (`discard` to drop it).
- `voice::dictation::list` — open sessions.
- `voice::speak` — read text aloud; returns a `speech_id`.
- `voice::speak::stop` — stop host playback.
- `voice::models::list` — local models and whether each is installed.
- `voice::models::download` — install a local model, checksum-verified.
- `voice::doctor` — backends, model state, open sessions.

## Reactive triggers

- `voice::transcript` — partial, final, closed and error events of dictation sessions (filter `session_id`).
- `voice::session-started`, `voice::session-stopped` — session lifecycle.
- `voice::model-progress` — download progress, one event per megabyte and a final `done`.
