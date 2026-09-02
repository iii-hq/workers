---
type: index
name: scrapling
description: >-
  Scrape the web over the iii bus with Scrapling — fast HTTP fetch with TLS
  impersonation, a Camoufox anti-bot browser (Cloudflare bypass), full
  Playwright rendering, screenshots, and CSS/XPath/regex/adaptive extraction.
---

# scrapling

The scrapling worker puts [Scrapling](https://github.com/D4Vinci/Scrapling) on
the iii bus. Three fetch tiers share one output shape: `scrapling::fetch` (fast
HTTP, curl_cffi TLS impersonation), `scrapling::stealthy-fetch` (Camoufox stealth
browser — solves Cloudflare/Turnstile, hardens WebRTC/canvas), and
`scrapling::dynamic-fetch` (Playwright/Chromium — JS rendering, waits, XHR
capture). Every fetch can extract in the same call by passing a `selectors` list,
and can fan out over a `urls` array. Parsing is also exposed standalone
(`extract`, `css`, `xpath`, `regex`, `find-similar`) for HTML you already have.

Fetches return `{status, url, headers, cookies, encoding}` plus `extracted` (when
`selectors` given) and `html` (when `include_html: true`). A bulk call returns
`{results: [...]}`.

## When to Use

- Grab a page fast and pull fields in one shot: `scrapling::fetch` with `url`
  and `selectors: [{name, css, all?}]`.
- Get past anti-bot / Cloudflare: `scrapling::stealthy-fetch` with
  `solve_cloudflare: true`.
- Render JS-heavy pages or wait on selectors: `scrapling::dynamic-fetch` with
  `wait_selector`, `network_idle`, or `capture_xhr`.
- Screenshot a page: `scrapling::screenshot` (`fetcher: dynamic|stealthy`,
  `full_page?`).
- Parse HTML you already fetched elsewhere: `scrapling::extract` /
  `scrapling::css` / `scrapling::xpath` / `scrapling::regex`.
- Scrape a repeating list from one example element: `scrapling::find-similar`
  with an `anchor` CSS selector.
- Fetch many URLs at once: pass `urls: [...]` to any fetch function.

## Boundaries

- The fetch functions make outbound requests to arbitrary URLs and are not
  agent-callable without human approval (SSRF surface); the pure parsers are
  (see iii-permissions.yaml).
- The stealthy/dynamic fetchers and screenshot need the bundled browsers
  (Camoufox/Chromium) installed by `scrapling install` in the image.
- Non-JSON Scrapling options (Python `page_action`/`page_setup` callbacks,
  proxy rotators, session objects, the Spider crawl layer) are intentionally not
  exposed. Pass a single `proxy` string, not a rotator.
- Selector spec fields: `{name, css|xpath|regex, attr?, html?, all?}` — `attr`
  pulls an attribute, `html` pulls inner HTML, otherwise text; `all` returns a
  list.

## Functions

- `scrapling::fetch` — HTTP get/post/put/delete; `url`|`urls`, `method`,
  `headers`, `impersonate`, `selectors`, `include_html`.
- `scrapling::stealthy-fetch` — Camoufox anti-bot fetch; `solve_cloudflare`,
  `block_webrtc`, `wait_selector`, `selectors`, bulk.
- `scrapling::dynamic-fetch` — Playwright fetch; `network_idle`, `wait_selector`,
  `real_chrome`, `cdp_url`, `capture_xhr`, `selectors`, bulk.
- `scrapling::screenshot` — page as image content blocks; `fetcher`, `full_page`, `format`.
- `scrapling::extract` — parse `html` with a `selectors` list → named map.
- `scrapling::css` — one CSS query over `html`; `first?`, `attr?`.
- `scrapling::xpath` — one XPath query over `html`; `first?`, `attr?`.
- `scrapling::regex` — regex over the visible text of `html`; `first?`.
- `scrapling::find-similar` — anchor element + structurally similar elements.
