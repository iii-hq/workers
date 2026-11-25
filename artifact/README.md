# artifact

Store versioned single-file artifacts — HTML designs and clickable
prototypes, markdown RFCs and docs, SVG drawings — and share one frozen
version through an expiring, token-guarded link.

## What it does

- **Artifacts**: one self-contained file under a stable 8-character id.
  `artifact::update` appends a version; earlier versions stay readable.
- **Shares**: `artifact::share` freezes a version behind a 256-bit secret
  token with an expiry (1 to `max_share_hours` hours). The link is served by
  the worker's own loopback listener at `/s/<token>` and, when the operator
  has configured it, published through [`quick-tunnel`](../quick-tunnel).
- **Expiry**: an expired link answers `410 Gone`. A sweeper drops expired
  shares every minute. The tunnel stays up `tunnel_grace_minutes` after the
  last public share ends, so a new share soon after reuses the same address,
  then its lease is released.
- **No dead links**: a fresh `*.trycloudflare.com` hostname takes up to a
  minute to reach DNS. `share` asks a public resolver directly (never the
  host's cache) and hands out the public URL only once it resolves; until
  then the share reports `tunnel_status: "warming"`.

## Functions

| Function | Purpose |
|---|---|
| `artifact::create` | Create an artifact (`html`, `markdown` or `svg`) at version 1. |
| `artifact::update` | Store the next version. |
| `artifact::get` | Read the record and one version's content. |
| `artifact::list` | List summaries, newest first, optionally by kind. |
| `artifact::delete` | Delete an artifact and revoke its shares. |
| `artifact::share` | Create an expiring link to one frozen version. |
| `artifact::shares` | List live shares with their current URLs. |
| `artifact::revoke` | Revoke a link at once. |

## Console UI

- **Chat card** for `artifact::create`, `update`, `get` and `share`: a
  sandboxed live preview with **Share…** (version, lifetime, public or
  local) and **Open**; a share shows its links with copy buttons.
- **Artifacts page**: every artifact, any version's preview, Share…,
  Delete, and the live links with Revoke.
- **Context section** in the chat side panel: recent artifacts and their
  live-link count; a row opens the artifact on the page.
- **Settings form** for the `artifact` configuration entry.

Previews run in an `<iframe sandbox>` without `allow-same-origin`, with the
same Content-Security-Policy the share listener sends.

## Security model

Shared content is untrusted (an agent wrote it) and is read by whoever holds
the link, so:

- The listener binds a **loopback** address only and serves a single route.
  Nothing else of the engine is reachable through the tunnel.
- Tokens carry 256 bits of OS entropy; anything not shaped like a token is
  rejected before the store is read.
- Every response is sent with a CSP `sandbox` (opaque origin, no cookies or
  storage) and `default-src 'none'`: shared pages cannot fetch, load remote
  resources or submit forms. Only HTML may run its own inline script.
  Markdown escapes raw HTML; SVG runs no script.
- Responses also carry `Cache-Control: no-store`, `X-Robots-Tag: noindex`,
  `Referrer-Policy: no-referrer`, `X-Content-Type-Options: nosniff` and
  `X-Frame-Options: DENY`.

## Configuration

Registered with the `configuration` worker under the id `artifact`.

| Field | Default | Meaning |
|---|---|---|
| `listen_addr` | `127.0.0.1:3120` | Loopback address of the share listener (restart to change). |
| `tunnel_id` | `artifacts` | quick-tunnel target public shares go through. |
| `default_share_hours` | `24` | Lifetime when the caller names none. |
| `max_share_hours` | `72` | Longest lifetime allowed (at most 720). |
| `max_content_bytes` | `2097152` | Largest content per version. |
| `max_versions` | `100` | Most versions per artifact. |
| `max_list` | `200` | Most rows per `artifact::list`. |
| `tunnel_ready_timeout_ms` | `45000` | How long `share` waits for the tunnel and its DNS. |
| `tunnel_grace_minutes` | `30` | Keep the tunnel this long after the last public share, so a new share reuses the same address (0 releases at once). |
| `dns_check_resolver` | `1.1.1.1:53` | Public resolver asked whether a fresh tunnel hostname resolves before the link is handed out (empty disables; restart to change). |

### Publishing links

Public links need an operator-approved quick-tunnel target pointing at the
share listener. Add it to the quick-tunnel configuration and restart that
worker:

```yaml
targets:
  webhooks: http://127.0.0.1:3112
  artifacts: http://127.0.0.1:3120
```

Either quick-tunnel provider (`cloudflared` or `opentunnel`) works. Do not
list `artifacts` under `proxy_protocol`: the share listener speaks plain HTTP.
Without the target, `artifact::share` still returns a working `local_url` and
reports the missing setup in `tunnel_error`.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
```
