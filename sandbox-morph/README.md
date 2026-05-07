# sandbox-morph

Narrow iii worker that wraps [Morph Cloud](https://cloud.morph.so) microVM sandboxes via Morph's REST API. Morph's wedge is **Infinibranch** — branching a running sandbox into N siblings with live process state preserved in roughly 250 ms. This worker is the only sandbox provider in the family that registers `branch`.

Registers the canonical `sandbox::*` ABI under the `sandbox::morph::*` namespace so callers can spawn and drive Morph sandboxes through `iii.trigger(...)` without depending on Morph's SDK.

## Functions

| Function id | Purpose |
|---|---|
| `sandbox::morph::create` | Boot a sandbox; returns `{sandbox_id, image, capabilities}` |
| `sandbox::morph::exec` | Run a command inside a live sandbox |
| `sandbox::morph::stop` | Tear down a sandbox |
| `sandbox::morph::list` | Enumerate live sandboxes plus concurrency status |
| `sandbox::morph::snapshot` | Snapshot a sandbox (chainable with `setup` upstream) |
| `sandbox::morph::branch` | Branch a running sandbox into N siblings (Infinibranch) |
| `sandbox::morph::expose_port` | Return a public URL for a port inside the sandbox |

`create` advertises capabilities `["branch", "snapshot", "expose_port"]`. `fs::read` and `fs::write` are not registered for v0 — Morph's filesystem ops are reachable via SSH-shape APIs that don't map cleanly to the channel-based FS surface used by other sandbox workers; revisit when consensus emerges.

## Configuration

`config.yaml` next to the binary, or pass `--config <path>`:

```yaml
api_base: "https://cloud.morph.so/api"
api_key_env: MORPH_API_KEY
max_concurrent_sandboxes: 10
default_idle_timeout_secs: 300
image_allowlist: []
```

`MORPH_API_KEY` must be present in the environment when the worker starts. Header sent on every upstream call: `Authorization: Bearer <MORPH_API_KEY>`.

## S-codes

Provider failures map onto a stable code space shared with the rest of the sandbox worker family:

| Code | Cause |
|---|---|
| `S100` | Image not in `image_allowlist` |
| `S400` | Concurrency cap reached |
| `S404` | Capability not supported |
| `S500` | Provider returned 429 (rate-limited) |
| `S501` | Provider returned 402 / quota exhausted |
| `S502` | Provider returned 5xx |
| `S503` | Provider returned 401 / 403 |

## Status

v0.1 ships the function registrations (including `branch`), types, error mapping, concurrency cap, and a smoke test. The HTTP call bodies that talk to Morph are stubbed and return `S502` until the next iteration wires them to the real REST endpoints. The ABI is stable.

When the real client lands, a follow-up benchmark must verify that `sandbox::morph::branch` round-trips inside ~300 ms p99 against raw Morph; the wedge dies if iii's trigger envelope adds noticeable overhead.
