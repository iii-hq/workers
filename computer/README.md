# computer

Drive a full desktop on the iii bus. Where the
[browser](https://github.com/iii-hq/workers/tree/main/browser) worker gives an
agent a Chromium tab, `computer` gives it a whole screen: it hands the model a
screenshot, clicks and scrolls at the coordinates it reads off that image, and
types wherever the desktop's focus is. The harness discovers
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
files with the [shell](https://github.com/iii-hq/workers/tree/main/ide)
worker, persist with `state`, schedule with `cron`.

## Install

```bash
iii trigger compose::add worker=computer
```

To drive a throwaway desktop instead of your own machine, turn on the engine's
sandbox worker: it boots the microVM the `image` driver runs in. `iii-sandbox`
is built into the engine, so Compose cannot add it as a container
(`compose::add` refuses it). In a Compose project it goes under
`engine.workers` in `worker-compose.yaml`:

```yaml
engine:
  workers:
    iii-sandbox:
      auto_install: true
      image_allowlist: [desktop]
      custom_images:
        desktop: ghcr.io/trycua/linux:24.04-slim-20261013-36bc520
```

With an engine you run yourself (`iii -c config.yaml`), the same settings are a
list entry in its `config.yaml` (the engine reloads it live):

```yaml
workers:
  - name: iii-sandbox
    config:
      auto_install: true
      image_allowlist: [desktop]
      custom_images:
        desktop: ghcr.io/trycua/linux:24.04-slim-20261013-36bc520
```

`sandbox::catalog::list` shows what it can boot. For the image, see
[Sandbox desktop image](#sandbox-desktop-image).

**macOS, native driver only.** Capture and input are permission-gated and macOS
degrades both silently, so the worker checks and fails loud. Grant the app that
runs the worker **Screen Recording** (without it every screenshot is the
wallpaper with all windows stripped) and **Accessibility** (without it clicks and
typing are dropped) in System Settings > Privacy & Security, then restart it. The
sandbox driver needs neither.

**Linux, native driver.** On an X11 session (Xorg or Xvfb) a native session
captures and acts by coordinate like macOS/Windows; coordinates are physical
root pixels. Run a window manager: on a bare X server a click gives no keyboard
focus, so toolkits that wait for focus (egui, Bevy, Slint) ignore typed keys.
On a Wayland desktop (GNOME and KDE default to it) the session is
semantic-only: `sessions::start` reports `coordinates: { supported: false }`,
`screen` is 0x0, screenshots and coordinate actions are refused with
`unsupported_session`, and you read and act by ref. Ref actions need the
AT-SPI bus; when accessibility is off, `semantic.reason` carries a warning
with the command that turns it on for the login session (`gdbus call … IsEnabled
'<true>'`). The worker never runs it itself: it also persists
`toolkit-accessibility=true` in dconf. Chromium/Electron apps also need
`--force-renderer-accessibility`.

## Quickstart

Start a session, look at the screen, click what you see:

```bash
# drive this machine: a bare start picks the native driver when neither
# sandbox_image nor default_endpoint is configured; '{"native": true}' always does
iii trigger computer::sessions::start --json '{}'
# -> { "session_id": "c1", "endpoint": "native", "os": "macos", "screen": { "width": 1512, "height": 945 }, "semantic": { "supported": true } }

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

The rest of the surface: `computer::act` — pointer actions addressed by
coordinate (click, right_click, double_click, move, drag, scroll) and keyboard
actions that land wherever the desktop's focus is (`type` with `text`,
`press` / `hotkey` with `keys`, e.g. `["cmd","c"]`) —
`computer::observe` (a screenshot and, with `include_a11y`, an app's
accessibility tree), `computer::find`, `computer::displays`,
`computer::sessions::list` and `computer::sessions::stop`.

For commands and files, mind which machine you mean: on a native session the
desktop IS this host, so the
[shell](https://github.com/iii-hq/workers/tree/main/ide) worker is the right
tool. A sandboxed desktop is reached with `sandbox::exec` / `sandbox::fs`, and
a remote one through its own guest executor — `shell` would run on the host
instead, which is a different machine entirely.

On a native session the worker also acts **by element**:
`computer::observe { include_a11y: true, include_image: false }` or
`computer::find { selector: "button[name='Save']" }` return lines carrying
`[ref=…]`, and `computer::act { action: "invoke" | "set_value" | "toggle" |
"select", ref }` acts on that element through the OS accessibility API — the
worker moves no mouse, sends no keys and sets no focus. The effect is not
verified; confirm with another `find`. Refs live until an `observe` with
`include_a11y` or a `find` with matches replaces them, any coordinate or
keyboard action (`type`, `press`, `hotkey`, console input included) runs on
any native session (they share the desktop), a timed-out, cancelled or
crashed accessibility call resets the session's executor, or the session
stops or the worker restarts. macOS needs Accessibility **and** Screen
Recording for the process running the worker. On Windows a password field's
value is hidden only when its name looks like a password (or the app itself
masks it): xa11y does not expose UIA's `IsPassword`. On Linux a password field
is recognised by its AT-SPI role, whatever its name. Remote and sandbox
sessions stay coordinate-only.

`computer::run { app_name, goal, inputs }` drives one app toward a goal in a
single call: each step, the [judge](https://github.com/iii-hq/workers/tree/main/judge)
worker picks the next element and operation from the tree, the worker picks
the verb and acts by ref, and the run returns a status, its steps and the
tree with live refs. Text comes only from `inputs`, keyed by field label.
Without a judge worker it returns `judge_unavailable` and the tree; with a
dialog already open it returns `blocked` and touches nothing. The judge
it asks is `judge_provider` in this worker's configuration when set, else the
calling session's pick, else the judge hub's default.

## Console

The worker ships its own console page (page `computer`): a session rail, a live
viewport fed by the live screencast, and click / type / scroll forwarding
straight into the desktop. With no desktop open (or after + in the rail) the
page shows a launcher: this machine, a sandbox picked from iii-sandbox's
catalog, or a remote endpoint. It is injected into any running console at
registration time — nothing to install, nothing to rebuild. Every `computer::*`
call in chat and traces renders through the same asset.

When an agent drives a session (`computer::act`, `computer::run`), a live
preview of it floats in a corner of the console, like the browser's and the
iOS simulator's: drag it, pinch or double-click to resize, tap to fan several
out, expand to open the page. A hidden card stays hidden while the agent keeps
acting, until two quiet minutes pass or a new `computer::run` starts. Nothing
pops for a session the page already shows in that window, or one without a
picture (Wayland).

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
  max_screenshot_dimension: 1280 # downscale screenshots/frames to this longest edge (native, remote; a sandbox is its display size)
  screenshot_quality: 70    # JPEG quality 1-100 (native, sandbox, remote)
  command_timeout_ms: 120000 # timeout for each driver action (fixed at connect)
  connect_timeout_ms: 15000  # driver connect timeout at session start
  sandbox_image: ''          # a name in iii-sandbox's catalog (custom_images) a bare sessions::start boots; empty = name it per call
  sandbox_width: 1280        # sandbox virtual display width
  sandbox_height: 800        # sandbox virtual display height
  sandbox_network: true      # give the sandboxed desktop network access; false keeps it offline
  sandbox_memory_mb: 2048    # RAM of the sandbox VM; 0 = iii-sandbox's default (512); a browser needs ~2048
  sandbox_cpus: 2            # vCPUs of the sandbox VM; 0 = iii-sandbox's default (1)
  sandbox_idle_timeout_secs: 86400 # idle_timeout_secs for sandbox::create; kept high, the worker owns teardown
  screen_capture_preflight: true # macOS: ask for Screen Recording at native start, fail loud if denied
  judge_provider: ''         # judge-<provider> computer::run asks (clef, decider, typesafe…); empty = the session's pick, then the judge default
```

## Sandbox desktop image

The sandbox driver runs `Xvfb`, `xdpyinfo`, `setsid`, `pgrep`, `xdotool`,
ImageMagick's `import` and `base64` inside the guest, then the image's own
desktop, the first it has: XFCE (`startxfce4`), mutter with a `tint2` panel, or
`openbox`. An image with none shows a black screen and frameless windows. Any
image with those tools, for arm64 (Apple Silicon) and amd64, works as is:

- **Ready to use, nothing to build:** [`ghcr.io/trycua/linux:24.04-slim-20261013-36bc520`](https://github.com/trycua/cua)
  (MIT, about 490 MB, both architectures; pin a dated tag, `24.04-slim` moves).
  It boots into XFCE (panel, dock with a terminal, files and a browser).
  Verified with a sandbox session that opens a window and closes it by a click.
- **With a browser:** [`ghcr.io/anthropics/anthropic-quickstarts:computer-use-demo-latest`](https://github.com/anthropics/claude-quickstarts/tree/main/computer-use-demo)
  (MIT, about 2.9 GB unpacked) adds Firefox, LibreOffice and xcalc, and boots
  into mutter with its own tint2 launcher panel. Firefox is OOM-killed at iii-sandbox's 512 MB: keep
  `sandbox_memory_mb` at 2048 or more. Verified on Apple Silicon with xcalc
  driven by clicks and Firefox navigated by typing a URL; the first boot pulls
  the image (about 90 s), later ones take under a second.

Register the image as the `desktop` entry of `custom_images` (see
[Install](#install)), then start a session with `{ "image": "desktop" }`.
In the console, the computer worker's settings (Sandbox desktop) pick the
default image among iii-sandbox's `custom_images`, shown as `name — OCI ref`,
and size the VM (memory, CPUs); a new OCI image goes into `custom_images`
first, since iii-sandbox only boots names from its catalog.
iii-sandbox boots on macOS Apple Silicon (libkrun) and Linux (`/dev/kvm`).

## Custom trigger types

Sibling workers (and the console page) subscribe to session activity instead of
polling. Every binding accepts an optional `{ "session_id": "..." }` filter.

| Trigger type | Fires when | Payload to subscribers |
|---|---|---|
| `computer::session-started` | A session connected and is ready | `{ session_id, endpoint, os, screen, timestamp }` |
| `computer::session-stopped` | A session ended | `{ session_id, reason: "stopped" \| "idle", timestamp }` |
| `computer::activity` | A `computer::act` call reached a session, or a `computer::run` started on it | `{ session_id, function, timestamp }` |

## Live screen

`computer::screencast::start` / `stop`, `computer::frame` and the
`computer::frame-changed` trigger type drive the live viewport. The worker
captures at `screencast_fps` and keeps only the newest frame per session in
memory (no history). After each frame it fires `computer::frame-changed` with a
small, image-free notification; the viewer reads the frame with
`computer::frame` (notify, then fetch), so the console and any number of
watchers follow the desktop without polling, and a slow watcher skips frames
instead of queueing them.

Each viewer (the page, a corner preview, in any console window) calls `start`
when it opens and `stop` when it closes; the capture runs while one remains,
and one nobody follows any more (a window closed without a `stop`) stops by
itself after 10 seconds.

| Trigger type | Fires when | Payload to subscribers |
|---|---|---|
| `computer::frame-changed` | A new frame was stored (`change: "updated"`), or the frame was dropped because the screencast or session stopped (`change: "cleared"`) | `{ session_id, epoch, frame_seq, change, width, height, mime?, bytes?, timestamp }` |

The binding config `{ "session_id": "..." }` is **required** (a viewer follows
one desktop). Order frames by `(epoch, frame_seq)`; `epoch` changes when the
worker restarts or restores the session. Bind first, then read
`computer::frame` once for the current frame; read it again after each
notification (pass `since_frame` to skip an unchanged image) and after a
reconnect. These are internal console plumbing rather than agent surface, and
stay out of agent function lists.
