---
name: computer
description: >-
  Drive a full desktop computer over the iii bus: connect a computer-use
  session, see the screen as an image, and click, type, scroll, and run shell
  by coordinate. Reach for it when a task needs to operate a real GUI app or
  whole desktop, not just a web page.
---

# computer

The computer worker connects to a live desktop (a computer-server backend, for
example a booted desktop sandbox) and turns it into iii functions. Start a
session, take a screenshot to see the screen, then act on it by pixel
coordinate: the screenshot is the source of truth for where things are, and
`computer::act` clicks and types at those coordinates. The session stays alive,
so you can act, screenshot the result, and act again.

Sessions are durable (a worker restart reconnects them) and cost one backend
connection each; the configured session cap is small. Stop sessions when a task
is done. Screenshots are large; take one when you need to see the screen, not
after every action.

## When to Use

- Operating a desktop GUI application that is not a web page (an installer, a
  native app, a settings panel).
- End-to-end tasks that span the whole screen: move a window, drag between
  apps, use a system dialog.
- Running a command in the desktop guest and seeing its effect on screen, with
  `computer::shell` for the command and `computer::screenshot` for the result.
- Reading or writing a file inside the guest as part of a desktop task.

## Boundaries

- Web-only tasks belong to the [browser](../browser) worker (a real Chromium
  tab with an accessibility outline and page console) or `web::fetch` for a
  one-shot page. Do not start a desktop session just to open a URL.
- The desktop itself is external: `computer` connects to a computer-server
  endpoint, it does not boot the VM. Point `default_endpoint` at a running
  backend or pass `endpoint` to `computer::sessions::start`.
- `computer::screencast::*` and `computer::frame` are console-UI plumbing, not
  agent surface.
- Coordinates are integer pixels, top-left origin, in the space of the most
  recent screenshot. Re-screenshot after the screen changes before acting.

## Functions

- `computer::sessions::start` — connect a desktop session; returns the
  session_id every other function needs, plus the screen size.
- `computer::sessions::list` — live sessions with endpoint, OS, and screen.
- `computer::sessions::stop` — stop a session; idempotent.
- `computer::screenshot` — the desktop as a viewable image; how you see the
  screen before acting.
- `computer::observe` — screenshot plus, on macOS guests, the accessibility
  tree (`include_a11y: true`).
- `computer::act` — click, right_click, double_click, move, drag, scroll, type,
  press, or hotkey, addressed by pixel coordinates.
- `computer::shell` — run a command in the guest; returns stdout, stderr, exit
  code.
- `computer::files::read` / `computer::files::write` — read and write text
  files in the guest.

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
