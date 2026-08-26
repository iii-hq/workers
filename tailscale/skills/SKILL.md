---
name: tailscale
description: >-
  Publish the local iii Console over Tailscale Serve (tailnet only) or, with
  two explicit opt-ins, Tailscale Funnel (public); use it when a person wants
  the Console on a phone or another device, not for general networking.
---

# tailscale

The tailscale worker drives the installed `tailscale` CLI to share the local
Console. A Serve route is reachable only by devices the tailnet policy admits
and carries Tailscale identity headers; a Funnel route is reachable by anyone
on the internet and carries none. The worker returns the HTTPS link as text
and as a QR code, and it removes exactly the route it is asked to, never the
node's whole Serve configuration. Requires Tailscale installed and signed in on
the Console host with MagicDNS and HTTPS certificates enabled.

## When to Use

- A person asks to open the Console on their phone, tablet, or another
  machine, or asks for a link or QR code to it.
- A person asks whether Tailscale is connected, what this node's MagicDNS name
  is, or which Console routes are currently published.
- A previously shared route should be taken down again.

## Boundaries

- Tailnet-only sharing (`mode: serve`) is the default; prefer it for every
  remote-control session.
- Public sharing (`mode: funnel`) needs `allow_funnel: true` in the worker
  configuration and `confirm_public: true` on the request. Do not set
  `confirm_public` on a person's behalf without their explicit yes, and say
  that the Console becomes reachable by anyone with the link.
- Funnel accepts HTTPS ports 443, 8443, and 10000 only. When the tailnet policy
  has not enabled Funnel for this node, `tailscale::share` returns
  `stage: authorization_required` with the approval page as the URL; a person
  must approve it in Tailscale before sharing again.
- The worker publishes the Console only; it does not expose other local
  services, change ACLs, or manage Tailscale itself. Ask a person to fix
  connectivity when `tailscale::status` reports the client is not running.

## Functions

- `tailscale::status` — connectivity, node name and MagicDNS name, Tailscale IPs, health notices, whether Funnel is allowed, and every active route with its URL.
- `tailscale::configuration` — the non-secret worker settings plus the raw Serve configuration.
- `tailscale::connect` — bring this node onto the tailnet (`tailscale up`), or return the Tailscale sign-in URL when the node still needs a login.
- `tailscale::disconnect` — take this node off the tailnet (`tailscale down`); shared routes stop answering until it connects again.
- `tailscale::share` — publish the Console on an HTTPS port and path, returning the link and its QR code, or the Funnel authorization page when that step is still needed.
- `tailscale::share::stop` — remove one route by mode, HTTPS port, and path; a Funnel route loses both its public and tailnet listener.

`tailscale::status` and `tailscale::configuration` are allowed by default;
`tailscale::connect`, `tailscale::disconnect`, `tailscale::share`, and
`tailscale::share::stop` are approval-gated because they change what the
network can reach.
