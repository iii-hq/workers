# sandbox-cloudflare

Narrow iii worker that exposes [Cloudflare Sandbox](https://developers.cloudflare.com/sandbox/) under the canonical `sandbox::cloudflare::*` ABI. Unlike the other workers in this family, `sandbox-cloudflare` ships **two artifacts**:

1. **iii worker** (this folder, top-level files) — runs on a host the iii engine controls. Registers `sandbox::cloudflare::*` functions. Talks HTTPS to (2).
2. **CF Worker bridge** (`bridge/` subfolder) — separately deployed via `wrangler deploy`. Hosts the `Sandbox` Durable Object class from `@cloudflare/sandbox`. Receives HTTPS calls from (1) and drives the Container.

The bridge exists because CF Sandbox lives inside the Workers V8 runtime — there is no way to reach `getSandbox()` from a host-side process. The bridge is the smallest amount of CF-native code needed.

## Functions

| Function id | Purpose |
|---|---|
| `sandbox::cloudflare::create` | Boot a sandbox; returns `{sandbox_id, image, capabilities}` |
| `sandbox::cloudflare::exec` | Run a command inside a live sandbox |
| `sandbox::cloudflare::stop` | Tear down a sandbox |
| `sandbox::cloudflare::list` | Enumerate live sandboxes plus concurrency status |
| `sandbox::cloudflare::expose_port` | Public URL for a port (requires custom domain on the bridge) |
| `sandbox::cloudflare::fs::read` | Read a file out of the sandbox |
| `sandbox::cloudflare::fs::write` | Write a file into the sandbox |

`create` advertises capabilities `["expose_port", "fs"]`. CF Sandbox does not ship `snapshot` or `branch`; callers that depend on those should pick a different provider.

## Deploy (two steps)

1. **Deploy the bridge** (see `bridge/README.md`):
   ```bash
   cd bridge && npm install
   wrangler secret put CLOUDFLARE_BRIDGE_TOKEN
   wrangler deploy
   ```
   Wrangler prints a bridge URL like `https://sandbox-cloudflare-bridge.<account>.workers.dev`.

2. **Run the iii worker** with the bridge URL + shared secret in the environment:
   ```bash
   export CLOUDFLARE_BRIDGE_URL="https://sandbox-cloudflare-bridge.<account>.workers.dev"
   export CLOUDFLARE_BRIDGE_TOKEN="<same token you set with wrangler secret>"
   iii worker add sandbox-cloudflare
   ```

## Configuration

`config.yaml` next to the binary, or set `SANDBOX_CLOUDFLARE_CONFIG` to a path:

```yaml
bridge_url_env: CLOUDFLARE_BRIDGE_URL
bridge_token_env: CLOUDFLARE_BRIDGE_TOKEN
max_concurrent_sandboxes: 10
default_idle_timeout_secs: 300
image_allowlist: []
```

The worker fails fast at startup if either env var is missing.

## S-codes

| Code | Cause |
|---|---|
| `S100` | Image not in `image_allowlist` |
| `S400` | Concurrency cap reached |
| `S404` | Capability not supported |
| `S500` | Bridge returned 429 |
| `S501` | Bridge returned 402 / quota exhausted |
| `S502` | Bridge returned 5xx (or stub bodies still in place) |
| `S503` | Bridge returned 401 / 403 (auth) |

## Status

v0.1 ships the iii worker side end-to-end (function registrations, types, error mapping, concurrency cap, smoke test) and the bridge route shell + auth check. Both halves' stub bodies return `S502` / HTTP 501 until the next iteration wires `@cloudflare/sandbox`'s `getSandbox()` calls. The wire protocol between worker and bridge is stable.
