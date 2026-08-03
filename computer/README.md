# computer

Drive a full desktop on the iii bus. Where the
[browser](https://github.com/iii-hq/workers/tree/main/browser) worker gives an
agent a Chromium tab, `computer` gives it a whole screen: it hands the model a
screenshot and clicks, types, and scrolls by coordinate. The harness discovers
`computer::*` as functions automatically, so a model sees the screen and acts on
it with no glue, and the console gets a live viewport of whatever the desktop is
doing.

The worker does the one thing no other worker can: capture the screen and move
the cursor. Three drivers sit behind one surface, picked per session:

- **native** (no `endpoint`, no `image`): drive the local machine this worker
  runs on, capture and input injection built in.
- **sandbox** (an `image`): boot a fresh desktop inside an
  [iii-sandbox](https://github.com/iii-hq/workers/tree/main/iii-sandbox) microVM
  and drive it through iii primitives alone (`sandbox::exec` / `sandbox::fs`),
  with no socket into the guest. A fixed virtual display means 1:1 coordinates,
  no HiDPI or multi-monitor ambiguity, and nothing to grant on the host.
- **remote** (an `endpoint`): drive an already-running desktop through the
  executor inside it, over a WebSocket. `computer` connects; that desktop boots
  out of band.

Everything else composes with workers that already exist: run commands and touch
files with the [shell](https://github.com/iii-hq/workers/tree/main/shell)
worker, persist with `state`, schedule with `cron`.

## Install

```bash
iii worker add computer
```

To drive a throwaway desktop instead of your own machine, add the sandbox worker
too — it boots the microVM the `image` driver runs in:

```bash
iii worker add iii-sandbox
```

**macOS, native driver only.** Capture and input are permission-gated and macOS
degrades both silently, so the worker checks and fails loud. Grant the app that
runs the worker **Screen Recording** (without it every screenshot is the
wallpaper with all windows stripped) and **Accessibility** (without it clicks and
typing are dropped) in System Settings > Privacy & Security, then restart it. The
sandbox driver needs neither.

## Quickstart

Start a session, look at the screen, click what you see:

```bash
# drive this machine (native driver)
iii trigger computer::sessions::start --json '{}'
# -> { "session_id": "c1", "endpoint": "native", "os": "macos", "screen": { "width": 1512, "height": 945 } }

# see the screen: an image block the vision model renders inline
iii trigger computer::screenshot --json '{"session_id":"c1"}'

# click a pixel read off that screenshot (top-left origin)
iii trigger computer::act --json '{"session_id":"c1","action":"click","x":640,"y":360}'

iii trigger computer::sessions::stop --json '{"session_id":"c1"}'
```

Swap the first call for `{"image":"desktop"}` to boot a sandboxed Linux desktop
instead (see [Sandbox desktop image](#sandbox-desktop-image)), or
`{"endpoint":"ws://host:8000"}` to drive a remote one. Everything after that call
is identical: the session id is the only handle.

The same from a sibling worker:

```rust
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, InitOptions};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let started = iii
        .trigger(TriggerRequest {
            function_id: "computer::sessions::start".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(30_000),
        })
        .await?;
    let session_id = started["session_id"].as_str().unwrap();

    iii.trigger(TriggerRequest {
        function_id: "computer::act".into(),
        payload: json!({ "session_id": session_id, "action": "click", "x": 640, "y": 360 }),
        action: None,
        timeout_ms: Some(10_000),
    })
    .await?;

    Ok(())
}
```

The rest of the surface: `computer::act` — pointer actions by coordinate
(click, right_click, double_click, move, drag, scroll) and keyboard actions
that go wherever focus is (`type` with `text`, `press` / `hotkey` with `keys`,
e.g. `["cmd","c"]`) —
`computer::observe` (screenshot plus the accessibility tree on macOS guests),
`computer::displays`, `computer::sessions::list` and `computer::sessions::stop`.
For running commands or reading and writing files on the desktop, use the
[shell](https://github.com/iii-hq/workers/tree/main/shell) worker.

## Console

The worker ships its own console page (`#/ext/computer`): a session rail, a live
viewport fed by the screencast stream, and click / type / scroll forwarding
straight into the desktop. It is injected into any running console at
registration time — nothing to install, nothing to rebuild. Every `computer::*`
call in chat and traces renders through the same asset.

## Configuration

Stored in the `configuration` worker under the `computer` key; every field is
editable live from the console. The screencast rate applies to running
sessions; everything a driver is built with — endpoint, OS label, timeouts,
capture limits, and the sandbox display and network settings — is read at
`sessions::start`, so a change reaches sessions started after it.

```yaml
computer:
  default_endpoint: ''      # guest-executor endpoint for a remote desktop; empty = drive the local machine
  os: linux                 # guest OS label recorded on sessions
  max_sessions: 2           # concurrent desktop connections
  idle_stop_ms: 300000      # stop sessions idle this long; 0 disables
  screencast_fps: 15        # live-view frame rate cap
  max_screenshot_dimension: 1280 # downscale screenshots/frames to this longest edge (native)
  screenshot_quality: 70    # JPEG quality 1-100 (native)
  command_timeout_ms: 120000 # timeout for each driver action (fixed at connect)
  connect_timeout_ms: 15000  # driver connect timeout at session start
  sandbox_image: ''          # iii-sandbox image for a sandbox session; empty = name it per call
  sandbox_width: 1280        # sandbox virtual display width
  sandbox_height: 800        # sandbox virtual display height
  sandbox_network: true      # give the sandboxed desktop network access; false keeps it offline
  sandbox_idle_timeout_secs: 86400 # idle_timeout_secs for sandbox::create; kept high, the worker owns teardown
  screen_capture_preflight: true # macOS: ask for Screen Recording at native start, fail loud if denied
```

## Sandbox desktop image

The sandbox driver needs a desktop inside the guest (Xvfb, xdotool, imagemagick,
openbox). A prebaked image lives in [`images/desktop`](images/desktop): build it,
push it, register it as a `custom_images` entry on the iii-sandbox worker, then
start a session with `{ "image": "desktop" }`. iii-sandbox boots on macOS Apple
Silicon (libkrun) and Linux (`/dev/kvm`).

## Custom trigger types

Sibling workers (and the console page) subscribe to session activity instead of
polling. Both bindings accept an optional `{ "session_id": "..." }` filter.

| Trigger type | Fires when | Payload to subscribers |
|---|---|---|
| `computer::session-started` | A session connected and is ready | `{ session_id, endpoint, os, screen, timestamp }` |
| `computer::session-stopped` | A session ended | `{ session_id, reason: "stopped" \| "idle", timestamp }` |

## Live screen

`computer::screencast::start` / `stop` and `computer::frame` drive the live
viewport: the worker captures at `screencast_fps` and pushes the newest frame
onto the `computer:frames` stream (one item per session), so the console and any
number of watchers follow the desktop without polling. These three are internal
console plumbing rather than agent surface, and stay out of agent function
lists.
