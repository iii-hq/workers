---
name: browser
description: >-
  Interactive Chromium sessions for reading and driving real web pages: open a
  URL, read the page as text, click and type, and read the page's own console
  and network history. Reach for it when a task involves a running web app,
  especially "why is this page broken".
---

# browser

The browser worker runs real Chromium sessions on the bus. Start a session,
navigate, and the page becomes data: `browser::snapshot` returns an
accessibility outline whose `[ref=eN]` handles feed straight into
`browser::act`, and everything the page logs (console calls, uncaught
exceptions, failed requests) is captured into per-session ring buffers you can
query. This is the difference from one-shot fetching: the session stays alive,
so you can act, observe the result, and read what the page said about it.

Sessions are headless by default and cost a Chromium process each; the
configured session cap is small. Stop sessions when a task is done. Refs die
on navigation; re-snapshot before acting after any page change.

## When to Use

- A web app misbehaves and you need the page's own evidence: read console
  errors and failed requests with `browser::console::read` /
  `browser::network::read` instead of guessing from source.
- Verifying frontend work end to end: navigate to the dev server, act on the
  UI, snapshot the result.
- Multi-step flows on a real page: forms, logins, anything that needs state
  to persist between steps.
- Reading a page that only renders with JavaScript, when you also need to
  interact with it afterwards.
- Live style experiments: `browser::styles::read` and
  `browser::styles::write` inspect and change an element's CSS in the running
  page without touching source files.

## Boundaries

- One-shot fetching and scraping belong to `web::fetch` (plain HTTP) and the
  scrapling worker (stealth fetching and bulk extraction). Do not start a
  browser session just to read a static page once.
- `browser::styles::write` edits are visual experiments only: they die on the
  next navigation and never touch source files. Use them to find the right
  value, then edit the codebase.
- `browser::pick::*`, `browser::screencast::*`, `browser::frame`, and
  `browser::pick::hint` are console-UI plumbing, not agent surface.
- Navigation is limited to the configured URL schemes (http/https by
  default).

## Functions

- `browser::sessions::start` — launch a Chromium session; returns the
  session_id every other function needs.
- `browser::sessions::list` — live sessions with their current URL.
- `browser::sessions::stop` — stop a session; idempotent.
- `browser::navigate` — go to a URL and wait for the load.
- `browser::snapshot` — the page as an accessibility outline with `[ref=eN]`
  handles; the default way to read a page.
- `browser::act` — click, hover, type, press, or scroll, addressed by ref or
  viewport coordinates.
- `browser::screenshot` — viewable JPEG of the viewport, for when layout or
  rendering matters.
- `browser::evaluate` — run a JavaScript expression in the page and get the
  completion value.
- `browser::console::read` — captured console entries; filter with
  pattern/level and page with since_seq.
- `browser::network::read` — captured requests; failed_only=true is the fast
  path for what broke.
- `browser::history` — back, forward, or reload.
- `browser::dom::read` — DOM tag outline with refs, for structure the
  accessibility tree hides.
- `browser::styles::read` / `browser::styles::write` — computed styles and
  live inline CSS edits on one element.

## Reactive triggers

Bind a `browser::*` trigger when another function should react to session
activity as it happens instead of polling the read functions. The types:
`browser::session-started`, `browser::session-stopped`, `browser::navigated`,
`browser::console-event` (one captured entry per firing; high volume), and
`browser::picked` (a human picked an element in the console UI; the payload
carries a ref that `browser::act` accepts directly).

If you just ran `browser::navigate` yourself, its return value already tells
you the outcome; bind triggers when a different worker needs to observe
sessions it does not drive.

### How to bind

1. Register a handler: `registerFunction('mywatcher::on-console', handler)`.
2. Register the trigger:

```typescript
iii.registerTrigger({
  type: 'browser::console-event',
  function_id: 'mywatcher::on-console',
  config: { session_id: 'b1' },
})
```

Every `browser::*` binding accepts the optional `session_id` equality filter;
omit it to receive events for all sessions. `browser::console-event` fires
per entry, so filter by session and treat `browser::console::read` as the
durable record. For event payload shapes, call `get function info` on the
trigger type.
