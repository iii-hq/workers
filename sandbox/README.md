# sandbox

Caller-facing `sandbox::*` router. Mirrors the `provider-router` shape:
the router owns the bare `sandbox::*` namespace and dispatches by the
`provider` field to `sandbox::provider::<name>::*`.

| Caller id | Forwards to |
|---|---|
| `sandbox::create` | `sandbox::provider::<name>::create` |
| `sandbox::exec` | `sandbox::provider::<name>::exec` |
| `sandbox::stop` | `sandbox::provider::<name>::stop` |
| `sandbox::list` | `sandbox::provider::<name>::list` |
| `sandbox::snapshot` | `sandbox::provider::<name>::snapshot` |
| `sandbox::expose_port` | `sandbox::provider::<name>::expose_port` |
| `sandbox::branch` | `sandbox::provider::<name>::branch` |
| `sandbox::fs::read` | `sandbox::provider::<name>::fs::read` |
| `sandbox::fs::write` | `sandbox::provider::<name>::fs::write` |

`<name>` resolves from `payload.provider`; absent or empty → `default_provider`
from `config.yaml` (default `local`).

## Install

```bash
iii worker add sandbox
```

Then add at least one adapter:

```bash
iii worker add sandbox-e2b        # microVM
iii worker add sandbox-daytona    # snapshot-based
iii worker add sandbox-vercel     # source-deploy
iii worker add sandbox-morph      # live-VM with branch
iii worker add sandbox-modal      # Python sandbox
iii worker add sandbox-cloudflare # CF Worker bridge
```

## Quickstart

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let created = iii
        .trigger(TriggerRequest {
            function_id: "sandbox::create".into(),
            payload: json!({ "provider": "e2b", "image": "base" }),
            action: None,
            timeout_ms: Some(60_000),
        })
        .await?;

    println!("{created:#?}");
    Ok(())
}
```

Omit `provider` to use `default_provider`. Set `default_provider` per
deployment in `config.yaml`:

```yaml
default_provider: e2b
```

## Error codes

Forwarded errors carry the adapter's `[Sxxx]` prefix. The router itself
emits:

| Code | When |
|---|---|
| `S502` | Forward to `sandbox::provider::<name>::<leaf>` failed at the bus |
| `S600` | No adapter registered for `<name>` (run `iii worker add sandbox-<name>`) |

See [`sandbox-CONTEXT.md`](../sandbox-CONTEXT.md) for the full ABI.
