---
name: tailscale
description: >-
  Operate this host's Tailscale node from iii: connectivity and peers, routes
  and exit nodes, preferences, Taildrop file transfer, HTTPS certificates,
  publishing local services or the Console to the tailnet (Serve) or the
  internet (Funnel), accounts, tailnet lock, updates. Use it for anything a
  person would otherwise do with the `tailscale` CLI or the Tailscale app.
---

# tailscale

The tailscale worker drives the installed `tailscale` CLI on the Console host
and exposes its whole surface as typed functions, plus a Console page. Every
function returns structured data with keys, secrets, and capability maps
stripped; every command that changes network reachability or node behaviour
is approval-gated. Requires Tailscale installed on the host; most functions
also need the node signed in and connected.

Publishing has two visibilities. Serve is tailnet-only: reachable by devices
signed into this tailnet, with Tailscale identity headers on every request.
Funnel is public: reachable by anyone with the link, no identity headers,
ports 443/8443/10000 only, and it needs `allow_funnel` in the worker
configuration plus `confirm_public` on the request. Stopping a Funnel route
removes public access and keeps the tailnet route; stopping a Serve route
removes it entirely. The worker never runs `serve reset` or `funnel reset`
unless `serve::reset` is called with `confirm=true`.

## When to Use

- A person asks whether Tailscale is connected, which devices are on the
  tailnet, what this node's Tailscale IP or MagicDNS name is, or why a peer
  is slow (DERP relay vs direct path).
- A person wants a link or QR code to open the Console, or any local port,
  service, or directory, on another device.
- A person wants to route traffic through an exit node, accept or advertise
  subnet routes, change hostname, shields-up, SSH, DNS, or auto-update.
- A person wants to send a file to another device or collect files from the
  Taildrop inbox, get an HTTPS certificate for this node, switch accounts,
  check tailnet lock, or update the client.

## Boundaries

- Read functions (`status`, `peers::list`, `netcheck`, `ping`, `whois`,
  `dns::*`, `prefs::get`, `serve::list`, `exit-node::list`, …) are allowed by
  default. Everything else changes the node or the network and is
  approval-gated; do not chain approvals for a person, ask once per action.
- Never publish anything publicly (`mode: funnel`) without the person's
  explicit yes in that conversation, and say that anyone with the link can
  open it.
- Funnel needs a one-time tailnet-admin approval; `share`/`serve::add` return
  `stage: authorization_required` with the approval URL until that is done.
- `tailscale up` refuses flags when non-default preferences exist, so
  `connect` runs it bare; change preferences with `prefs::set`, never by
  reconnecting.
- Taildrive commands are rejected by the macOS GUI app (it manages Taildrive
  in its own settings); expect an error there.
- `serve::add` targets must be a local port, a loopback URL, or an absolute
  path; `file::send` paths and `cert` output paths must be absolute.
- The worker does not run `tailscale ssh`, `nc`, or `web`, and it does not
  edit the tailnet policy file.

## Functions

Node and session:

- `tailscale::status` — connectivity, node identity, health, Funnel allowed, peer counts, exit node in use, active routes.
- `tailscale::configuration` — non-secret worker settings plus the raw Serve configuration.
- `tailscale::connect` / `tailscale::disconnect` — `tailscale up` / `tailscale down`; connect returns a sign-in URL when the node needs a login.
- `tailscale::login` / `tailscale::logout` — start a sign-in (returns the browser URL) or expire the node key.
- `tailscale::version` — installed client version, optionally the latest upstream release.
- `tailscale::ip` — Tailscale IPs of this node or of a peer.
- `tailscale::netcheck` — UDP, IPv4/IPv6, NAT mapping, port mapping, preferred DERP and relay latencies.
- `tailscale::ping` — ping a peer at the Tailscale layer; each reply says DERP or direct.
- `tailscale::whois` — machine and user behind a Tailscale IP.
- `tailscale::dns::status` / `tailscale::dns::query` — MagicDNS and split-DNS configuration; resolve a name through the forwarder.

Peers and preferences:

- `tailscale::peers::list` — devices on the tailnet with IPs, OS, online state, tags, exit-node offers, relay, traffic counters.
- `tailscale::exit-node::list` / `tailscale::exit-node::suggest` / `tailscale::exit-node::set` — exit nodes on offer, the best one, and switching or clearing the one in use.
- `tailscale::prefs::get` / `tailscale::prefs::set` — read the node's preferences; change only the fields given (`tailscale set`).

Publishing:

- `tailscale::share` / `tailscale::share::stop` — publish or stop the iii Console on a port and path.
- `tailscale::serve::list` / `tailscale::serve::add` / `tailscale::serve::remove` — list every route; publish any local port, loopback URL, or directory; remove one route.
- `tailscale::serve::reset` — remove every Serve and Funnel route on this node; needs `confirm=true`.

Files and certificates:

- `tailscale::file::targets` / `tailscale::file::send` / `tailscale::file::receive` — Taildrop: who accepts files, send files to a device, move inbox files into a directory.
- `tailscale::cert` — fetch a Let's Encrypt certificate and key for one of this node's MagicDNS domains.
- `tailscale::drive::list` / `tailscale::drive::share` / `tailscale::drive::unshare` — Taildrive directory shares.

Administration:

- `tailscale::lock::status` — tailnet lock state and this node's tailnet-lock key.
- `tailscale::accounts::list` / `tailscale::accounts::switch` — accounts signed in on this device; switch the active one.
- `tailscale::update` — update the client, or `dry_run=true` to see what would change.
- `tailscale::bugreport` — a shareable diagnostic identifier for Tailscale support.
- `tailscale::metrics` — client metrics in Prometheus text format.
