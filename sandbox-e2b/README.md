# sandbox-e2b

Narrow iii worker that wraps [E2B](https://e2b.dev) microVM sandboxes via E2B's REST API. Registers the canonical `sandbox::*` ABI under the `sandbox::e2b::*` namespace so callers can spawn and drive E2B sandboxes through `iii.trigger(...)` without depending on E2B's TypeScript or Python SDK.

The same ABI is implemented by every sandbox provider worker in this repo (`sandbox-daytona`, `sandbox-morph`, `sandbox-vercel`, `sandbox-modal`, `sandbox-cf`, ...). Callers swap providers by changing the function-id prefix; capability negotiation tells callers which optional functions a given provider supports.

## Functions

| Function id | Purpose |
|---|---|
| `sandbox::e2b::create` | Boot a sandbox and return `{sandbox_id, image, capabilities}` |
| `sandbox::e2b::exec` | Run a command inside a live sandbox |
| `sandbox::e2b::stop` | Tear down a sandbox |
| `sandbox::e2b::list` | Enumerate live sandboxes plus concurrency status |
| `sandbox::e2b::snapshot` | Pause a sandbox into a resumable snapshot |
| `sandbox::e2b::expose_port` | Return a public URL for a port inside the sandbox |
| `sandbox::e2b::fs::read` | Read a file out of the sandbox |
| `sandbox::e2b::fs::write` | Write a file into the sandbox |

`create` advertises capabilities `["snapshot", "expose_port", "fs"]`. `branch` is not registered — callers that depend on branching should prefer `sandbox-morph`.

## Configuration

`config.yaml` next to the binary, or pass `--config <path>`:

```yaml
api_base: "https://api.e2b.dev"
api_key_env: E2B_API_KEY
max_concurrent_sandboxes: 10
default_idle_timeout_secs: 300
image_allowlist: []      # empty = allow all
```

`E2B_API_KEY` must be present in the environment when the worker starts. The worker fails fast if it cannot read the variable named by `api_key_env`.

## S-codes

Provider failures map onto a stable code space shared with the rest of the sandbox worker family:

| Code | Cause |
|---|---|
| `S100` | Image not in `image_allowlist` |
| `S400` | Concurrency cap reached |
| `S404` | Capability not supported (e.g. caller invoked `branch`) |
| `S500` | Provider returned 429 (rate-limited) |
| `S501` | Provider returned 402 / quota exhausted |
| `S502` | Provider returned 5xx |
| `S503` | Provider returned 401 / 403 (auth invalid or expired) |

## Status

v0.1 ships the function registrations, types, error mapping, concurrency cap, and a smoke test. The HTTP call bodies that talk to E2B are stubbed and return `S502` until the next iteration wires them to the real REST endpoints. The ABI is stable.
