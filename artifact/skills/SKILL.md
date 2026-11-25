---
name: artifact
description: >-
  Store versioned single-file artifacts — HTML designs and clickable
  prototypes, markdown RFCs and docs, SVG drawings — and share one frozen
  version through an expiring secret link, locally or through a quick-tunnel.
---

# artifact

An artifact is one self-contained file under a stable 8-character id: an HTML
page (a screen design or a clickable prototype), a markdown document (an RFC,
a spec, a write-up) or an SVG drawing. Every `artifact::update` appends a
version; old versions stay readable.

Sharing freezes one version behind a 64-character secret token that expires
after a number of hours. The link works on this machine immediately
(`local_url`) and, when the operator has set up the quick-tunnel target,
publicly (`public_url`). Whoever holds the link can view it, so share only
what may be seen, and revoke links that are no longer needed.

## When to Use

- Prototype a screen or flow as one HTML file and iterate on it by version.
- Write an RFC or design doc in markdown and send it to someone for review.
- Preview an artifact in the console browser: share with `public: false`
  and open `local_url` with `browser::sessions::start`.
- Hand a reviewer a link that stops working on its own after a few hours.

## Writing content that renders when shared

Shared pages run under a strict sandbox: nothing loads from the network.

- HTML: inline all CSS and JS; no CDN scripts, web fonts or remote images
  (use `data:` URIs or inline SVG). Scripts run, network calls and form
  submissions do not.
- Markdown: raw HTML is shown as text; use markdown syntax only.
- SVG: no script runs.

## Functions

- `artifact::create { title, kind?, format?, content, note? }` — version 1.
  `format` is `html` (default), `markdown` or `svg`; `kind` is a free
  lowercase label such as `design`, `prototype`, `rfc` or `doc`.
- `artifact::update { id, content, note?, title? }` — the next version.
- `artifact::get { id, version? }` — record plus one version's content.
- `artifact::list { kind?, limit? }` — summaries, newest first.
- `artifact::delete { id }` — removes every version and revokes its shares.
- `artifact::share { id, version?, hours?, public? }` — a frozen, expiring
  link. Returns `token`, `local_url`, `public_url` (when the tunnel is
  ready), `expires_at_utc` and, if publishing failed, `tunnel_error`.
- `artifact::shares { id? }` — live shares with their current URLs. A
  quick-tunnel URL can change when the tunnel restarts; read it here again
  instead of reusing an old one.
- `artifact::revoke { token }` — the link stops working at once; accepts the
  full link too.

## Boundaries

- One file per version, up to `max_content_bytes` (2 MiB by default).
- Shares last 1 to `max_share_hours` hours (72 by default).
- Public links need the operator to map the `artifacts` quick-tunnel target
  to the share listener. Without it, `share` still returns a working
  `local_url` and explains the missing setup in `tunnel_error`. Never change
  the quick-tunnel targets yourself.
