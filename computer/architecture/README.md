# computer — architecture

One desktop, three ways to reach it, behind one function surface. The worker is
a capability primitive — it captures pixels and moves a cursor, and on native
sessions it also reads an app's accessibility tree and acts on
its elements by reference. Everything an agentic computer-use product needs
around that (the model loop, approval, transcripts, tracing) belongs to the
engine and the harness, not here.

```text
  agent / console                worker                      the desktop
  ────────────────               ──────                      ───────────
  computer::act  ──────▶  functions/ ─▶ session ─▶ Driver ─┬─▶ native (this machine)
  computer::screenshot                    │                ├─▶ sandbox (microVM)
  computer::observe / find                │                └─▶ remote (guest executor)
                                          ├─▶ semantic/ (native sessions) ─▶ xa11y
  console viewport ◀── frame-changed ◀────┘ screencast pump (newest frame only)
        └────── computer::frame ──────▶ reads the stored frame
  sibling workers  ◀── session-started / session-stopped
```

## Module map

| Module | Role |
|---|---|
| `config.rs` | `WorkerConfig` schema (endpoint, session cap, timeouts, capture limits, sandbox display) and its shared hot-reloadable handle |
| `configuration.rs` | `configuration::register` + the `computer::on-config-change` trigger |
| `driver/mod.rs` | The `Driver` trait: the whole desktop semantic (capture, click, type, keypress, close) |
| `driver/native.rs` | This machine: what the three OSes share — display pinning, downscale + JPEG, image → input coordinates |
| `driver/native/desktop.rs` | macOS/Windows: `xcap` capture + `enigo` input, macOS permission gates |
| `driver/native/x11.rs` | Linux: X11/Wayland detection, RandR monitors, GetImage capture, XTest input, Unicode typing via spare keycodes |
| `driver/semantic_only.rs` | A native session with no coordinates (Wayland): refuses capture and input with the reason |
| `vendor/xa11y-linux/` | xa11y-linux 0.15.2, AT-SPI only, with our patches (`PATCHES.md`) |
| `driver/sandbox.rs` | A desktop in an iii-sandbox microVM, driven through `sandbox::exec` / `sandbox::fs` |
| `driver/remote.rs` | A desktop reached through its guest executor over a WebSocket |
| `session.rs` | Session registry, driver selection, durable records in `state`, the screencast pump |
| `semantic/` | Semantic computer use beside the `Driver` (native sessions only): a per-session executor thread owning the xa11y provider, the active snapshot and its ref table, budgeted walk + redaction, selector validation, tree text, error mapping, the provider factory and, on Linux, the read-only AT-SPI bus probe (`fake.rs` is the Linux test provider) — see internals.md "Semantic layer" |
| `events.rs` | `computer::session-started` / `session-stopped` trigger types and their subscriber fan-out |
| `frames.rs` | `computer::frame-changed` trigger type: bounded bindings and per-binding coalescing delivery |
| `functions/` | The `computer::*` wire surface, one module per function plus the golden-tested catalog |
| `ui.rs` + `ui/` | The injected console page and chat renderer (see the injectable-console-UI SOP) |

## Vocabulary

| Term | Means |
|---|---|
| driver | One implementation of `Driver` — how this session reaches its desktop |
| session | A live desktop plus its id, screen size, and screencast state |
| screen | Desktop pixel dimensions; the coordinate space `act` and screenshots share |
| frame | One screencast capture; each session keeps only its newest one, read with `computer::frame` |

## Doc map

| Doc | For |
|---|---|
| [`internals.md`](internals.md) | Changing the worker: driver selection, capture pipeline, durability, permission gates |
| [`integration.md`](integration.md) | Calling the worker: function ids, trigger types, the guardrail split |
| [`../README.md`](../README.md) | Operating the worker: install, quickstart, configuration |
