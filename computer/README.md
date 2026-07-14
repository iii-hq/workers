# computer

Drive a full desktop on the [iii engine](https://github.com/iii-hq/iii) bus.
Where the [browser](../browser) worker gives an agent a Chromium tab, `computer`
gives it a whole screen: it connects to a computer-use session, hands the model
a screenshot, and clicks, types, scrolls, and runs shell inside the desktop by
coordinate. The harness discovers `computer::*` as tools automatically, so the
model sees the screen and acts on it with no glue.

The desktop itself lives outside iii (booting real macOS/Windows/Linux desktops
is deep systems work). `computer` connects to a **computer-server** endpoint,
the in-guest executor that open desktop-sandbox stacks (for example a
[Cua](https://github.com/trycua/cua) sandbox) already expose. iii owns the part
it is good at: the session lifecycle, the bus, durable state, the live screen
stream, and the harness that drives it all.

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
`~/.iii/config.yaml`; the engine starts the worker on the next `iii start`.
Point `default_endpoint` at a running computer-server (see Configuration) or
pass an `endpoint` per session.

## Quickstart

Start a session, look at the screen, act on it, then read a file back:

```rust
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, InitOptions};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    // Connect to a computer-server backend (a booted desktop sandbox).
    let started = iii.trigger(TriggerRequest {
        function_id: "computer::sessions::start".into(),
        payload: json!({ "endpoint": "http://127.0.0.1:8000", "os": "linux" }),
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

The rest of the surface: `computer::act` (click / right_click / double_click /
move / drag / scroll / type / press / hotkey, by coordinate), `computer::observe`
(screenshot plus the accessibility tree on macOS guests), `computer::shell`
(run a command in the guest), `computer::files::read` / `computer::files::write`,
and `computer::sessions::list` / `computer::sessions::stop`. Function ids and
schemas live in the code and `iii worker info computer`.

## Configuration

Stored in the `configuration` worker under the `computer` key; every field is
editable live from the console. Timeouts and the screencast rate hot-reload;
`default_endpoint` and `os` apply to sessions started after the change.

```yaml
computer:
  default_endpoint: ''      # computer-server endpoint (ws/http/host:port); empty = pass per session
  os: linux                 # guest OS label recorded on sessions
  max_sessions: 2           # concurrent desktop connections
  idle_stop_ms: 300000      # stop sessions idle this long; 0 disables
  screencast_fps: 15        # live-view frame rate cap
  default_timeout_ms: 30000 # action default when the caller omits timeout_ms
  max_timeout_ms: 120000    # ceiling; caller timeout_ms clamped DOWN to this
  connect_timeout_ms: 15000 # backend connect timeout at session start
```

## Custom trigger types

Sibling workers (and the console UI) can subscribe to session activity. Both
bindings accept an optional `{ "session_id": "..." }` filter.

| Trigger type | Fires when | Payload to subscribers |
|---|---|---|
| `computer::session-started` | A session connected and is ready | `{ session_id, endpoint, os, screen, timestamp }` |
| `computer::session-stopped` | A session ended | `{ session_id, reason: "stopped" \| "idle" \| "crashed", timestamp }` |

## Live screen

`computer::screencast::start` / `stop` and `computer::frame` drive the live
viewport: the worker polls the backend at `screencast_fps` and pushes the newest
frame onto the `computer:frames` stream (one item per session). These three are
internal console-UI plumbing, not agent surface, and stay out of agent tool
lists.
