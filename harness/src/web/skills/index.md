---
type: index
title: web
description: Outbound HTTP(S) client on the iii bus. Authoring guide for the single web::fetch trigger — the structured request/response envelope, the json-vs-body and response_format rules, truncation vs error semantics, and the server-side SSRF guard (blocked ranges, pin-to-IP, per-hop redirect re-check, cross-origin auth stripping). Single self-contained skill — meant for system-prompt injection; do not re-fetch.
functions:
  - web::fetch
---

> **Callable id:** `web::fetch` — pass this to `agent_trigger { function: "web::fetch" }` (NOT the skill path from `directory::skills::list`; that's documentation, not a function id). Use this **instead of** `shell::exec` with `curl`/`wget` for any HTTP request — you get a parsed `{ ok, status, headers, body }` envelope, enforced size/timeout caps, and SSRF protection that curl-in-a-shell does not.

# When to use

A guarded outbound HTTP client. Reach for it to call a JSON API, scrape a page, hit a health endpoint, or POST a webhook — anything that would otherwise be a `curl`. The request is validated, capped, and routed through an SSRF blocklist server-side, so a model-chosen URL can't be turned into a request against your private network or cloud-metadata endpoint.

| Question | Use this |
|----------|----------|
| GET a page / API | `web::fetch { url }` |
| POST/PUT JSON | `web::fetch { url, method, json }` |
| Send a custom body / headers | `web::fetch { url, method, headers, body }` |
| Download binary | `web::fetch { url, response_format: "base64" }` |
| Auto-parse a JSON response | `web::fetch { url, response_format: "json" }` |
| Run a shell `curl` | ❌ — use `web::fetch`, not `shell::exec` |

# Fetch the live schema, don't trust this page

For exact parameter and response shapes, call:

```jsonc
// engine::functions::info { function_id: "web::fetch" }
// → request_format / response_format / description JSON
```

`web::fetch` publishes a full request schema (zod → JSON-schema). This page is for the cross-cutting behaviors and traps that the schema alone won't tell you — the envelope semantics, the SSRF rules, and the precedence between overlapping fields. Don't reconstruct field names from this page if the engine disagrees; the schema is the source of truth.

# The one function

`web::fetch` — fetch a URL over HTTP(S) and return a structured envelope. **It never throws** — success and failure are both returned as a value (the `ok` discriminant), so always branch on the result, not on a thrown error.

### Request

```jsonc
{
  "url": "https://api.example.com/v1/things",  // required, absolute http(s)://
  "method": "post",                             // optional, default GET, case-insensitive ("get" works)
  "headers": { "x-trace": "abc" },              // optional
  "json": { "name": "thing" },                  // optional structured payload (see precedence below)
  "body": "raw string body",                    // optional raw body (mutually exclusive with json)
  "timeout_ms": 5000,                            // optional, CAPPED by the worker ceiling
  "max_bytes": 1048576,                          // optional, CAPPED by the worker ceiling
  "follow_redirects": true,                      // optional, default true
  "response_format": "json"                      // optional: "text" (default) | "base64" | "json"
}
```

### Response — success

```jsonc
{
  "ok": true,
  "status": 200,
  "status_text": "OK",
  "headers": { "content-type": "application/json", ... },
  "body": "<string — utf8 text, or base64 when response_format=base64>",
  "json": { ... },              // present only when response_format="json" AND parse succeeded
  "parse_error": "…",           // present only when response_format="json" AND parse FAILED (body still set)
  "response_format": "json",
  "bytes_truncated": false,     // true when the body hit max_bytes (NOT an error — see below)
  "redirect_chain": ["https://…/a", "https://…/b"]  // omitted when no redirects happened
}
```

### Response — failure

```jsonc
{ "ok": false, "error": "blocked_host", "message": "…", "status": 502 }
```

`error` is one of: `invalid_payload`, `invalid_url`, `blocked_host`, `timeout`, `too_many_redirects`, `transport_error`. **Branch on `error`, not on `message` text** — the message is for humans and may change.

# Behaviors that aren't obvious from the schema

### `json` wins over `body` — set exactly one

If you set `json`, the worker stringifies it and forces `content-type: application/json`; do **not** also set `body`. If both are present, `json` wins and `body` is ignored. For a non-JSON payload (form-encoded, plain text, XML) use `body` + your own `content-type` header.

### Oversize is TRUNCATED, not an error

A response larger than `max_bytes` (or the worker's `max_response_bytes` ceiling) comes back as `ok: true` with the body cut to the cap and **`bytes_truncated: true`**. There is no `too_large` error — `too_large` is reserved in the enum but never emitted. If you need the whole body, raise `max_bytes` (up to the ceiling) and check `bytes_truncated` before trusting the body is complete.

### `response_format: "json"` can succeed-with-`parse_error`

When you ask for `"json"` and the body isn't valid JSON, the call still returns `ok: true` — but with `parse_error` set and `json` absent. The raw text is still in `body`. Check for `json` before reading it; fall back to `body` + `parse_error` otherwise. (`"text"` is the default; `"base64"` is for binary downloads.)

### Caps are hard ceilings — asking for more is silently clamped

`timeout_ms` and `max_bytes` can only ever *lower* the limit. The worker's `web.max_timeout_ms` (default 30000), `web.max_response_bytes` (default 5 MiB), and `web.max_redirects` (default 5) are hard ceilings; a per-call value above them is clamped down, not honored. This stops one runaway fetch from eating the whole budget.

### Method is case-insensitive

`"get"`, `"Get"`, `"GET"` all normalize to `GET`. Allowed: `GET HEAD POST PUT PATCH DELETE OPTIONS`. Anything else fails validation with `error: "invalid_payload"`.

# SSRF guard — what gets blocked and why

The worker resolves DNS **once**, validates **every** resolved address against the blocklist, and then dials the **validated IP** directly (pinned `lookup` + TLS `servername`). That closes the DNS-rebinding window: the IP that passed the check is the IP that's connected to.

Blocked **unconditionally** (returns `error: "blocked_host"`):

- Private RFC1918 (`10/8`, `172.16/12`, `192.168/16`)
- Link-local incl. **cloud metadata** `169.254.169.254` / `169.254.0.0/16`
- IPv6 ULA (`fc00::/7`), link-local (`fe80::/10`), multicast
- **`::ffff:`-mapped IPv4** in *both* dotted (`::ffff:169.254.169.254`) and hex (`::ffff:a9fe:a9fe`) forms — a common bypass that's explicitly closed

Loopback (`127.0.0.0/8`, `::1`, `localhost`) is **allowed by default** (`web.allow_loopback: true`) because the harness's own dev workflow targets loopback workers. In strict deployments set `web.allow_loopback: false`; loopback then returns `blocked_host` with a message hinting at the flag.

### Redirects are re-checked every hop, and auth is stripped cross-origin

- Each 3xx `location` is re-resolved and re-validated against the blocklist before following — a public URL that 302s to `169.254.169.254` is caught at the hop, not just at the entry URL.
- `Authorization` and `Cookie` headers are **stripped** when a redirect crosses to a different host, or downgrades `https → http`. Don't rely on your auth header surviving a cross-host redirect — it won't.
- More than `max_redirects` hops returns `error: "too_many_redirects"`. The walked URLs come back in `redirect_chain`.

# Examples

```jsonc
// GET JSON and auto-parse
// web::fetch
{ "url": "https://api.example.com/status", "response_format": "json" }
// → { ok:true, status:200, json:{ healthy:true }, ... }

// POST a JSON body (content-type set for you)
// web::fetch
{ "url": "https://api.example.com/things", "method": "post", "json": { "name": "x" }, "response_format": "json" }

// Capped download of a possibly-large text file
// web::fetch
{ "url": "https://example.com/big.log", "max_bytes": 65536 }
// → { ok:true, body:"<first 64 KiB>", bytes_truncated:true }
```

# Related

- [`shell/index`](iii://shell) — local filesystem + process ops; `web::fetch` is the network counterpart (use it instead of `shell::exec curl`).
- [`sandbox/index`](iii://sandbox) — to fetch from *inside* an ephemeral VM, the sandbox reaches the host engine via the boot-time `III_ENGINE_URL` rewrite, not `web::fetch`.
