# scrapling

[Scrapling](https://github.com/D4Vinci/Scrapling) as an iii worker. It maps
Scrapling's three fetch tiers and its parsing engine to `scrapling::*` functions
on the iii bus: fast HTTP with TLS impersonation, a Camoufox anti-bot browser, a
full Playwright/Chromium browser, screenshots, and CSS/XPath/regex/adaptive
extraction.

While connected it also injects a usage section into the agent system prompt
via the harness `pre-generate` hook (`scrapling::inject-guidance`), so the
guidance is presence-gated: no scrapling worker, no prompt text. The binding is
one-shot at startup and relies on the engine's recoverable triggers (iii #1962,
engine ≥ 0.21.8): bound before the harness is up, it parks as a pending intent
and activates when the harness registers the trigger type. On older engines the
bind is silently dropped.

## Install

```bash
iii worker add scrapling
```

The worker is a `deploy: image` Python worker. The image build runs
`scrapling install`, which downloads the Camoufox and Chromium browsers used by
`stealthy-fetch`, `dynamic-fetch`, and `screenshot`.

## Functions

| Function | What it does |
|---|---|
| `scrapling::fetch` | HTTP get/post/put/delete (curl_cffi, TLS impersonation) |
| `scrapling::stealthy-fetch` | Camoufox stealth browser — Cloudflare/Turnstile bypass, WebRTC/canvas hardening |
| `scrapling::dynamic-fetch` | Playwright/Chromium — JS render, waits, XHR capture, CDP |
| `scrapling::screenshot` | Page screenshot (image content blocks) via a browser fetcher |
| `scrapling::extract` | Parse HTML with a declarative selector list |
| `scrapling::css` | One CSS query over provided HTML |
| `scrapling::xpath` | One XPath query over provided HTML |
| `scrapling::regex` | Regex over the visible text of provided HTML |
| `scrapling::find-similar` | An example element + structurally similar elements |

### Fetch output

Fetch functions return:

```json
{ "status": 200, "url": "...", "headers": {}, "cookies": {}, "encoding": "utf-8",
  "extracted": { "...": "..." }, "html": "<...>" }
```

`extracted` appears only when `selectors` are given; `html` only when
`include_html: true`. Called with a `urls` array, the response is
`{ "results": [ ...one object per url... ] }` (a failed URL yields
`{ "url", "error" }`, so one bad URL doesn't sink the batch).

### Selector contract

`selectors` (on the fetchers and `scrapling::extract`) is a list of specs:

```json
[
  { "name": "title", "css": "h1" },
  { "name": "links", "css": "a", "attr": "href", "all": true },
  { "name": "price", "regex": "price (\\d+)" },
  { "name": "card_html", "css": ".card", "html": true }
]
```

- one of `css` / `xpath` / `regex` per spec
- `attr` pulls an attribute; `html` pulls inner HTML; otherwise text
- `all: true` returns every match as a list, else the first match (or `null`)

## Examples

```bash
# HTTP fetch + extract in one call
iii trigger scrapling::fetch --payload '{
  "url": "https://example.com",
  "selectors": [{ "name": "title", "css": "title" }]
}'

# Anti-bot page
iii trigger scrapling::stealthy-fetch --payload '{
  "url": "https://nopecha.com/demo/cloudflare",
  "solve_cloudflare": true,
  "selectors": [{ "name": "body", "css": "#padded_content a", "all": true }]
}'

# Parse HTML you already have
iii trigger scrapling::extract --payload '{
  "html": "<ul><li>a</li><li>b</li></ul>",
  "selectors": [{ "name": "items", "css": "li", "all": true }]
}'

# Fetch many URLs at once
iii trigger scrapling::fetch --payload '{ "urls": ["https://a.com", "https://b.com"] }'
```

## Config

`config.yaml` holds operator defaults applied when a call omits the field:

```yaml
defaults:
  impersonate: chrome     # HTTP fetcher fingerprint
  headless: true          # browser fetchers
  network_idle: false
  proxy: ""               # "" = none
  include_html: false
max_bulk_concurrency: 5
```

`timeout` is passed per call, not defaulted here — it means **seconds** for the
HTTP fetcher and **milliseconds** for the browser fetchers.

## How it maps

| Scrapling | iii |
|---|---|
| `Fetcher.{get,post,put,delete}` | `scrapling::fetch` |
| `StealthyFetcher.fetch` | `scrapling::stealthy-fetch` |
| `DynamicFetcher.fetch` | `scrapling::dynamic-fetch` |
| browser `page.screenshot()` | `scrapling::screenshot` |
| `Selector.css / .xpath / .re / .find_similar` | `extract` / `css` / `xpath` / `regex` / `find-similar` |

## Boundaries

- Non-JSON Scrapling options (Python `page_action`/`page_setup` callbacks, proxy
  rotators, persistent sessions, the `Spider` crawl layer) are not exposed. Pass
  a single `proxy` string.
- The fetch functions are not agent-callable without human approval (outbound
  requests to arbitrary URLs); the pure parsers are. See `iii-permissions.yaml`.
