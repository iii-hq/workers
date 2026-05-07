# sandbox-cloudflare bridge

Thin Cloudflare Worker that exposes HTTPS routes corresponding to every `sandbox::cloudflare::*` function, calling `@cloudflare/sandbox`'s `getSandbox(env.Sandbox, id)` underneath. The iii worker in the parent directory talks to this bridge — that's the only way to reach a CF Sandbox from outside the Cloudflare Workers runtime.

## Why a bridge

CF Sandbox is a Durable Object that owns a Container. Both run inside the Cloudflare Workers V8 isolate runtime. The iii engine ships as a Rust/Node binary on Linux/macOS hosts — it can't host a CF Worker. This bridge is the smallest amount of CF-native code that lets a host-side iii worker reach a Sandbox.

## Routes

| HTTP | Path | Backed by |
|---|---|---|
| POST | `/create` | `getSandbox(env.Sandbox, id)` |
| POST | `/exec` | `sandbox.exec(cmd, opts)` |
| POST | `/stop` | `sandbox.destroy()` |
| GET | `/list` | (TODO — SDK does not currently expose a list primitive) |
| POST | `/expose-port` | `sandbox.exposePort(port, opts)` |
| POST | `/fs/read` | `sandbox.readFile(path)` |
| POST | `/fs/write` | `sandbox.writeFile(path, bytes, { mode })` |

All routes require `Authorization: Bearer <CLOUDFLARE_BRIDGE_TOKEN>` (shared secret with the iii worker).

## Deploy

```bash
npm install
wrangler secret put CLOUDFLARE_BRIDGE_TOKEN     # paste the same token you'll set in CLOUDFLARE_BRIDGE_TOKEN on the iii worker side
wrangler deploy
```

`wrangler deploy` prints the bridge URL (e.g. `https://sandbox-cloudflare-bridge.<account>.workers.dev`). Set that as `CLOUDFLARE_BRIDGE_URL` on the iii worker side.

## Status

v0.1 ships the auth check, route shell, and SDK re-export of the `Sandbox` Durable Object class. The handler bodies that call `getSandbox()` are stubbed and return HTTP 501 until the next iteration. Routes/auth are stable; SDK wiring is the only remaining work.

## Why no tests here

CF Worker testing requires `miniflare` or `@cloudflare/vitest-pool-workers` plus a containerd-style runtime to exercise the Sandbox SDK end-to-end. Adding it complicates v0 without proportionate value while the route bodies are stubbed. The iii worker side has full smoke tests against a mocked bridge response.
