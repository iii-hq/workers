---
name: quick-tunnel
description: Acquire and observe expiring leases on operator-authorized cloudflared Quick Tunnels through iii, without choosing arbitrary origins or managing processes directly.
---

# Quick Tunnel

Use this worker only when a development webhook service needs a public ephemeral HTTPS URL. Acquiring exposes the entire configured local service publicly: require user approval. Do not run cloudflared or download binaries yourself.

1. Discover and inspect `quick-tunnel::acquire`, `quick-tunnel::release`, `quick-tunnel::status`, and trigger type `quick-tunnel::changed`.
2. Register a bounded changed binding **before** acquiring, filtered by `tunnel_id` (default `webhooks`). Preserve callback metadata and namespace; use the engine's wake primitive for future work.
3. Acquire with a stable opaque `consumer_id`, authorized `tunnel_id`, and future RFC3339 `expires_at`. No origin, port, executable, arguments or credentials are accepted. Repeated acquire renews the same lease, never shortens expiry.
4. Read status once after acquiring to recover races. Use `public_url` only for `ready`. `starting` and `reconnecting` are not failures but are not ready; `failed` includes sanitized diagnostics. Do not poll: react to changed events.
5. New process generations are opaque UUIDs and can change URLs. GitHub integration must update its webhook when the ready generation changes; validate webhook signatures in the HTTP receiver.
6. Release your lease when finished. Never release another consumer's lease. Last lease stops the child; expiration is automatic without an agent. Unregister unneeded bindings.

Prerequisite cloudflared is supplied by the operator, not this worker. Missing prerequisite is a real observable failure. Quick Tunnels have no SLA and are not production endpoints. Status exposes no Cloudflare secrets, but consumer IDs must not contain secrets either.

After engine reconnection reconcile status; events are best-effort and callbacks are not a durable queue. Configuration is central and changes apply on worker restart. Never expose engine ports or broaden the target allowlist autonomously.
