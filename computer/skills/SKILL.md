---
name: computer
description: >-
  Drive a full desktop computer over the iii bus: start a session, see the
  screen as an image, and click, type, and scroll by coordinate. Reach for it
  when a task needs to operate a real GUI app or whole desktop, not just a web
  page.
---

# computer

The computer worker turns a live desktop into iii functions. Start a session
with neither `image` nor `endpoint` to drive the local machine this worker runs
on (unless the operator configured a default image or endpoint, which is
resolved first), pass an `image` to boot a sandboxed desktop, or an `endpoint`
to drive a remote one. Take a
screenshot to see the screen, then act on it by pixel coordinate: the screenshot
is the source of truth for where things are, and `computer::act` clicks and
types at those coordinates. The session stays alive, so you can act, screenshot
the result, and act again.

Sessions usually survive a worker restart (it reconnects them best-effort, so
an id can still go away — start a new session if one does) and cost one driver
connection each; the configured session cap is small. Stop sessions when a task
is done. Screenshots are large; take one when you need to see the screen, not
after every action.

## When to Use

- Operating a desktop GUI application that is not a web page (an installer, a
  native app, a settings panel).
- End-to-end tasks that span the whole screen: move a window, drag between
  apps, use a system dialog.
- Watching the effect of a change on screen: run the command with the `shell`
  worker, then `computer::screenshot` to see the result.

## Boundaries

- Web-only tasks belong to the
  [browser](https://github.com/iii-hq/workers/tree/main/browser) worker (a real
  Chromium tab with an accessibility outline and page console) or `web::fetch`
  for a one-shot page. Do not start a desktop session just to open a URL.
- Shell and files on the desktop belong to the `shell` worker (`shell::exec`,
  `shell::fs::*`), not this worker. `computer` only sees the screen and drives
  the cursor.
- With neither argument it drives whatever the configuration defaults to, and
  the local machine when nothing is configured; an `image` boots a sandboxed
  desktop through the iii-sandbox worker; an `endpoint` connects to a desktop
  somebody else booted.
- `computer::screencast::*` and `computer::frame` are console-UI plumbing, not
  agent surface.
- Coordinates are integer pixels, top-left origin, in the space of the most
  recent screenshot. Re-screenshot after the screen changes before acting.

## Functions

- `computer::sessions::start` — connect a desktop session; returns the
  session_id every session-scoped function needs (all but
  `computer::sessions::list` and `computer::displays`), plus the screen size.
- `computer::sessions::list` — live sessions with endpoint, OS, and screen.
- `computer::sessions::stop` — stop a session; idempotent.
- `computer::screenshot` — the desktop as a viewable image; how you see the
  screen before acting.
- `computer::observe` — screenshot plus, on macOS guests, the accessibility
  tree (`include_a11y: true`).
- `computer::act` — click, right_click, double_click, move, drag and scroll are
  addressed by pixel coordinates; type, press and hotkey carry their own `text`
  or `keys` and land wherever the desktop's focus is.

## Keeping context small

Desktop screenshots are large and land in the transcript, so a few careless
captures fill the context window. Screenshot when you need to see the screen,
not reflexively after each action; a `computer::act` returns a short confirming
`detail` on its own. Reuse one session across steps and stop it when done.

## Reactive triggers

Bind a `computer::*` trigger when another function should react to session
activity instead of polling. The types: `computer::session-started` (payload
carries `endpoint`, `os`, `screen`) and `computer::session-stopped` (payload
carries `reason`). Both accept an optional `session_id` equality filter.

### How to bind

1. Register a handler: `registerFunction('mywatcher::on-desktop', handler)`.
2. Register the trigger:

```typescript
iii.registerTrigger({
  type: 'computer::session-stopped',
  function_id: 'mywatcher::on-desktop',
  config: { session_id: 'c1' },
})
```

Omit `session_id` to receive events for all sessions. For event payload shapes,
call `get function info` on the trigger type.
