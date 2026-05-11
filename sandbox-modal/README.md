# sandbox-modal

Narrow iii worker that wraps [Modal](https://modal.com) sandboxes. Modal is gRPC-only — there is no public REST API — so this worker imports the official Modal Python SDK as its transport. The SDK is an implementation detail; callers see only the canonical `sandbox::provider::modal::*` ABI.

The same ABI is implemented by every sandbox provider worker in this repo (`sandbox-e2b`, `sandbox-daytona`, `sandbox-morph`, `sandbox-vercel`, `sandbox-cf`, ...). Callers swap providers by changing the function-id prefix.

## Functions

| Function id | Purpose |
|---|---|
| `sandbox::provider::modal::create` | Boot a sandbox; returns `{sandbox_id, image, capabilities}` |
| `sandbox::provider::modal::exec` | Run a command inside a live sandbox |
| `sandbox::provider::modal::stop` | Tear down a sandbox |
| `sandbox::provider::modal::list` | Enumerate live sandboxes plus concurrency status |
| `sandbox::provider::modal::snapshot` | Snapshot the sandbox filesystem for fan-out |
| `sandbox::provider::modal::expose_port` | Public URL for a port via Modal's Tunnel |

`create` advertises capabilities `["snapshot", "expose_port"]`. `branch` and `fs::*` are not registered for v0 — Modal's filesystem ops use the `Sandbox.open()` file-handle API which doesn't map cleanly to the channel-based FS surface used by the rest of the family. Revisit when consensus emerges.

## Configuration

`config.yaml` next to the binary, or set `SANDBOX_MODAL_CONFIG` to a path:

```yaml
max_concurrent_sandboxes: 10
default_idle_timeout_secs: 300
default_cpus: 1
default_memory_mb: 512
image_allowlist: []      # empty = allow all
```

Modal authenticates via `MODAL_TOKEN_ID` + `MODAL_TOKEN_SECRET` — the official SDK reads them automatically. The worker fails fast if Modal cannot find tokens at startup.

## S-codes

Provider failures map onto the same code space the rest of the sandbox worker family uses:

| Code | Cause |
|---|---|
| `S100` | Image not in `image_allowlist` |
| `S400` | Concurrency cap reached |
| `S404` | Capability not supported |
| `S500` | Modal raised `RateLimitError` |
| `S501` | Modal raised quota error |
| `S502` | Other Modal SDK exception |
| `S503` | Modal raised `AuthError` / `InvalidError` |

## Running

```bash
pip install -e .[dev]
sandbox-modal     # entry point from pyproject.scripts
```

## Status

v0.1 ships function registrations, types, error mapping, async concurrency tracking, and a smoke test. The Modal SDK calls inside `ModalClient` are stubbed and raise `SandboxError(S502)` until the next iteration wires them to `modal.Sandbox.create(...)` / `sandbox.exec(...)` / `sandbox.terminate()`. The ABI is stable.
