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
- Input maps back through the display's size and global origin as the OS half
  reports them — **logical points on macOS, physical pixels on Windows** (the
  process is per-monitor-v2 DPI aware) **and on Linux** (X11 root pixels:
  RandR, GetImage and XTest share them) — the space that OS's input injection
  uses, so a click on a scaled display lands where the model saw it. Never
  multiply by a scale factor, and never click by AT-SPI bounds (GTK4 reports
  (0,0) for every element).
- On macOS every move waits until the OS reports the pointer at the target:
  a synthetic move lands asynchronously, while `enigo` 0.3 posts a button at
  the position it reads back (and a scroll carries the current one), so an
  immediate press or scroll hits where the pointer was.

### Linux

`detect()` decides once per session: `WAYLAND_DISPLAY` set, or an X server
that advertises `XWAYLAND`, is a Wayland desktop (semantic-only driver); a
plain X server gets the X11 driver; no display refuses the session. The X11
driver keeps one connection per session, and every X call is bounded by 10 s,
so a hung X server fails the call instead of wedging the session. Text is
typed as keysyms; a character missing from the keyboard map is bound to a
spare keycode for that call (per segment, as many distinct characters as
there are free keycodes), with a 150 ms settle before typing and before
restoring the map. A client that re-reads the map after the restore drops that
character (xdotool's race), and the map is server-global: another client can
compete for the same free keycodes. `type_text` turns Caps Lock off for the
duration of the call and back on afterwards (it refuses when it cannot). Every
press is released by a guard, error or not.

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
Every native session drives the same desktop, so all of them share one
invalidation counter and one `act_lock`. A coordinate or keyboard action on
any of them bumps the counter before the input and again when it returns,
fails or is cancelled (a drop guard), so refs read before or during it are
refused; the next semantic call drops the invalidated snapshot and its
provider.

The executor runs one operation per session, taken with `try_lock`: an
observe/find/act that meets another operation gets `busy` (find retries it
within `timeout_ms`), never a queue. Acts also hold the desktop's `act_lock`,
so a ref act waits behind a running coordinate or ref act instead of failing.
A call whose deadline (walk 5 s, act 10 s, +1 s grace) passes, or whose caller
cancels it, is never dispatched later: its executor is discarded along with
the active snapshot, and the next call starts a fresh thread and provider.

Observe walks breadth-first; find lets xa11y's matcher drive the same
memoized, budgeted children closure (real providers mint a fresh handle on
every fetch, so children are fetched once per parent). Walks are bounded by
node count, absolute depth and a 5 s deadline. Secret values are dropped
before the selector sees them (macOS `AXSecureTextField`, or a macOS element
whose `AXSubrole` read went unanswered, the AT-SPI password role, and a
password-like name everywhere; on Linux the vendored provider never even reads
a password field's text), and selectors are restricted to normalized attribute
names, because xa11y resolves unknown names against the unredacted `raw` map.
Before a write the macOS provider (vendored, `vendor/xa11y-macos/PATCHES.md`)
re-reads the element's `AXRole`, refuses `select` on an element that exposed
no `AXSelected` and `set_value` where `AXValue` is not settable; the engine
refuses `set_value` on a text field observed read-only (Windows, Linux). On
Windows the process is pinned to per-monitor-v2 DPI awareness before any
capture, input or accessibility call.

## Known limits

- **macOS writes AppKit drops.** AppKit answers success to an `AXSelected` or
  `AXValue` write the element refuses, and settability is true for any class
  that overrides the setter, AccessKit's included. So on macOS an AccessKit
  read-only field, or an item that is not selectable, still answers
  `dispatched … effect not verified` with nothing sent; `find` is the check.
  The fix belongs upstream in accesskit_macos.
- **Windows passwords.** A password field's value is hidden only when its
  name looks like a password (or the app masks it): xa11y does not read UIA's
  `IsPassword`. The fix belongs upstream.
- **Pathological selectors.** A 1 KiB selector such as `* * * …` is bounded
  by the walk's 5 s deadline, not by its size.
- **Parent cycles.** The walker keeps no visited set; AX and UIA mint a fresh
  handle per fetch, so a cycle cannot form there.
- **Old Windows.** The DPI pin links Windows 10 1607+ APIs; older Windows
  cannot load the worker.
- **Bidi controls.** xa11y strips them from name, value and description; the
  tree text escapes control characters and line separators, not bidi
  formatting beyond that.
- **Late cancel.** A caller cancelled after the executor replied, but before
  it read the reply, still discards the executor (conservative).
- **Cancelled input.** A coordinate action cancelled mid-dispatch may still
  land after its invalidation bump and after `act_lock` is released.
- **Wayland.** No screenshots, pointer or keyboard: text goes in only through
  `set_value` (AT-SPI `EditableText`), so terminals and fields without it take
  no text.
- **No window manager.** On a bare X server (Xvfb/Xorg alone) a click does not
  give keyboard focus, so toolkits that wait for focus (winit/AccessKit apps:
  egui, Bevy, Slint) ignore typed keys; run a window manager.
- **No gate on the accessibility bus.** Any process of the same user reads
  and drives every app's tree; redaction by role and name covers passwords,
  and there is no app allowlist.
- **IsEnabled on at-spi2-core ≥ 2.61** falls back to false when the last
  listener (e.g. Orca) leaves; the warning reappears on the next `list`.
- **Flatpak.** An app behind `xdg-dbus-proxy` may report the proxy's pid, so
  `app_pid` scoping can miss it (unverified).
- **Per-snapshot cost on Linux.** Every snapshot builds a provider (two D-Bus
  connections and an X one): about 32 ms per find attempt, measured in the e2e
  (`snapshot_cost_is_measured`).

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

The semantic layer is unit-tested against `semantic::fake::FakeTree`, a
provider the test shapes. `tests/semantic_live.rs` runs the same layer
against a real AccessKit window (`tests/fixture`); CI runs it on Windows and
Linux.

On Linux, run the live tests only through `tests/run-linux-live.sh`, from
`computer/` after `cargo build --locked --manifest-path
tests/fixture/Cargo.toml`. It starts a private Xvfb,
D-Bus session and AT-SPI bus and runs the ignored X11 driver test and
`semantic_live` there; arguments after it pick one test
(`--test semantic_live <name>`). Never run the Linux `--ignored` tests directly
on your own desktop session: they type CJK text and ctrl+a into the focused
window and toggle Caps Lock, and the script's `IsEnabled` write would persist
`toolkit-accessibility=true` in your dconf. CI runs the e2e on ubuntu (through
the script) and Windows; the `rust` job also runs the vendored crate's tests
(`cargo test --locked --manifest-path vendor/xa11y-linux/Cargo.toml --lib`)
and a `x86_64-unknown-linux-musl` build, which fails if anything links a
system C library. The macOS `computer-native` job runs the vendored
xa11y-macos patch tests (`cargo test --locked --manifest-path
vendor/xa11y-macos/Cargo.toml --lib vendored_patches`); its upstream tests
build a provider, need TCC grants and run only on a Mac
(`vendor/xa11y-macos/PATCHES.md`).

On a Mac, from `computer/` in a terminal with Accessibility and Screen
Recording:

    cargo build --locked --manifest-path tests/fixture/Cargo.toml
    COMPUTER_A11Y_FIXTURE=$PWD/tests/fixture/target/debug/computer-a11y-fixture \
      cargo test --locked --test semantic_live -- --ignored --test-threads=1

Set `COMPUTER_A11Y_FIXTURE_LOG_DIR` to keep the fixture logs somewhere else than
the temp dir. Keep hands off the mouse while it runs. Under tmux the grants
that count are those of the terminal app that started the tmux server. With two native sessions, a click in one must turn the other's
refs into `stale_ref`.

Then, on a live stack with the harness and approval-gate, have an agent run
`computer::find` and `computer::act` against the fixture or TextEdit: check
the approval card, an act approved after several seconds (refs stay valid),
the result reaching the model, and `observe` without image in the console.
Without Accessibility, `find` must fail with `permission_denied`, never an
empty tree.
