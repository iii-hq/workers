---
name: quick-tunnel
description: Acquire and observe expiring leases on operator-authorized public tunnels (cloudflared Quick Tunnels or OpenTunnel) through iii, without choosing arbitrary origins or managing processes directly.
---

# Quick Tunnel

Use this worker only when a development webhook service needs a public ephemeral HTTPS URL. Acquiring exposes the entire configured local service publicly: require user approval. Do not run cloudflared or download binaries yourself.

1. Discover and inspect `quick-tunnel::acquire`, `quick-tunnel::release`, `quick-tunnel::status`, and the observable function `quick-tunnel::on-changed` (private; its calls are tunnel snapshots).
2. Observe `quick-tunnel::on-changed` **before** acquiring, with a filter on `input.tunnel_id` (default `webhooks`) and `capture: ['output']` (your target reads each snapshot at `payload.output`). It takes no args. Observer metadata reaches your target with every snapshot; use the engine's wake primitive for future work.
3. Acquire with a stable opaque `consumer_id`, authorized `tunnel_id`, and future RFC3339 `expires_at`. No origin, port, executable, arguments or credentials are accepted. Repeated acquire renews the same lease, never shortens expiry.
4. Read status once after acquiring to recover races. Use `public_url` only for `ready`. `starting` and `reconnecting` are not failures but are not ready; `failed` includes sanitized diagnostics. `status` may carry `provider` (absent means cloudflared) and `leases[].proxy_protocol`. Do not poll: react to changed events.
5. New generations are opaque UUIDs. With cloudflared they change URLs; with OpenTunnel the URL stays the same, so detect change by generation, never by URL. GitHub integration must update its webhook when the ready generation changes; validate webhook signatures in the HTTP receiver.
6. Release your lease when finished. Never release another consumer's lease. Last lease stops the child; expiration is automatic without an agent. Dispose observers you no longer need.

The operator picks the provider. With `provider: cloudflared` (default) the cloudflared binary is supplied by the operator, not this worker, and a missing one is a real observable failure. With `provider: opentunnel` no binary is needed: TLS ends inside the worker and the local service sees every visitor as `127.0.0.1`, unless the operator listed its target in `proxy_protocol` (then every connection starts with a PROXY protocol v2 header and `status` lists the lease with `proxy_protocol: "v2"`). Never trust loopback peers or forwarding headers. Neither has an SLA; they are not production endpoints. Status exposes no Cloudflare secrets, but consumer IDs must not contain secrets either.

After engine reconnection reconcile status; events are best-effort and deliveries are not a durable queue. Configuration is central and changes apply on worker restart. Never expose engine ports or broaden the target allowlist autonomously.
