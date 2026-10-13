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
- Input maps back through the display's size and global origin as `xcap`
  reports them — **logical points on macOS, physical pixels on Windows** (the
  process is per-monitor-v2 DPI aware) — the space `enigo` absolute
  coordinates use, so a click on a scaled display lands where the model saw
  it. Never multiply by `scale_factor()`.

The sandbox driver sidesteps all of it with one fixed virtual display: 1:1
coordinates, no HiDPI, no multi-monitor ambiguity.

## Semantic layer

`src/semantic/` sits beside the `Driver`. Each native session owns a
`SemanticSession`: one executor thread that alone builds and uses xa11y
providers (COM is initialised there on Windows). Every snapshot not scoped by
`root_ref` creates a fresh provider (a `root_ref` snapshot reuses the active
one's, whose handle cache holds that element, and keeps that root's ref so a
scoped find can be repeated to confirm an act). An observe with `include_a11y`
always becomes the session's **active snapshot**; a find does only when it
matches, so a find without matches keeps the old refs. Replacing the snapshot
drops its provider, which frees xa11y's handle cache and kills the old refs.
A coordinate or keyboard action bumps an invalidation counter, so refs read
before it are refused.

The executor runs one operation per session, taken with `try_lock`: an
observe/find/act that meets another operation gets `busy` (find retries it
within `timeout_ms`), never a queue. Acts also hold the session's `act_lock`,
so a ref act waits behind a running coordinate or ref act instead of failing.
A call whose deadline (walk 5 s, act 10 s, +1 s grace) passes, or whose caller
cancels it, is never dispatched later: its executor is discarded along with
the active snapshot, and the next call starts a fresh thread and provider.

Observe walks breadth-first; find lets xa11y's matcher drive the same
memoized, budgeted children closure (real providers mint a fresh handle on
every fetch, so children are fetched once per parent). Walks are bounded by
node count, absolute depth and a 5 s deadline. Secret values are dropped
before the selector sees them (macOS `AXSecureTextField`; a password-like
name everywhere), and selectors are restricted to normalized attribute names,
because xa11y resolves unknown names against the unredacted `raw` map. On
Windows the process is pinned to per-monitor-v2 DPI awareness before any
capture, input or accessibility call.

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

The semantic layer is tested on Linux against `semantic::fake::FakeTree`, a
provider the test shapes. `tests/semantic_live.rs` runs the same layer
against a real AccessKit window (`tests/fixture`); CI runs it on Windows.
On a Mac, from `computer/` in a terminal with Accessibility and Screen
Recording:

    cargo build --locked --manifest-path tests/fixture/Cargo.toml
    COMPUTER_A11Y_FIXTURE=$PWD/tests/fixture/target/debug/computer-a11y-fixture \
      cargo test --locked --test semantic_live -- --ignored --test-threads=1

Then, on a live stack with the harness and approval-gate, have an agent run
`computer::find` and `computer::act` against the fixture or TextEdit: check
the approval card, an act approved after several seconds (refs stay valid),
the result reaching the model, and `observe` without image in the console.
Without Accessibility, `find` must fail with `permission_denied`, never an
empty tree.
