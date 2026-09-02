---
type: index
name: browser
description: >-
  Interactive Chromium sessions for reading and driving real web pages: open a
  URL, read the page as text, click and type, and read the page's own console
  and network history. Also scrapes: HTTP and browser fetching, screenshots,
  persistent sessions, crawling, and CSS/XPath/regex parsing of HTML you
  already have. Reach for it when a task involves a running web app,
  especially "why is this page broken", or when pulling data off the web.
---

# browser

The browser worker does two things. It runs real Chromium sessions on the bus
(`browser::*`), and it parses HTML natively without a browser
(`browser::*` — CSS/XPath/regex queries, element search,
HTML→Markdown, over any HTML string you already have).

Start a session,
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

- Do not start a browser session just to read a page once. One-shot fetching
  is `browser::fetch` (no browser) or `browser::dynamic-fetch`
  (Chromium, when the page needs JS). Sessions are for flows that need state
  between steps.
- Parsing HTML you already have needs neither a session nor a fetch: use the
  `browser::*` parse function below. Starting Chromium to run a
  CSS selector over a string you are already holding is pure waste.
- `solve_cloudflare` is available on `browser::stealthy-fetch` and stealthy
  Scrapling sessions. Use `browser::handoff` for challenges in an interactive
  session or when automated solving does not clear the page.
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

### Fetching, sessions and crawl

These reach the network, so they need approval. All return one envelope —
`{status, url, headers, cookies, encoding}` — and can extract or render inline
via `selectors` / `format: markdown|text` / `include_html`, so you rarely need
a second call to parse what you fetched. Each takes a single `url` or a bulk
`urls` list.

- `browser::fetch` — plain HTTP, no browser. The default choice: fastest,
  cheapest. Safe mode uses bounded native HTTP; certified compat mode uses the
  frozen curl-impersonate wire behavior.
- `browser::dynamic-fetch` — real Chromium over CDP, for pages that
  need JavaScript. Supports `wait_selector` (+ `wait_selector_state`),
  `network_idle`, and a plain `wait`.
- `browser::stealthy-fetch` — same, plus masking of the automation
  tells a page can read. Escalate here only when `dynamic-fetch` is detected.
- `browser::screenshot-url` — page as image tiles (≤1024px wide, ≤6
  tiles); says so in the caption when a page is taller than the budget.
- `browser::session-open` / `session-fetch` / `session-close` /
  `session-list` — keep cookies and browser state across fetches.
  HTTP, dynamic and stealthy types are private FIFO sessions with UUID4 hex
  ids; they never appear in `browser::sessions::list` and reject interactive
  ids. Close sessions when done.
- `browser::crawl` — breadth-first from `start_urls`, same-domain by
  default, capped by `max_pages` (20) and `max_depth` (2). The response holds
  only a ≤10-item sample; read the rest from the stream it names.

Safe mode refuses private, loopback and cloud-metadata addresses on every one
of these connections (including redirects and crawl hops). To scrape a local
dev server the operator must set `browser.scrapling.allow_loopback` in worker
config. Compat mode reproduces the standalone worker's unrestricted network
behavior and is for trusted calls.

### HTML parsing — no session, no browser, no network

These take an `html` string and never touch Chromium. Use them on HTML from
any source (a fetch body, a file, a page you already read).

- `browser::extract` — declarative selector list in one call:
  each entry names a `css`/`xpath`/`regex` plus optional `attr`/`html`/`all`,
  and the response is a `{name: value}` map. The right default when pulling
  several fields off one document.
- `browser::css` / `browser::xpath` — one query;
  `first: true` returns a scalar, otherwise an array. `attr` pulls an
  attribute instead of text.
- `browser::regex` — regex over the document's visible text.
- `browser::find` — element search by tag/attribute filters
  (+ optional text regex), BeautifulSoup-style.
- `browser::find-by-text` / `browser::find-by-regex` —
  find elements by their visible text. Responses carry generated css/xpath
  selectors for each hit, so you can feed one straight back into a query.
- `browser::find-similar` — give one example element, get its
  structural siblings. The fast path for "extract every card/row on this
  page" without hand-writing a selector.
- `browser::describe` — inspect the first match: attributes,
  class list, generated selectors, parent/child/sibling counts.
- `browser::to-markdown` — HTML → compact Markdown (or text), with
  an optional CSS scope and a main-content cleaner. Use it to shrink a page
  before putting it in context.

`adaptive: true` persists element identities in the configured SQLite file.
Parse calls are auto-allowed, so do not assume parsing is side-effect-free
when adaptive tracking is enabled. Safe mode enforces the configured database
quota; compat mode preserves the standalone worker's unbounded behavior.

### Safe and compat modes

`browser.scrapling.security_mode` defaults to `safe`. Safe mode keeps SSRF,
TLS, proxy, response-size, timeout, and adaptive-database policy checks; an
option the safe backend cannot enforce is refused with an actionable error.
Compat is eligible only on certified Linux x86_64/aarch64 builds containing
the frozen curl-impersonate and Chromium artifacts. Other targets reject it,
and a Tier-1 build missing an artifact reports a capability error instead of
silently using the safe transport.

Native ids are `browser::<leaf>`. Map `scrapling::screenshot` to
`browser::screenshot-url`; `browser::screenshot` is the interactive-session
function. Crawl's default stream is `browser::crawl`.

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
