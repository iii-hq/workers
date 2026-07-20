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
- Attach mode reaches the user's real browser profile with its logged-in
  sessions. It is disabled unless `allow_attach` is set, and adoption is
  exclusive (one session per tab) so two sessions never fight over a tab.
  Reach for a launched session when you do not specifically need the user's
  existing logins.
- `browser::styles::write` edits are visual experiments only: they die on the
  next navigation and never touch source files. Use them to find the right
  value, then edit the codebase.
- `browser::pick::*`, `browser::screencast::*`, and `browser::frame` are console-UI plumbing, not agent surface.
- The ghost cursor and session-status badge are in-page overlays for a human
  watching the streamed viewport; they appear only while screencast is
  active and never affect page content or the accessibility snapshot.
- Navigation is limited to the configured URL schemes (http/https by
  default).

## Functions

- `browser::sessions::start` — launch a Chromium session; returns the
  session_id every other function needs. `read_only: true` starts an
  inspection-only session.
- `browser::sessions::list` — live sessions with their current URL.
- `browser::sessions::stop` — stop a session; idempotent. A launched session
  closes its browser; an attached session closes only a tab it opened and
  releases an adopted user tab untouched.
- `browser::sessions::attach` — bind a session to an already-running browser
  over CDP (start Chrome with `--remote-debugging-port`): open a fresh tab
  the session owns, or adopt an existing logged-in tab by URL substring.
  Off unless `allow_attach` is set in config.
- `browser::tabs::list` — open tabs of a running browser at a CDP endpoint,
  with which are already adopted; read-only.
- `browser::doctor` — read-only environment report: which Chromium would
  launch, its version, capacity, whether attach and recording are available,
  and anything degraded with how to enable it.
- `browser::recording::start` / `browser::recording::stop` — capture a
  session's live viewport to a webm or mp4 file via ffmpeg; stop returns the
  path, duration, and frame count. Requires ffmpeg on PATH.
- `browser::navigate` — go to a URL and wait for the load.
- `browser::snapshot` — the page as an accessibility outline with `[ref=eN]`
  handles; the default way to read a page. `diff: true` returns only what
  changed since the previous snapshot.
- `browser::act` — click, hover, type, press, or scroll, addressed by ref or
  viewport coordinates.
- `browser::screenshot` — viewable JPEG of the viewport, for when layout or
  rendering matters.
- `browser::evaluate` — run a JavaScript expression in the page and get the
  completion value.
- `browser::execute` — run a multi-step async script in the page: top-level
  await and return, `log(...)`, `sleep(ms)`, `waitFor(selector)`, and a
  `state` object persisted across execute calls for the session. One call
  replaces a chain of act/evaluate round-trips.
- `browser::handoff` — pause the session for a human-only step (CAPTCHA,
  2FA, payment): show an in-page continue banner and block until the human
  clicks it, a `browser::handoff::confirm` call resolves it, or the timeout
  elapses. Verify the expected page state after it returns.
- `browser::handoff::confirm` — resolve a paused handoff from outside the
  page (by handoff_id, or the one pending handoff for a session_id).
- `browser::console::read` — captured console entries; filter with
  pattern/level and page with since_seq.
- `browser::network::read` — captured requests; failed_only=true is the fast
  path for what broke.
- `browser::history` — back, forward, or reload.
- `browser::dom::read` — DOM tag outline with refs, for structure the
  accessibility tree hides.
- `browser::styles::read` / `browser::styles::write` — computed styles and
  live inline CSS edits on one element.

## Workflow: inspect before acting

1. Snapshot first; act on refs from the latest snapshot, never from memory
   of an earlier one. Refs are unique per snapshot and die on navigation, so
   a stale ref fails with an error instead of clicking the wrong element.
2. After an action that changes the page, re-read with
   `browser::snapshot { diff: true }`: it returns only added and removed
   lines, which keeps loops cheap.
3. Use `browser::execute` when a step needs waiting or several dependent
   reads and writes; use `browser::act` for single trusted input events
   (in-page script clicks are not trusted events).
4. Pure inspection tasks (audits, scraping a logged-in page you must not
   touch) belong in a `read_only: true` session.
5. When a flow hits a step only a human can do (CAPTCHA, 2FA, payment
   confirmation), call `browser::handoff` and wait, rather than trying to
   automate it. After it returns confirmed, re-read the page and verify the
   step actually landed — a human clicking Continue is not proof the step
   succeeded.

## Workflow: destructive UI actions

For destructive page actions (deleting records, cancelling subscriptions)
use two phases, never one script:

1. Discovery: read candidates and return their exact stable text or ids for
   the user to approve. Do not act in this call.
2. Action: operate only on the approved identifiers, read any confirmation
   dialog text, and abort unless it matches the approved action.

## Keeping context small

Browser outputs are large and land in the transcript, so a few careless reads
fill the context window and force a compaction. Read economically:

- Prefer `browser::snapshot` (a compact text outline) over
  `browser::screenshot` (a whole image) for reading a page; take a screenshot
  only when layout or rendering is the actual question.
- Filter every read instead of dumping: `browser::console::read` takes
  `pattern`/`level`, `browser::network::read` takes `failed_only`, and both
  page with `since_seq` so a follow-up read returns only what is new.
- On a big page, read a subtree: pass a `ref` to `browser::dom::read` rather
  than snapshotting the whole document repeatedly.
- Reuse one session across steps and `browser::sessions::stop` it when done;
  do not re-snapshot after every action, only after the page actually
  changed.

## Reactive triggers

Bind a `browser::*` trigger when another function should react to session
activity as it happens instead of polling the read functions. The types:
`browser::session-started`, `browser::session-stopped`, `browser::navigated`,
`browser::console-event` (one captured entry per firing; high volume),
`browser::picked` (a human picked an element in the console UI; the payload
carries a ref that `browser::act` accepts directly), and
`browser::handoff-requested` (a session is paused waiting for a human; the
console surfaces it beside the live viewport).

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
