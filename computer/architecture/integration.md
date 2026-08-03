# computer — integration

For calling the worker from an agent, a sibling worker, or the console.

## Functions

Every session-scoped call takes the `session_id` that `sessions::start`
returned; `sessions::list` and `displays` are the two that do not.

| Function | In | Out |
|---|---|---|
| `computer::sessions::start` | `image?`, `endpoint?`, `os?`, `monitor?` | `session_id`, `endpoint`, `os`, `screen` |
| `computer::sessions::list` | — | `sessions[]` with endpoint, os, screen, timestamps, screencast state |
| `computer::sessions::stop` | `session_id` | `ok`, `was_running` (idempotent) |
| `computer::displays` | — | local `displays[]`; empty off a desktop host |
| `computer::screenshot` | `session_id` | image + text content blocks, `details.width/height/mime` |
| `computer::observe` | `session_id`, `include_a11y?` | screenshot plus the accessibility tree where the guest has one |
| `computer::act` | `session_id`, `action`, coordinates / `text` / `keys` | `ok`, `detail` |

`act` actions: `click` (`left_click`), `right_click`, `double_click`, `move`,
`drag` (`to_x`/`to_y`), `scroll` (`scroll_x`/`scroll_y`), `type` (`text`),
`press` / `hotkey` (`keys`, e.g. `["cmd","c"]`).

**Coordinates are the screenshot's pixels**, top-left origin. That is the whole
contract: screenshot, read a position off the image, act on it. After anything
that changes the screen, screenshot again before acting — a stale coordinate is
a click somewhere else.

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

## The guardrail split (read this before wiring an agent)

`computer::act` has **no** confirm step, denylist, or read-only mode. That is
deliberate, and it is the same split the browser worker uses: device-level
limits live in the worker, action-level policy lives above it.

| Layer | Lives in | Examples |
|---|---|---|
| Device limits | this worker | permission preflight, `max_sessions`, capture downscale, command timeouts |
| Action policy | engine + harness | dispatch policy (`options.functions`), the approval gate on `computer::act`, permission profiles |

A raw `act` does what it is told, on a real machine. Anything that should ask a
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
