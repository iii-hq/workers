---
name: voice
description: Speech in and out for iii — transcribe audio files to timestamped text, run live dictation sessions fed with microphone audio, and read text aloud.
---

# voice

Local speech-to-text and read-aloud on the iii bus. Nothing leaves the machine by default: a small streaming recognizer produces live text, while bundled Parakeet or a configured host `whisper-cli` can produce accurate final text. An OpenAI-compatible endpoint or any speech provider registered with llm-router can replace either half through configuration.

## When to Use

- A user attached or named a WAV recording and wants its text: `voice::transcribe`.
- A surface streams microphone audio and wants live text: `voice::dictation::*`.
- A reply should be spoken: `voice::speak`.

## Boundaries

- Audio in is 16 kHz mono 16-bit PCM for dictation; WAV files of any rate for `voice::transcribe`. No other container is decoded.
- The bundled models are English. Portuguese and other languages need `stt.backend: whisper_cpp` with a multilingual GGML model, or a remote backend.
- For natural local read-aloud, install `piper-tts`, enter your language in Voice's Piper language filter, choose a matching voice from the bundled multilingual catalog, explicitly download it, and select `tts.backend: piper`. `piper-pt-br-faber-medium` remains available for Brazilian Portuguese. Filtering does not download or change the active voice. `tts.piper.command` is an executable path, not a shell command. No fallback to robotic eSpeak occurs if Piper is unavailable.
- `tts.piper.device` defaults to `auto` (request NVIDIA CUDA, allow ONNX Runtime CPU fallback; retry once on CPU if the GPU-preferred process fails). `cpu` forces CPU. Auto allocates 30 seconds per attempt; CPU-only gets 60 seconds. GPU requires compatible onnxruntime-gpu/CUDA/cuDNN in Piper's own environment. No drivers are auto-installed; doctor reports policy, not verified GPU usage.
- In Console, Read selection speaks only highlighted text in that chat. Voice chat is opt-in per browser/conversation and reads future completed responses; it does not send messages or keep the microphone open. Stop disables automatic reading.
- Every `voice::speak` backend returns `audio_base64` and `mime` to the caller. `host` generates WAV using say/espeak in file-output mode, never server playback. The requesting browser plays/stops its own audio; other clients are unaffected.
- Dictation sessions idle past `session_idle_secs` are closed by the worker.

## Functions

- `voice::transcribe` — a WAV file (path or base64) to text with timestamped segments.
- `voice::dictation::start` — open a session; transcript events go to `output_function_id`.
- `voice::dictation::push` — feed one base64 PCM chunk (rising `seq`).
- `voice::dictation::stop` — close a session and return its transcript (`discard` to drop it).
- `voice::dictation::list` — open sessions.
- `voice::speak` — read text aloud; returns a `speech_id`.
- `voice::speak::stop` — legacy compatibility, returns `stopped: 0`. Stop browser audio locally.
- `voice::models::list` — local models, whether each is installed, and each one's license, author and source (Parakeet TDT 0.6B v2 is CC BY 4.0: keep the attribution if you copy its files elsewhere).
- `voice::models::download` — install a local model, checksum-verified.
- `voice::models::remove` — delete a downloaded model.
- `voice::doctor` — backends, model state, open sessions.

## Reactive triggers

- `voice::transcript` — partial, final, closed and error events of dictation sessions (filter `session_id`).
- `voice::session-started`, `voice::session-stopped` — session lifecycle.
- `voice::model-progress` — download progress, one event per megabyte and a final `done`.
