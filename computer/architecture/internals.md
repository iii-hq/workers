# computer — internals

For changing the worker. Consumers want [`integration.md`](integration.md).

## Driver selection

`sessions::start` resolves exactly one driver, in this order (`session.rs`):

1. **`image`** (argument, else the configured `sandbox_image`) — boot a desktop
   in an iii-sandbox microVM. Endpoint label: `sandbox:<sandbox_id>`, guest OS
   `linux`.
2. **`endpoint`** (argument, else the configured `default_endpoint`) — connect
   to the desktop's guest executor. Endpoint label: the normalized url.
3. **neither** — the native driver, this machine. Endpoint label: `native`.

An image beats an endpoint deliberately: a caller who names an image wants a
fresh desktop, not whatever the operator configured as a default target.

Selection ends with a `screen_size` probe. A driver that connects but cannot
report a screen is not a session — the start fails there instead of handing
back an id that breaks on first use.

## Capture pipeline

`Shot` carries encoded bytes plus the mime **detected from the magic bytes**,
never assumed: a driver returns whatever its guest encodes, and the content
block advertises the truth to the model.

The native driver does the work no guest does for it:

- One display per session, chosen at first capture (the display under the
  cursor unless `monitor` names one) and then pinned by id, so coordinates stay
  stable for the life of the session.
- Downscale to `max_screenshot_dimension` and JPEG-encode at
  `screenshot_quality`. A full Retina frame is tens of megabytes of PNG; that
  floods both the model context and the live viewport.
- Input maps back through the display's **logical point** size and global
  origin — the space `enigo` absolute coordinates use — so a click on a scaled
  display lands where the model saw it.

The sandbox driver sidesteps all of it with one fixed virtual display: 1:1
coordinates, no HiDPI, no multi-monitor ambiguity.

## Durability and the screencast

Sessions are mirrored into `state` (scope `computer_sessions`) on start and
deleted on stop; `Sessions::restore` reconnects them at boot, so a worker
restart usually costs nothing. It is best-effort by design: a desktop that went
away in the meantime fails to reconnect and its record is dropped, and restore
never displaces a session that started while it was running. Callers should be
ready to start a new session rather than assume an id survives.

The screencast pump is one task per session. It captures at `screencast_fps`
and stores each frame in the session's single `latest_frame` slot, replacing
(and freeing) the previous one: resident frame memory is at most one frame per
live session (`max_sessions` caps sessions), and there is no frame history.
Only AFTER the slot holds the new frame does it fire `computer::frame-changed`
(`frames.rs`), so a viewer that reads `computer::frame` on a notification
always finds that frame or a newer one. The notification is ~200 bytes and
carries no image on purpose: frames are 60-250 KB of JPEG (native, sandbox) up
to 0.5-2 MB of PNG (remote executors), 15 times a second, and copying them
into every binding's payload would turn a slow viewer into a backlog.

`frames.rs` keeps the fan-out bounded: at most 64 bindings, a required
`session_id` filter, and per binding one coalescing slot (one delivery in
flight, one pending notification; newer replaces pending). Delivery is a
synchronous call with a 5 s timeout, so a slow consumer keeps its slot busy
and only loses intermediate notifications; the pump never waits on a viewer.
Each binding's `namespace` and `metadata` are forwarded unchanged.

`epoch` (ms when this process created or restored the session object) orders
frames across worker restarts, where `frame_seq` restarts at 1. A capture
failure stops the pump rather than looping on a broken driver;
`stop_screencast` (also run by session stop, idle stop and worker shutdown)
and a capture failure clear the slot and fire `change: "cleared"`, so a
stopped session never leaves a multi-megabyte image resident.

## macOS permission gates

macOS degrades both capabilities silently, which is worse than failing:

- **Screen Recording** missing → capture returns wallpaper and menu bar with
  every window stripped. Looks like a screenshot, shows nothing.
- **Accessibility** missing → synthetic input is dropped while the input
  library still reports success, so `act` would claim a click that never landed.

So the worker checks. Input is preflighted with `AXIsProcessTrusted`. Capture
calls `CGRequestScreenCaptureAccess` — the *request* API, which surfaces the
system prompt — and fails loud, gated by `screen_capture_preflight` for setups
where the grant is already in place. Do **not** switch that to
`CGPreflightScreenCaptureAccess`: it reports per-process state a child of a
granted terminal does not inherit, so it false-negatives on capture that works.
The grant is per binary, so a rebuild loses it.

## Tests

`tests/schemas.rs` is the wire contract: a golden snapshot per function, plus
an assertion that no function publishes an untyped schema. Regenerate
deliberately with `UPDATE_GOLDENS=1 cargo test` — a diff there is a change to
what every agent sees. `src/ui.rs` tests assert the embedded console assets are
present and scoped.
