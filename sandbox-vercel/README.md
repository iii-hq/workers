# sandbox-vercel

Narrow iii worker that wraps [Vercel Sandbox](https://vercel.com/docs/vercel-sandbox) (Firecracker microVMs on Vercel's "Hive" infrastructure) via the Vercel REST API. Registers the canonical `sandbox::*` ABI under the `sandbox::vercel::*` namespace so callers can spawn and drive Vercel sandboxes through `iii.trigger(...)` without depending on `@vercel/sandbox`.

The same ABI is implemented by every sandbox provider worker in this repo (`sandbox-e2b`, `sandbox-daytona`, `sandbox-morph`, `sandbox-modal`, `sandbox-cf`, ...). Callers swap providers by changing the function-id prefix.

## Functions

| Function id | Purpose |
|---|---|
| `sandbox::vercel::create` | Boot a sandbox; returns `{sandbox_id, image, capabilities}` |
| `sandbox::vercel::exec` | Run a command inside a live sandbox |
| `sandbox::vercel::stop` | Tear down a sandbox |
| `sandbox::vercel::list` | Enumerate live sandboxes plus concurrency status |
| `sandbox::vercel::snapshot` | Snapshot a sandbox (Vercel shuts the parent down after) |
| `sandbox::vercel::expose_port` | Public URL for a port (must be in `ports` at create time) |
| `sandbox::vercel::fs::read` | Read a file out of the sandbox |
| `sandbox::vercel::fs::write` | Write a file into the sandbox |

`create` advertises capabilities `["snapshot", "expose_port", "fs"]`. `branch` is not registered — Vercel Sandbox doesn't ship branching.

## Configuration

`config.yaml` next to the binary, or set `SANDBOX_VERCEL_CONFIG` to a path:

```yaml
api_base: "https://api.vercel.com"
oidc_token_env: VERCEL_OIDC_TOKEN
fallback_token_env: VERCEL_TOKEN
team_id_env: VERCEL_TEAM_ID
project_id_env: VERCEL_PROJECT_ID
max_concurrent_sandboxes: 10
default_idle_timeout_secs: 300
default_runtime: node24
image_allowlist: []
```

The worker prefers `VERCEL_OIDC_TOKEN` (auto-injected in Vercel-deployed projects, 12 h dev token via `vercel env pull`). Falls back to `VERCEL_TOKEN + VERCEL_TEAM_ID + VERCEL_PROJECT_ID`. Fails fast at startup if neither is set.

## S-codes

Provider failures map onto the same code space the rest of the sandbox worker family uses:

| Code | Cause |
|---|---|
| `S100` | Image not in `image_allowlist` |
| `S400` | Concurrency cap reached |
| `S404` | Capability not supported |
| `S500` | Provider returned 429 |
| `S501` | Provider returned 402 / quota exhausted |
| `S502` | Provider returned 5xx |
| `S503` | Provider returned 401 / 403 (auth) |

## Status

v0.1 ships function registrations, types, error mapping, concurrency cap, and a smoke test. The HTTP call bodies that talk to Vercel are stubbed and return `S502` until the next iteration wires them to the real REST endpoints. The ABI is stable.
