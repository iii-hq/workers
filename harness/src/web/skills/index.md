---
type: index
title: web
description: Outbound HTTP(S) client on the iii bus — the single web::fetch trigger. Use instead of shell::exec curl. Authoring guide for an agent calling it: the minimal call, the full request/response envelope, the ok:true-vs-ok:false rule (HTTP 4xx/5xx are ok:true), an error→cause→fix table, the json-vs-body and response_format rules, and the SSRF guard (blocked ranges, pin-to-IP, per-hop redirect re-check, cross-origin auth stripping). Self-contained; meant for system-prompt injection — do not re-fetch.
functions:
  - web::fetch
---

> **Callable id:** `web::fetch` — pass to `agent_trigger { function: "web::fetch" }` (NOT the `iii://...` skill path; that's docs, not a function id). Use this **instead of** `shell::exec` with `curl`/`wget` for any HTTP request: you get a parsed `{ ok, status, headers, body }` envelope, enforced size/timeout caps, and server-side SSRF protection a shell `curl` doesn't have.

# Decide fast

| You want to… | Call |
|---|---|
| GET a page/API | `{ "url": "https://…" }` |
| Parse a JSON API response | `{ "url": "https://…", "response_format": "json" }` |
| POST/PUT JSON | `{ "url": "…", "method": "post", "json": { … } }` |
| Send a raw body / form | `{ "url": "…", "method": "post", "body": "…", "headers": { "content-type": "…" } }` |
| Download binary | `{ "url": "…", "response_format": "base64" }` |
| Run a shell `curl` | ❌ stop — use `web::fetch` |

# Minimal call

```jsonc
// agent_trigger { function: "web::fetch", payload: { "url": "https://api.github.com/zen" } }
// → { "ok": true, "status": 200, "body": "Design for failure.", ... }
```

That's the whole T0 path: a URL is the only required field. Everything else has a default.

# Read the result correctly (the #1 trap)

**`ok` tells you if the request COMPLETED, not whether the server was happy.**

- A 404 or 500 is a **successful fetch** → `ok: true`, `status: 404`. Branch on `status` for HTTP outcomes.
- `ok: false` means the request never produced a response (bad URL, blocked host, timeout, transport failure). Branch on `error` for those.

```jsonc
if (!r.ok)            → fetch failed; look at r.error  (see table below)
else if (r.status>=400) → server returned an HTTP error; look at r.status / r.body
else                  → success; use r.body or r.json
```

Do **not** treat `ok: true` as "2xx". Always check `status` too.

# Request fields

| Field | Default | Notes |
|---|---|---|
| `url` *(required)* | — | absolute `http(s)://` |
| `method` | `GET` | case-insensitive (`"post"` works); GET HEAD POST PUT PATCH DELETE OPTIONS |
| `headers` | `{}` | object; keys as you write them |
| `json` | — | structured payload; auto-stringified + sets `content-type: application/json`. **Wins over `body`.** |
| `body` | — | raw string body; use for non-JSON. Ignored on GET/HEAD. |
| `response_format` | `"text"` | `"text"` \| `"base64"` (binary) \| `"json"` (also parses into `json`) |
| `timeout_ms` | worker max (30000) | clamped DOWN to the ceiling; can't raise it |
| `max_bytes` | worker max (5 MiB) | over-cap body is truncated, not errored |
| `follow_redirects` | `true` | each hop re-checked against the SSRF blocklist |

# Response

**Success** (`ok: true`) — returned for *any* completed response, 2xx through 5xx:

```jsonc
{
  "ok": true,
  "status": 200,
  "status_text": "OK",
  "headers": { "content-type": "application/json", ... },  // keys LOWER-CASED; set-cookie joined with ", "
  "body": "<utf8 text | base64 when response_format=base64>",
  "json": { ... },            // only when response_format="json" AND parse succeeded AND not truncated
  "parse_error": "…",         // only when response_format="json" AND JSON.parse failed (body still set)
  "response_format": "json",
  "bytes_truncated": false,   // true when body hit max_bytes (NOT an error)
  "redirect_chain": ["https://…/a"]  // omitted when no redirects
}
```

**Failure** (`ok: false`) — the fetch never produced a response:

```jsonc
{ "ok": false, "error": "blocked_host", "message": "…" }   // no status field
```

## error → cause → fix

| `error` | Cause | Fix |
|---|---|---|
| `invalid_payload` | Payload failed schema (bad `method`, wrong types). `message` lists the bad fields. | Correct the named fields. |
| `invalid_url` | `url` isn't a parseable absolute `http(s)://` URL. | Pass a full absolute URL incl. scheme. |
| `blocked_host` | Target resolves to a private / link-local / cloud-metadata IP (SSRF guard). | Don't target internal/metadata hosts. For loopback in dev, the operator sets `web.allow_loopback`. |
| `timeout` | Slower than `timeout_ms` (or the 30 s ceiling). | Raise `timeout_ms` (up to ceiling) or shrink work via `max_bytes`. |
| `too_many_redirects` | More than `max_redirects` (5) hops. | Use the final URL directly, or set `follow_redirects: false` and read the `location` header. |
| `transport_error` | Connection refused/reset, TLS failure, DNS failure, or a redirect `Location` that won't parse. | Check host/port/cert; retry if transient. |

There is **no `too_large` error** — oversize responses come back `ok: true` with `bytes_truncated: true`. Branch on the flag, not on an error.

# Rules that save a turn

- **`json` vs `body`: set exactly one.** `json` wins if both are present and forces `content-type: application/json`. Use `body` + your own `content-type` for form/text/XML.
- **GET/HEAD ignore the body.** Putting `json`/`body` on a GET sends nothing — switch `method`.
- **`response_format: "json"` can succeed with `parse_error`.** Non-JSON (or truncated) bodies return `ok: true`, no `json`, `parse_error` set; the raw text is still in `body`. Check for `json` before reading it.
- **Caps only go down.** `timeout_ms`/`max_bytes` above the worker ceiling are clamped; you can't request more than the operator allows.
- **Response header keys are lower-cased.** Index `headers["content-type"]`, never `"Content-Type"`.
- **Auth headers don't survive a cross-origin redirect** (see below) — for an authed endpoint, hit the final URL directly.

# SSRF guard (server-side, not optional)

DNS is resolved **once**, every resolved address is checked, then the request is dialed at the **validated IP** (pinned `lookup` + TLS `servername`) — so the IP that passed the check is the IP connected to (no DNS-rebind window).

Blocked **unconditionally** → `error: "blocked_host"`:

- Private RFC1918 (`10/8`, `172.16/12`, `192.168/16`)
- Link-local incl. **cloud metadata** `169.254.169.254`
- IPv6 ULA (`fc00::/7`), link-local (`fe80::/10`), multicast
- **`::ffff:`-mapped IPv4** in both dotted (`::ffff:169.254.169.254`) and hex (`::ffff:a9fe:a9fe`) forms

Loopback (`127.0.0.0/8`, `::1`, `localhost`) is **allowed by default** (dev convenience); operators flip `web.allow_loopback: false` for prod, after which loopback returns `blocked_host`.

On each 3xx the `Location` is re-resolved and **re-validated** before following, and `Authorization` / `Cookie` / `Proxy-Authorization` are **stripped** when the redirect changes host or downgrades `https → http`.

# Examples

```jsonc
// Parse a JSON API
{ "url": "https://api.example.com/status", "response_format": "json" }
// → { ok:true, status:200, json:{ healthy:true } }

// POST JSON (content-type set for you)
{ "url": "https://api.example.com/things", "method": "post",
  "json": { "name": "x" }, "response_format": "json" }

// Authenticated GET (auth stays only because there's no cross-host redirect)
{ "url": "https://api.example.com/me",
  "headers": { "authorization": "Bearer TOKEN" }, "response_format": "json" }

// Bounded download of a maybe-large file
{ "url": "https://example.com/big.log", "max_bytes": 65536 }
// → { ok:true, body:"<first 64 KiB>", bytes_truncated:true }
```

# Authoritative schema

This page covers behavior the schema can't. For exact field types, call
`engine::functions::info { function_id: "web::fetch" }` — the live schema wins if it ever disagrees with this page.

# Related

- [`shell/index`](iii://shell) — local filesystem + process ops; `web::fetch` is the network counterpart (use it, not `shell::exec curl`).
- [`sandbox/index`](iii://sandbox) — code *inside* a sandbox reaches the host engine via the boot-time `III_ENGINE_URL` rewrite, not `web::fetch`.
