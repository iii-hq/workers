# Aura — iii On-Device Multimodal AI

Real-time voice and vision conversations powered by local models, wired entirely through iii primitives. No standalone WebSocket server, no REST polling — the **iii-browser-sdk** handles everything: function registration, triggers, channels, and state.

## Architecture

```text
Browser (iii-browser-sdk)          iii Engine               Python Worker (iii-sdk)
─────────────────────────     ─────────────────────     ─────────────────────────
registerWorker(ws://RBAC)  ◄──► iii-worker-manager  ◄──► registerWorker(ws://internal)

ui::aura::transcript       ◄─── trigger(Void)       ◄─── aura::ingest::turn
ui::aura::playback         ◄─── trigger(Void)       ◄─── (TTS chunks via channel)

createChannel() ───────────────► /ws/channels/...   ──► ChannelReader (audio+image)

trigger(aura::session::open) ──► invoke             ──► state::set (session metadata)
trigger(aura::interrupt)    ──► invoke(Void)        ──► asyncio.Event
```

### How it works

1. The **browser** connects as a full iii worker via `iii-browser-sdk` and registers two functions (`ui::aura::transcript`, `ui::aura::playback`) that the backend can invoke.
2. When the user speaks, the browser creates an iii **channel**, sends audio + camera frame as binary data, then triggers `aura::ingest::turn` on the Python worker.
3. The Python worker reads the channel, runs Gemma 4 E2B inference, and pushes results back: transcript text via a **void trigger** and TTS audio via a new **channel**.
4. The browser receives both, updates the UI, and plays audio — all through iii primitives.

### Primitives used

| Primitive | Role |
|-----------|------|
| **iii-browser-sdk** | Browser acts as a full worker — registers functions, triggers, creates channels |
| **iii-worker-manager** (2 ports) | Internal for Python worker, RBAC-filtered for browser |
| **Channels** | Binary audio upload (browser → worker) and TTS playback (worker → browser) |
| **State** (`state::set` / `state::get`) | Session metadata persistence |
| **Triggers** (sync + `TriggerAction.Void()`) | Function invocation and fire-and-forget push |

## Requirements

- Python 3.12+
- macOS with Apple Silicon, or Linux with a supported GPU
- ~3 GB free RAM for Gemma 4 E2B
- Node.js 18+ (for the browser dev server)
- A running iii engine

## Quick Start

### 1. Start the engine

```bash
# From an iii-engine checkout or binary:
iii --config iii-aura/iii-config.example.yaml
```

### 2. Start the Python worker

```bash
cd iii-aura/python-worker
uv sync                   # install dependencies (add --extra mac or --extra linux for TTS)
uv run iii-aura
```

Models download automatically on first run (~2.6 GB for Gemma 4 E2B + TTS models).

### 3. Start the browser app

```bash
cd iii-aura/browser
npm install               # or pnpm install
npm run dev               # or pnpm dev
```

Open http://localhost:5180, grant camera and microphone access, and start talking.

## Configuration

### Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `III_URL` | `ws://127.0.0.1:49134` | Internal worker-manager URL (Python worker) |
| `III_INTERNAL_PORT` | `49134` | Engine internal WS port |
| `III_BROWSER_PORT` | `49135` | Engine RBAC WS port (browser SDK) |
| `MODEL_PATH` | auto-download | Local path to `gemma-4-E2B-it.litertlm` |
| `KOKORO_ONNX` | unset | Set to force ONNX TTS backend on macOS |

### Browser SDK URL

The browser app connects to `ws://localhost:49135` by default. To override, set `window.__III_URL__` before the module loads, or edit `src/aura.ts`.

### Context management

Each voice turn runs in a fresh Gemma conversation to prevent GPU context overflow from accumulated audio + image tokens. This trades multi-turn memory for reliability — every response is independent.

### Queue-based inference (optional)

If sync trigger timeouts are an issue with slow hardware, uncomment the `iii-queue` block in `iii-config.example.yaml` and switch the browser's `aura::ingest::turn` trigger to use `TriggerAction.Enqueue({ queue: 'aura' })`. The Python worker already pushes results back via `Void` triggers, so the completion path is unchanged.

## Function IDs

| Function | Direction | Description |
|----------|-----------|-------------|
| `aura::session::open` | browser → worker | Create a session, returns `session_id` |
| `aura::ingest::turn` | browser → worker | Send audio+image via channel, receive inference result |
| `aura::interrupt` | browser → worker | Cancel in-flight generation |
| `aura::session::close` | browser → worker | Close session and free resources |
| `ui::aura::transcript` | worker → browser | Push transcript text + LLM timing |
| `ui::aura::playback` | worker → browser | Push TTS audio via channel reader ref |

## Extending Aura

Aura is designed as a **default inference worker**, not a monolith. Additional workers can plug into the same engine and extend capabilities without modifying Aura's code.

### Add a new worker

Register new functions under `aura::*` or a sibling namespace (e.g. `aura-tools::*`) from any Python, TypeScript, or Rust worker connected to the same engine port:

```python
from iii import register_worker

iii = register_worker("ws://127.0.0.1:49134")

iii.register_function("aura-tools::summarize", summarize_handler,
    description="Summarize the last N conversation turns")
```

The browser (or Python worker) can then call `aura-tools::summarize` via `trigger()`.

### Share state

Use the `state` module with scoped keys:

- **`aura`** scope: session metadata (managed by the default worker)
- **`aura-settings`** scope: user preferences (e.g. voice, language, system prompt)
- **`aura-history`** scope: conversation transcripts

Any worker that can reach `state::set` / `state::get` on the engine can read or write these scopes.

### Cron / queue triggers

Add periodic tasks (e.g. session cleanup, model health checks) using cron triggers:

```python
iii.register_trigger({
    "type": "cron",
    "function_id": "aura::cleanup::expired-sessions",
    "config": {"expression": "0 0 * * * * *"},  # every hour
})
```

Or queue-based pipelines for heavy background processing:

```python
await iii.trigger_async({
    "function_id": "aura-tools::long-analysis",
    "payload": {"session_id": session_id, "data": large_payload},
    "action": TriggerAction.Enqueue(queue="aura"),
})
```
