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
  floods both the model context and the frame stream.
- Input maps back through the display's **logical point** size and global
  origin — the space `enigo` absolute coordinates use — so a click on a scaled
  display lands where the model saw it.

The sandbox driver sidesteps all of it with one fixed virtual display: 1:1
coordinates, no HiDPI, no multi-monitor ambiguity.

## Durability and the screencast

Sessions are mirrored into `state` (scope `computer_sessions`) on start and
deleted on stop; `Sessions::restore` reconnects them best-effort at boot, so a
worker restart does not lose live desktops.

The screencast pump is one task per session. It captures at
`screencast_fps`, pushes each frame onto the `computer:frames` stream
(`stream::set`, group = session id, one item), and keeps the newest frame in
memory for `computer::frame`. Both writes matter: the stream is how the console
follows without polling, the in-memory copy is how a late subscriber paints
immediately. A capture failure stops the pump rather than looping on a broken
driver, and `stop_screencast` clears both the stream item and the buffer so a
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
