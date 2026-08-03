# computer — architecture

One desktop, three ways to reach it, behind one function surface. The worker is
a capability primitive — it captures pixels and moves a cursor. Everything an
agentic computer-use product needs around that (the model loop, approval,
transcripts, tracing) belongs to the engine and the harness, not here.

```text
  agent / console                worker                      the desktop
  ────────────────               ──────                      ───────────
  computer::act  ──────▶  functions/ ─▶ session ─▶ Driver ─┬─▶ native (this machine)
  computer::screenshot                    │                ├─▶ sandbox (microVM)
                                          │                └─▶ remote (guest executor)
  console viewport ◀── computer:frames ◀──┘ screencast pump
  sibling workers  ◀── session-started / session-stopped
```

## Module map

| Module | Role |
|---|---|
| `config.rs` | `WorkerConfig` schema (endpoint, session cap, timeouts, capture limits, sandbox display) and its shared hot-reloadable handle |
| `configuration.rs` | `configuration::register` + the `computer::on-config-change` trigger |
| `driver/mod.rs` | The `Driver` trait: the whole desktop semantic (capture, click, type, keypress, a11y tree, close) |
| `driver/native.rs` | This machine: `xcap` capture + `enigo` input, display pinning, downscale, macOS permission gates |
| `driver/sandbox.rs` | A desktop in an iii-sandbox microVM, driven through `sandbox::exec` / `sandbox::fs` |
| `driver/remote.rs` | A desktop reached through its guest executor over a WebSocket |
| `session.rs` | Session registry, driver selection, durable records in `state`, the screencast pump |
| `events.rs` | `computer::session-started` / `session-stopped` trigger types and their subscriber fan-out |
| `functions/` | The `computer::*` wire surface, one module per function plus the golden-tested catalog |
| `ui.rs` + `ui/` | The injected console page and chat renderer (see the injectable-console-UI SOP) |

## Vocabulary

| Term | Means |
|---|---|
| driver | One implementation of `Driver` — how this session reaches its desktop |
| session | A live desktop plus its id, screen size, and screencast state |
| screen | Desktop pixel dimensions; the coordinate space `act` and screenshots share |
| frame | One screencast capture, pushed onto `computer:frames` under the session id |

## Doc map

| Doc | For |
|---|---|
| [`internals.md`](internals.md) | Changing the worker: driver selection, capture pipeline, durability, permission gates |
| [`integration.md`](integration.md) | Calling the worker: function ids, trigger types, the guardrail split |
| [`../README.md`](../README.md) | Operating the worker: install, quickstart, configuration |
