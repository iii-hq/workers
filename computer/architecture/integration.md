# computer — integration

For calling the worker from an agent, a sibling worker, or the console.

## Functions

Every session-scoped call takes the `session_id` that `sessions::start`
returned; `sessions::list` and `displays` are the two that do not.

| Function | In | Out |
|---|---|---|
| `computer::sessions::start` | `image?`, `endpoint?`, `os?`, `monitor?` | `session_id`, `endpoint`, `os`, `screen`, `semantic` |
| `computer::sessions::list` | — | `sessions[]` with endpoint, os, screen, timestamps, screencast state, `semantic` |
| `computer::sessions::stop` | `session_id` | `ok`, `was_running` (idempotent) |
| `computer::displays` | — | local `displays[]`; empty off a desktop host |
| `computer::screenshot` | `session_id` | image + text content blocks, `details.width/height/mime` |
| `computer::observe` | `session_id`, `include_image?` (true), `include_a11y?` (false), `app_pid?`/`app_name?`/`root_ref?`, `max_nodes?` (≤256), `max_depth?` (≤12) | image (optional) + text content; with `include_a11y`, the tree as `[ref=…]` lines |
| `computer::find` | `session_id`, `selector`, `app_pid?`/`app_name?`/`root_ref?`, `max_nodes?`, `max_depth?`, `timeout_ms?` (≤30000) | `tree`, `matches`, `truncated`, `waited_ms` |
| `computer::act` | `session_id`, `action`, coordinates / `text` / `keys`, or `ref` | `ok`, `detail` |
| `computer::run` | `session_id`, `goal`, `app_name?`/`app_pid?` (one required), `inputs?`, `max_steps?` (≤60), `timeout_ms?` (≤240000) | `status`, `reason?`, `steps[]`, `needs_text?`, `tree`, `judge_requests`, `elapsed_ms` |

`act` actions: `click` (`left_click`), `right_click`, `double_click`, `move`,
`drag` (`to_x`/`to_y`), `scroll` (`scroll_x`/`scroll_y`), `type` (`text`),
`press` / `hotkey` (`keys`, e.g. `["cmd","c"]`).

**Coordinates are the screenshot's pixels**, top-left origin. That is the whole
contract: screenshot, read a position off the image, act on it. After anything
that changes the screen, screenshot again before acting — a stale coordinate is
a click somewhere else.

**By ref** (native sessions): `invoke`, `set_value` (`text`),
`toggle`, `select`, on a ref from `observe` (with `include_a11y`) or `find`.
Refusals come back as errors with a stable code — `invalid_input`,
`invalid_selector`, `stale_ref`, `not_supported`, `busy`,
`permission_denied`, `unsupported_session`, `backend_error` — none of which
had any effect; `effect_unknown` means the action may or may not have
happened. With `include_image=true`, `observe` still returns the image when the
tree cannot be read right now (`unsupported_session`, `permission_denied`,
`backend_error`, `busy`), with an appended `a11y tree unavailable: <code>: …`
line; a wrong request (`invalid_input`, `stale_ref`) is an error either way,
except that an empty foreground (no `app_*`/`root_ref` and no app in front)
also degrades to that line.
Every tree starts with the untrusted-content marker and an
`app "<name>" pid <n>` line. `find` with `timeout_ms` also waits for an
`app_name` that is not running yet, or for any app to reach the foreground.
All native sessions share one desktop: a coordinate or keyboard action on any
of them kills the refs of all of them. On Windows
`invoke` falls through Invoke → Toggle → Select → ExpandCollapse.
`sessions::start` and `sessions::list` report `semantic: { supported,
reason? }` per session.
They also report `coordinates: { supported, reason? }`: false only on a native
Linux session on Wayland, where every screenshot and coordinate action
(including `type`, `press`, `hotkey` and the console screencast) is refused
with `unsupported_session`. On a native Linux session `semantic.reason` can be
set while `supported` is true: a warning that accessibility is off. There
`semantic.supported` is false when the AT-SPI bus is missing or did not answer
within 1.5 s; every `sessions::list` asks the bus again, so tell the user and
re-check with it. Remote and sandbox sessions no longer return a tree:
`observe` with `include_a11y` appends
`a11y tree unavailable: unsupported_session` (or fails with that code when
`include_image` is false).

`computer::screencast::start` / `stop` and `computer::frame` are console
plumbing (flagged internal, denied in `iii-permissions.yaml`). Agents use
`screenshot`.

## Trigger types

| Trigger type | Fires when | Payload |
|---|---|---|
| `computer::session-started` | A session connected and probed | `session_id`, `endpoint`, `os`, `screen`, `timestamp` |
| `computer::session-stopped` | A session ended | `session_id`, `reason` (`stopped` \| `idle`), `timestamp` |

Both accept an optional `{ "session_id": "..." }` equality filter. Bind them
instead of polling `sessions::list`.

| Trigger type | Fires when | Payload |
|---|---|---|
| `computer::frame-changed` | The stored newest frame of a session changed: `change` is `updated` (new frame) or `cleared` (screencast/session stopped, capture failed) | `session_id`, `epoch`, `frame_seq`, `change`, `width`, `height`, `mime?`, `bytes?`, `timestamp` |

Config `{ "session_id": "..." }` is required; unknown keys are rejected; at most
64 live bindings. The payload carries no image: read it with `computer::frame`
(`since_frame` = the `frame_seq` you already have). Order by `(epoch,
frame_seq)`. Delivery is latest-state only: while a delivery to a binding is in
flight, newer notifications replace the pending one, so a slow consumer sees
fewer notifications, never a backlog. Nothing is stored or replayed: bind
first, then read `computer::frame` for the current frame, and read again after
a reconnect.

## The guardrail split (read this before wiring an agent)

`computer::act` has **no** confirm step, denylist, or read-only mode. That is
deliberate, and it is the same split the browser worker uses: device-level
limits live in the worker, action-level policy lives above it.

| Layer | Lives in | Examples |
|---|---|---|
| Device limits | this worker | permission preflight, `max_sessions`, capture downscale, command timeouts |
| Action policy | engine + harness | dispatch policy (`options.functions`), the approval gate on `computer::act` and `computer::run`, permission profiles |

A raw `act` does what it is told, on a real machine. `computer::run` acts
in-process, so approving one run approves every accessibility action it
takes; there is no per-step gate. Anything that should ask a
human first belongs in an approval-gate `pre-trigger` hook or a dispatch
allow-list, where the human-facing surface (the console renders every capture
inline) already is.

## What not to do

- Do not use this worker to open a URL. That is the
  [browser](https://github.com/iii-hq/workers/tree/main/browser) worker, which
  gives you a DOM instead of pixels.
- Do not use it to run commands or move files. That is `shell` for a native
  session, `sandbox::exec` / `sandbox::fs` inside a sandboxed desktop, and the
  guest executor for a remote one — `shell` runs on the worker's host, which
  is not the guest. This worker sees the screen and drives the cursor, nothing
  else.
- Do not screenshot after every action. Captures are large and land in the
  transcript; `act` returns a `detail` line that says what happened.
- Do not hold a session open across unrelated work. The cap is small by design;
  stop it and start another.
