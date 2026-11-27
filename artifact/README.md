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

Shared content is untrusted (an agent wrote it, possibly steered by content
it read) and is read by whoever holds the link, so:

- **Two loopback listeners.** The public one (`listen_addr`, the only thing
  the quick-tunnel target may point at) serves shares made with
  `public: true` and answers `404` for any other token, exactly like an
  unknown one. The local one (`local_listen_addr`) serves every share for
  previews on this machine. Each answers a single path, `/s/<token>`;
  nothing else of the engine is reachable.
- **A frame the content cannot escape.** HTML and SVG never become the
  top-level page: the link serves a fixed, script-free shell that embeds the
  content in an `<iframe sandbox srcdoc>` without `allow-top-navigation`,
  `allow-popups` or `allow-same-origin`. Content can run its inline script
  but cannot redirect the viewer (no phishing hop on a trusted-looking
  host), open windows, or read the link token.
- **No network.** The shell's CSP, inherited by the frame, is
  `default-src 'none'` with `connect-src`, `frame-src` and `form-action`
  `'none'`: nothing loads, nothing is sent, and the frame cannot navigate
  off-site. Markdown escapes raw HTML and runs no script; SVG runs no
  script.
- **Tokens and passcodes.** Tokens carry 256 bits of OS entropy; anything not
  shaped like a token is rejected before the store is read. A share may
  require a passcode (stored as an iterated, salted SHA-256, checked in
  constant time, five wrong guesses per ten minutes) and may be single-view.
- **Limits and audit.** Requests are rate-limited per token and per
  listener (`429`). Each share counts its views, with the time and, for
  tunnelled views, the country Cloudflare reports; the viewer's IP is never
  stored.
- **Headers.** `Cache-Control: no-store`, `X-Robots-Tag: noindex`,
  `Referrer-Policy: no-referrer`, `X-Content-Type-Options: nosniff`,
  `X-Frame-Options: DENY`; every page carries a banner naming its source.
- **Revocation fails closed.** The link key is deleted before the list row,
  so an interrupted revoke leaves a dead link, never a live one that cannot
  be revoked again. Ended links answer `410` for 7 days; revoked ones `404`.
- **Console previews** run in an `<iframe sandbox>` without
  `allow-same-origin`; the policy is a trusted prefix ahead of the content,
  never spliced into it.

What this does not cover: a quick tunnel terminates TLS at Cloudflare,
which sees the content, and `trycloudflare.com` has no SLA. Do not share
confidential material through it; use a named tunnel on your own domain.
Tokens are stored in clear in the worker's `state` scope so a link can be
shown again after the tunnel's address changes; anyone who can read that
scope can already read the artifacts themselves.

## Configuration

Registered with the `configuration` worker under the id `artifact`.

| Field | Default | Meaning |
|---|---|---|
| `listen_addr` | `127.0.0.1:3120` | Public listener: public shares only; the quick-tunnel target (restart to change). |
| `local_listen_addr` | `127.0.0.1:3121` | Local listener: every share, for previews; never tunnel it (restart to change). |
| `tunnel_id` | `artifacts` | quick-tunnel target public shares go through. |
| `default_share_hours` | `24` | Lifetime when the caller names none. |
| `max_share_hours` | `72` | Longest lifetime allowed (at most 720). |
| `max_content_bytes` | `2097152` | Largest content per version. |
| `max_versions` | `100` | Most versions per artifact. |
| `max_list` | `200` | Most rows per `artifact::list`. |
| `tunnel_ready_timeout_ms` | `45000` | How long `share` waits for the tunnel and its DNS (at most 120000). |
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
reports the missing setup in `tunnel_error`. Never point a tunnel at
`local_listen_addr`: it serves local-only shares too.

When starting the worker by hand from inside another worker's shell, clear
the inherited `III_*` variables first (`env -i ...`): an inherited
`III_CONFIG_NAME` makes the worker register its configuration over that
worker's entry.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
```
