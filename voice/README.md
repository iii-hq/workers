# voice

Speak to the console. The voice worker turns microphone audio into text as
you talk, transcribes recordings to timestamped segments, and reads replies
aloud. Speech-to-text can use bundled English models or a host-installed
`whisper.cpp` command and multilingual GGML model; nothing leaves the machine
unless you select a remote backend.

Dictation is two passes. A small streaming model produces words as you speak
and decides where utterances end. The final pass can use the bundled Parakeet
TDT 0.6B v2 model, a local `whisper-cli` process, or a remote speech backend.
The selected final pass re-decodes each finished utterance with punctuation and
casing; until it answers, the streaming text remains visible.

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

## Direct whisper.cpp

On **Voice → Overview**, select **whisper.cpp on this machine**, choose a
Whisper model, then click **Download** when it is missing. Tiny, base, small,
medium, and large-v3-turbo are multilingual choices; the UI shows download
sizes, installation state, progress, and errors. The **Models** section also
lets you select a model with **Use**, even before downloading it. Model
selection does not start a download. Settings offers the same picker and
keeps a **Custom GGML file path** option for existing files.

Downloads are stored under `models_dir/<model-id>/` and checked against
pinned SHA-256 digests. Save changes to the models directory before downloading
from Settings; download actions use the saved directory, not the form draft.
The live streaming model can also be selected and downloaded on Overview,
regardless of the final transcription backend.

Set `stt.backend` to `whisper_cpp` to invoke an existing `whisper-cli` binary
without running an HTTP server. The worker writes a temporary 16 kHz mono WAV,
asks the CLI for JSON, parses its timestamped segments, and removes the
temporary files. The command is executed directly, never through a shell.

```yaml
stt:
  backend: whisper_cpp
  model: zipformer-en-20m       # live dictation preview and boundaries
  num_threads: 4
  whisper_cpp:
    command: whisper-cli
    model: whisper-large-v3-turbo # or /models/ggml-large-v3-turbo.bin
    language: pt
    timeout_secs: 300
```

Install `whisper-cli` separately; downloading weights does not install the
command. Download a catalog model ahead of use with:

```bash
iii trigger voice::models::download id=whisper-large-v3-turbo
```

Whisper weights are downloaded only when requested, not during transcription.
Use a multilingual GGML model for Portuguese. `.en` models only recognize
English. `voice::doctor` reports a missing command or model before the first
transcription. The configured language is used unless a `voice::transcribe`
request supplies its own `language` value.

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

Generate speech on the server and play it in the requesting browser:

- **Voice → Read aloud → Local synthesis → this browser** uses `espeak-ng`
  (Linux) or `say` (macOS) in WAV file-output mode. No sound plays on the server.
- `voice::speak` returns `audio_base64`, `mime`, a unique `speech_id`, and
  `played: false` for every enabled backend. The browser plays that response.
- Each request uses a private temporary file, removed after synthesis. Multiple
  clients can generate speech independently; there is no audio broadcast.
- **Stop** pauses and releases only this browser's audio. While generation is
  pending it discards the eventual response; it does not cancel other clients'
  requests or stop them globally. Generation may continue until completion or
  the local 60-second synthesis timeout.
- `espeak-ng` is needed only on the worker machine, not on each client. A server
  audio device is not needed. For Brazilian Portuguese set `tts.voice: pt-br`.
- Audio is returned after synthesis completes, not streamed. Local output is
  capped at 64 MiB in addition to `tts.max_speak_chars`.

For an API caller, generate audio (this command alone does **not** play it):

```bash
iii trigger voice::speak text="Build finished, three tests failed."
```

Check what the worker can do right now:

```bash
iii trigger voice::doctor
```

## Natural local voice and voice chat

### Listening and reading model configuration

The Models page separates model selection and downloads into two independent
sections rather than a single Speech models list:

- **Listening models · Speech to text**: Zipformer (live dictation), Parakeet
  and Whisper (final transcription). These process microphone or recorded audio
  into text; translation into another language is not configured on this page.
- **Reading models · Text to speech**: Piper voices used by Read aloud and
  Voice chat to speak text in the requesting browser. The language filter is
  scoped to this section only; it never hides listening models.

Each section has its own installed count. Selection buttons explicitly say
**Use for live dictation**, **Use for transcription**, or **Use for read aloud**.
Choosing a listening model preserves the configured reading voice and vice versa.
Download, progress, error and removal controls remain available in both groups.
This is a model-configuration layout change, not a new microphone/playback mode.

### Choose your language before downloading

Piper is not limited to Brazilian Portuguese. In Overview → Neural voice,
Settings → Read aloud, or Models, the **What language do you speak?** field
filters the available voices. Type a language name (`português`, `English`,
`español`) or a regional code (`pt-BR`, `pt-PT`, `en-US`). Suggestions show the
language/region and number of voices. Language names are searchable in English,
Portuguese, the browser's locale and the language's native name.

Choose a matching voice and explicitly click **Download**. Names, quality,
download size and installed status help distinguish the choices. A language
without matches shows an empty-state explanation; it never falls back silently
to Portuguese. Filtering does not change the configured voice, download files,
or delete installed voices. On Models, installed Piper voices remain visible
for management until a language filter is entered.

The bundled snapshot contains **176 voices across 57 language/region variants**
from `rhasspy/piper-voices` at commit
`1162a9173d0ce503555aed757976b7a9912eae4c`. Only metadata is embedded, not model
weights. Catalog browsing works offline; a requested download needs network
access. Each additional voice's ONNX, configuration and model card use pinned
URLs and SHA-256 digests. The existing Faber id and installed directory remain
compatible. Availability in this catalog is not a claim that every voice has
been synthesis-tested; licenses and quality vary by voice, so consult its card.

Maintainers can refresh a reviewed snapshot with
`python3 voice/scripts/import_piper_catalog.py --revision <40-character-commit> --output <new-snapshot.json>`.
The importer reads upstream metadata, verifies small files against Git blob
ids and derives their SHA-256, and takes weight hashes from LFS metadata. It
does not download weights and refuses to overwrite an existing destination.
Catalog updates require a worker build; browsing does not fetch a mutable
remote index at runtime.

### GPU preferred, CPU fallback

`tts.piper.device` defaults to `auto`: the worker requests Piper's `--cuda`
mode (NVIDIA CUDA). ONNX Runtime may transparently use CPU when that provider
is unavailable. If the CUDA-preferred process fails or times out, the worker
retries once without `--cuda`, with the same model and text, on CPU. Auto uses
up to 30 seconds per attempt, within the existing 60-second synthesis budget.
If both attempts fail, both errors are reported. Files are private to each
attempt and the CPU attempt never consumes partial GPU output.

Choose **Settings → Read aloud → Processing device → CPU only** to skip GPU
and allow a single CPU attempt up to 60 seconds. This option changes Piper TTS
only, not Whisper, listening models or the browser playback destination.

```yaml
tts:
  backend: piper
  piper:
    device: auto # prefer GPU; use cpu to force CPU
```

GPU acceleration requires a compatible NVIDIA GPU/driver, CUDA/cuDNN libraries
and the GPU build of ONNX Runtime (`onnxruntime-gpu`) **inside the same Python
environment/container as Piper**. The regular `piper-tts` install uses CPU
ONNX Runtime. Do not treat a GPU physically present on the host as sufficient,
or install CPU and GPU ONNX Runtime packages side by side (they share the same
Python module). Match package/library versions using ONNX Runtime's CUDA
requirements. The worker installs no GPU runtime or drivers automatically.
This CUDA path does not enable Apple Metal or AMD acceleration.

Doctor/Overview show the requested policy, not the actual provider used. The
CLI does not report its effective provider in the speech response; successful
auto synthesis alone is not proof of GPU usage. Model quality and voice do
not change between CPU and GPU, and GPU is not guaranteed to be faster for
short requests because Piper currently loads the model per request.

### Install and use Piper

For a more natural voice than eSpeak, select **Piper · natural local voice** in
Voice → Overview or Settings. Install `piper-tts==1.8.0` separately on the worker
(for example with `uv tool install piper-tts==1.8.0 --python 3.12`). The executable
must be on the worker's PATH, or set `tts.piper.command` to its absolute path.
Piper is invoked as a separate process, not linked into the worker.

For example, filter by `pt-BR`, download **Piper Faber · Brazilian Portuguese**
in Models (~63 MB), then choose **Use model**. Other languages use the same flow. ONNX weights, configuration and the model card are SHA-256 verified.
This changes TTS only: your Whisper/dictation configuration is preserved. No
model downloads happen implicitly during speech generation, and missing Piper
or weights produce an actionable error rather than silently using eSpeak.

```yaml
tts:
  backend: piper
  piper:
    command: piper
    model: piper-pt-br-faber-medium
```

The chat footer has three independent controls:

- **Read aloud** reads the last final assistant reply.
- **Read selection** appears when you highlight a passage in this conversation's
  messages. It reads only that passage, not selections in other panes or forms.
  The Voice → Read aloud text area also supports selecting only a passage.
- **Voice chat** explicitly enables automatic reading of future completed replies
  in this browser/conversation. Enable it before sending a typed or dictated
  message. It does not open the microphone or send a draft automatically.
  Duplicate, failed, cancelled and intermediate turn events are ignored; enabling
  the mode does not replay old history. A new turn stops the previous audio.
  **Stop** also turns automatic reading off. Switching chats, closing the view,
  or a playback error disables it; other clients are unaffected.

The browser may block automatic playback according to its autoplay policy; errors
are shown rather than claiming playback succeeded. Piper loads its model per
request and returns the completed WAV, so there is a generation delay. The
configured `tts.max_speak_chars` still applies; select a shorter passage if needed.

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
| `stt.backend` | `local` | `local` (bundled recognizer), `whisper_cpp` (host `whisper-cli`), `openai` (any `/v1/audio/transcriptions` server), or `router` (any speech provider registered with llm-router). |
| `stt.model` | `zipformer-en-20m` | Streaming model for live words and sentence boundaries: `zipformer-en-20m` (44 MB, fastest) or `zipformer-en-large` (73 MB, fewer misheard words in the preview). |
| `stt.final_model` | `parakeet-tdt-0.6b-v2` | Second-pass model that re-decodes each utterance for the final text. Empty disables the second pass. |
| `stt.num_threads` | `2` | Decoder threads. |
| `stt.silence_after_speech_secs` | `0.8` | Trailing silence that commits an utterance. |
| `stt.silence_without_speech_secs` | `2.4` | Trailing silence that ends an empty segment. |
| `stt.max_utterance_secs` | `20` | Longest utterance before a forced commit. |
| `stt.openai.base_url`, `api_key`, `model`, `language` | OpenAI defaults | The remote transcription endpoint. `api_key` accepts `${OPENAI_API_KEY}`. |
| `stt.router.model`, `language` | empty | Router model (`provider::model`) for `voice::transcribe` and for the second pass of every dictated sentence; empty lets the router pick. Keys live in the router's Settings, not here. |
| `stt.whisper_cpp.command` | `whisper-cli` | Command on `PATH`, or an explicit executable path. |
| `stt.whisper_cpp.model` | `whisper-large-v3-turbo` | Catalog id (`whisper-tiny`, `whisper-base`, `whisper-small`, `whisper-medium`, `whisper-large-v3-turbo`) or a custom GGML file path. Download catalog models from the Voice page or `voice::models::download`; custom paths remain user-managed. |
| `stt.whisper_cpp.language`, `timeout_secs` | `auto`, `300` | ISO 639-1 language hint (`pt`) or detection, and the per-process timeout. |
| `tts.backend` | `host` | `piper` (natural local neural voice), `host` (local WAV synthesis with `say` on macOS or `espeak-ng` on Linux, returned to the browser), `openai` (`/v1/audio/speech`, audio returned to the caller), `router` (llm-router's `router::speak`, any registered speech provider, audio returned to the caller), or `off`. |
| `tts.piper.device` | `auto` | Prefer NVIDIA CUDA, with CPU fallback; `cpu` forces CPU. Requires GPU dependencies in the Piper environment to accelerate. |
| `tts.piper.command`, `tts.piper.model` | `piper`, `piper-pt-br-faber-medium` | Local neural TTS executable and downloaded voice catalog id. Audio plays in the browser. |
| `tts.voice`, `tts.rate_wpm` | system default | Host voice and speaking rate. |
| `tts.max_speak_chars` | `4000` | Longest text one `voice::speak` call reads. |
| `tts.openai.base_url`, `api_key`, `model`, `voice` | OpenAI defaults | The remote speech endpoint. |
| `tts.router.model`, `voice`, `format` | empty, empty, `mp3` | Router model, voice id or name, and container for `voice::speak`. |
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
| `voice::speak` | Generate audio for the requesting client to play. |
| `voice::speak::stop` | Legacy compatibility: returns `stopped: 0`; browser playback is stopped locally. |
| `voice::models::list` / `voice::models::download` / `voice::models::remove` | Local model catalog, install, delete. |
| `voice::doctor` | Backends, model state, open sessions. |

Triggers: `voice::transcript`, `voice::session-started`,
`voice::session-stopped`, `voice::model-progress`. The legacy
`voice::speech-ended` trigger remains registered for compatibility but is no
longer emitted: each browser uses its audio element's ended/error events.

## Limits

- The bundled models are English only. Other languages require a multilingual
  whisper.cpp model or a remote `openai`/`router` backend.
- WAV is the only container decoded. Convert other formats first, for
  example `ffmpeg -i in.m4a -ac 1 -ar 16000 out.wav`.
- Read-aloud on the `host` backend plays on the worker's machine. In a remote
  deployment use the `openai` backend so the audio reaches the browser.
- Prebuilt binaries cover macOS (Intel and Apple silicon) and x86_64 glibc
  Linux. The speech engine publishes no static no-TTS archive for Linux on
  aarch64, musl or 32-bit ARM, and the build only accepts archives whose
  SHA-256 it pins.

## Licenses

The worker is Apache-2.0. [THIRD_PARTY_NOTICES.md](./THIRD_PARTY_NOTICES.md) lists bundled components and the documented child-process and service integrations with their licenses and sources. If you configure a custom child-process command or model, review that artifact and its license separately. In short: sherpa-onnx and its Kaldi-derived libraries are Apache-2.0, ONNX Runtime is MIT, KISS FFT is BSD-3-Clause; the streaming Zipformer model is Apache-2.0 and Parakeet TDT 0.6B v2 is NVIDIA's under CC BY 4.0, with the attribution text in the notices file. The build links the no-TTS archives and drops espeak-ng and its `ucd` tables (GPL-3.0), so no GPL code is in the binary; on Linux, read-aloud runs `espeak-ng` as a separate program when it is installed. Models are downloaded on the user's machine from Hugging Face on first use, and `voice::models::list` reports each model's license, author and source.

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
