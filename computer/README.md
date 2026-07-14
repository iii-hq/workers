# computer

Drive a full desktop on the [iii engine](https://github.com/iii-hq/iii) bus.
Where the [browser](../browser) worker gives an agent a Chromium tab, `computer`
gives it a whole screen: it hands the model a screenshot and clicks, types, and
scrolls by coordinate. The harness discovers `computer::*` as tools
automatically, so the model sees the screen and acts on it with no glue.

The worker does only the one thing no other worker can: capture the screen and
move the cursor. Two backends sit behind one surface:

- **native** (no endpoint): drive the local machine this worker runs on, with
  screen capture and input injection built in. No external server.
- **computer-server** (an endpoint): drive a remote or sandboxed desktop by
  connecting to its in-guest executor over WebSocket. Boot that desktop out of
  band (a sandbox worker owns VM lifecycle); `computer` only connects to it.

Everything else composes with the workers that already exist: run commands and
touch files with the [shell](../shell) worker, persist with `state`, schedule
with `cron`. iii owns the session lifecycle, the bus, durable state, the live
screen stream, and the harness that drives it all.

## What iii adds over a one-shot computer-use client

- **Durable sessions.** Every session is mirrored into `state`; on restart the
  worker reconnects them best-effort, so a crash or redeploy does not lose live
  desktops.
- **A live screen, without polling.** The screencast pump pushes frames onto the
  `computer:frames` stream (`stream::set`), so the console and any number of
  watchers follow the desktop in real time.
- **Reactive lifecycle.** Sibling workers bind `computer::session-started` /
  `computer::session-stopped` instead of polling.
- **Harness-native.** No separate agent loop: the model drives the desktop as
  ordinary tools, and every action is an iii function call with tracing for
  free.

## Install

```bash
iii worker add computer
```

`iii worker add` fetches the binary and writes a config block into
`~/.iii/config.yaml`; the engine starts the worker on the next `iii start`. With
no endpoint it drives the local machine (native). On macOS grant the worker
process Screen Recording (capture) and Accessibility (input) in System Settings,
or capture is black and input is ignored.

## Quickstart

Start a session on the local machine, look at the screen, and act on it:

```rust
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, InitOptions};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    // No endpoint: drive the machine this worker runs on (native backend).
    let started = iii.trigger(TriggerRequest {
        function_id: "computer::sessions::start".into(),
        payload: json!({}),
        action: None,
        timeout_ms: Some(30_000),
    }).await?;
    let session_id = started["session_id"].as_str().unwrap();

    // See the screen: an image block the vision model renders inline.
    iii.trigger(TriggerRequest {
        function_id: "computer::screenshot".into(),
        payload: json!({ "session_id": session_id }),
        action: None,
        timeout_ms: Some(15_000),
    }).await?;

    // Act by coordinate (pixels read off that screenshot, top-left origin).
    iii.trigger(TriggerRequest {
        function_id: "computer::act".into(),
        payload: json!({ "session_id": session_id, "action": "click", "x": 640, "y": 360 }),
        action: None,
        timeout_ms: Some(10_000),
    }).await?;

    Ok(())
}
```

To drive a remote or sandboxed desktop instead, pass an `endpoint` pointing at
its computer-server: `json!({ "endpoint": "http://host:8000" })`.

The rest of the surface: `computer::act` (click / right_click / double_click /
move / drag / scroll / type / press / hotkey, by coordinate), `computer::observe`
(screenshot plus the accessibility tree on macOS), and
`computer::sessions::list` / `computer::sessions::stop`. For running commands or
reading and writing files on the desktop, use the [shell](../shell) worker.
Function ids and schemas live in the code and `iii worker info computer`.

## Configuration

Stored in the `configuration` worker under the `computer` key; every field is
editable live from the console. Timeouts and the screencast rate hot-reload;
`default_endpoint` and `os` apply to sessions started after the change.

```yaml
computer:
  default_endpoint: ''      # computer-server endpoint for a remote desktop; empty = drive the local machine (native)
  os: linux                 # guest OS label recorded on sessions
  max_sessions: 2           # concurrent desktop connections
  idle_stop_ms: 300000      # stop sessions idle this long; 0 disables
  screencast_fps: 15        # live-view frame rate cap
  max_screenshot_dimension: 1280 # downscale screenshots/frames to this longest edge (native backend)
  screenshot_quality: 70    # JPEG quality 1-100 (native backend)
  command_timeout_ms: 120000 # timeout for each backend action (fixed at connect)
  connect_timeout_ms: 15000  # backend connect timeout at session start
```

## Custom trigger types

Sibling workers (and the console UI) can subscribe to session activity. Both
bindings accept an optional `{ "session_id": "..." }` filter.

| Trigger type | Fires when | Payload to subscribers |
|---|---|---|
| `computer::session-started` | A session connected and is ready | `{ session_id, endpoint, os, screen, timestamp }` |
| `computer::session-stopped` | A session ended | `{ session_id, reason: "stopped" \| "idle", timestamp }` |

## Live screen

`computer::screencast::start` / `stop` and `computer::frame` drive the live
viewport: the worker polls the backend at `screencast_fps` and pushes the newest
frame onto the `computer:frames` stream (one item per session). These three are
internal console-UI plumbing, not agent surface, and stay out of agent tool
lists.
