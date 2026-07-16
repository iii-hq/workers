# web

Outbound HTTP(S) client on the [iii engine](https://github.com/iii-hq/iii)
bus. A single bus function, `web::fetch`, performs a network request and
returns a parsed `{ ok, status, headers, body }` envelope with enforced
size/timeout caps and server-side SSRF protection. It is the network
counterpart to the `shell` worker: use `web::fetch` instead of
`shell::exec` with `curl`/`wget` for any HTTP request.

The worker is a faithful Rust port of the original TypeScript `web::fetch`
implementation — same request fields and success/error/image envelopes, so
existing callers and the harness consumer are unaffected.

While connected it also injects a usage section into the agent system prompt
via the harness `pre-generate` hook (`web::inject-guidance`), so the guidance
is presence-gated: no web worker, no prompt text. The binding is one-shot at
startup and relies on the engine's recoverable triggers (iii #1962, engine ≥
0.21.8): bound before the harness is up, it parks as a pending intent and
activates when the harness registers the trigger type. On older engines the
bind is silently dropped.

## Table of contents

1. [Install](#install)
2. [Configuration](#configuration)
3. [The `web::fetch` function](#the-webfetch-function)
4. [Reading the result](#reading-the-result)
5. [Page-reading mode](#page-reading-mode)
6. [SSRF guard](#ssrf-guard)
7. [Local development & testing](#local-development--testing)

---

## Install

```bash
iii worker add web
```

`iii worker add` fetches the binary (`web`), writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next
`iii start`.

---

## Configuration

The worker reads its operational ceilings from the `configuration` worker
under the `web` key. It registers a schema + seed on boot, fetches the
authoritative value, and hot-reloads on change. All keys are optional and
fall back to the defaults below.

```yaml
web:
  default_timeout_ms: 30000      # per-request timeout when caller omits timeout_ms
  max_timeout_ms: 120000         # ceiling; caller timeout_ms is clamped DOWN to this
  default_response_bytes: 262144 # default body cap in page-reading mode (256 KiB)
  max_response_bytes: 5242880    # absolute body ceiling (5 MiB); caller max_bytes clamped to this
  max_transform_bytes: 1048576   # HTML→markdown/text runs only on bodies ≤ this (1 MiB)
  max_redirects: 5               # hops before too_many_redirects
  user_agent: 'iii-harness/0.1 (+web::fetch)'
  allow_loopback: true           # set false in prod so 127.0.0.0/8, ::1, localhost are blocked
```

A copy of these defaults lives in [`config.yaml.example`](./config.yaml.example).

---

## The `web::fetch` function

Minimal call — a URL is the only required field; everything else has a
default:

```jsonc
// agent_trigger { function: "web::fetch", payload: { "url": "https://api.github.com/zen" } }
// → { "ok": true, "status": 200, "body": "Design for failure.", ... }
```

### Request fields

| Field | Default | Notes |
|---|---|---|
| `url` *(required)* | — | absolute `http(s)://` |
| `method` | `GET` | case-insensitive; GET HEAD POST PUT PATCH DELETE OPTIONS |
| `headers` | `{}` | object; keys as written |
| `json` | — | structured payload; auto-stringified + sets `content-type: application/json`. **Wins over `body`.** |
| `body` | — | raw string body; ignored on GET/HEAD |
| `response_format` | `"text"` | `"text"` \| `"base64"` (binary) \| `"json"` (also parses into `json`) |
| `format` | — | page-reading mode: `"markdown"` \| `"text"` \| `"html"` (see below) |
| `content_filter` | — | `{ type: "pruning"\|"bm25", query?, threshold?, threshold_type?, min_word_threshold? }`. Page mode only. When set, `body` holds the **filtered** content (pruning default threshold 0.48; bm25 default 1.0; `query` falls back to page metadata). |
| `target_elements` | — | CSS selectors (tl subset: tag/`.class`/`#id`); restrict rendered content to these regions |
| `excluded_tags` | — | tag/selectors to drop before rendering (e.g. `nav`, `footer`) |
| `include_links` | `false` | adds `links: { internal, external }` (absolute URLs, classified by host) |
| `include_media` | `false` | adds `media: { images, videos, audios }` (absolute URLs) |
| `timeout_ms` | `default_timeout_ms` | clamped DOWN to `max_timeout_ms` |
| `max_bytes` | `max_response_bytes` (`default_response_bytes` in `format` mode) | over-cap body is truncated, not errored |
| `follow_redirects` | `true` | each hop re-checked against the SSRF blocklist |

### Response

**Success** (`ok: true`) — returned for *any* completed response, 2xx through 5xx:

```jsonc
{
  "ok": true,
  "status": 200,
  "status_text": "OK",
  "headers": { "content-type": "application/json", ... },  // keys LOWER-CASED
  "body": "<utf8 text | base64 when response_format=base64>",
  "json": { ... },            // only when response_format="json" and parse succeeded
  "parse_error": "…",         // only when response_format="json" and JSON.parse failed
  "response_format": "json",
  "bytes_truncated": false,   // true when body hit max_bytes (NOT an error)
  "redirect_chain": ["https://…/a"],  // omitted when no redirects
  "content_type": "text/html",  // page-reading mode only
  "transformed": "markdown",    // only when an HTML transform actually ran
  "links": { "internal": [{ "href": "…", "text": "…" }], "external": [ … ] },  // include_links
  "media": { "images": [{ "src": "…", "alt": "…" }], "videos": [ … ], "audios": [ … ] },  // include_media
}
```

**Failure** (`ok: false`) — the fetch never produced a response (no `status`):

```jsonc
{ "ok": false, "error": "blocked_host", "message": "…" }
```

| `error` | Cause | Fix |
|---|---|---|
| `invalid_payload` | Payload failed schema (bad `method`, wrong types) | Correct the named fields |
| `invalid_url` | `url` isn't a parseable absolute `http(s)://` URL | Pass a full absolute URL incl. scheme |
| `blocked_host` | Target resolves to a private / link-local / cloud-metadata IP | Don't target internal/metadata hosts; for dev loopback set `allow_loopback` |
| `timeout` | Slower than `timeout_ms` | Raise `timeout_ms` (up to ceiling) or shrink work via `max_bytes` |
| `too_many_redirects` | More than `max_redirects` hops | Use the final URL, or `follow_redirects: false` and read the `location` header |
| `transport_error` | Connection refused/reset, TLS/DNS failure, or unparseable redirect `Location` | Check host/port/cert; retry if transient |

There is **no `too_large` error** — oversize responses come back `ok: true`
with `bytes_truncated: true`.

---

## Reading the result

**`ok` tells you if the request COMPLETED, not whether the server was happy.**

- A 404 or 500 is a **successful fetch** → `ok: true`, `status: 404`. Branch on `status` for HTTP outcomes.
- `ok: false` means the request never produced a response. Branch on `error`.

```jsonc
if (!r.ok)              → fetch failed; look at r.error
else if (r.status>=400) → server returned an HTTP error; look at r.status / r.body
else                    → success; use r.body or r.json
```

---

## Page-reading mode

`response_format` (transport encoding) and `format` (page-reading) are
orthogonal — set one, not both. When `format` is set, `response_format` is
ignored.

- **`format: "markdown"`** — `text/html` → Markdown (the right default for reading pages; far fewer tokens than raw HTML).
- **`format: "text"`** — `text/html` → plain text.
- **`format: "html"`** — raw HTML.

When `content_filter` is set, `body` is the filtered output (there is no separate field) — feed it straight to a model. (If filtering would empty the page, or the page is too large or deeply nested to transform, `body` falls back to the unfiltered content.)

In page-reading mode the request goes out with a browser User-Agent +
format-matched `Accept` header, and retries once with the honest UA on a
Cloudflare `cf-mitigated: challenge` response. The HTML→markdown/text
transform runs only on bodies ≤ `max_transform_bytes`; larger pages come
back raw with `transformed` unset.

**Images:** a viewable `image/*` response (jpeg/png/gif/webp, 2xx,
complete, non-empty) returns the actual image plus a one-line text summary
instead of the JSON envelope. Anything else falls back to the normal
envelope with `response_format: "base64"`.

---

## SSRF guard

DNS is resolved **once**, every resolved address is checked, then the
request is dialed at the **validated IP** (pinned `lookup` + TLS
`servername`) — so the IP that passed the check is the IP connected to (no
DNS-rebind window). On each 3xx the `Location` is re-resolved and
re-validated before following, and `Authorization` / `Cookie` /
`Proxy-Authorization` are stripped when the redirect changes host or
downgrades `https → http`.

Blocked **unconditionally** → `error: "blocked_host"`:

- Private RFC1918 (`10/8`, `172.16/12`, `192.168/16`)
- Link-local incl. **cloud metadata** `169.254.169.254`
- IPv6 ULA (`fc00::/7`), link-local (`fe80::/10`), multicast
- `::ffff:`-mapped IPv4 in both dotted (`::ffff:169.254.169.254`) and hex (`::ffff:a9fe:a9fe`) forms

Loopback (`127.0.0.0/8`, `::1`, `localhost`) is **allowed by default** for
dev convenience; operators set `web.allow_loopback: false` for prod, after
which loopback returns `blocked_host`.

---

## Local development & testing

```bash
cargo build                  # build the web binary
cargo test                   # unit + wiremock integration + adversarial SSRF tests
cargo clippy -- -D warnings  # lints
cargo fmt --check            # formatting
```

The agent-facing authoring guide lives in
[`skills/index.md`](./skills/index.md) (served at runtime via
`directory::skills::get`). For the exact, authoritative field types, call
`engine::functions::info { function_id: "web::fetch" }` — the live schema
wins if it ever disagrees with this document.
