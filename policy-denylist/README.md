# policy-denylist

Subscribes to `agent::before_function_call` and blocks any call whose
`function_call.function_id` is on a configured denylist (exact string match, case-sensitive).
The engine and other workers publish that hook topic so you get a second line of
defense after client-side allowlists.

## Install

```bash
iii worker add policy-denylist
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next `iii start`.

## Quickstart

This worker does not expose a separate HTTP tool surface: it registers
`policy::denylist` and binds a `subscribe` trigger to the configured topic. From
another process on the bus you only need the engine running and this worker
started; hook traffic is driven by `provider-router` (or any publisher of
`agent::before_function_call`).

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let result = iii
        .trigger(TriggerRequest {
            function_id: "policy::denylist".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;

    println!("{result:#?}");
    Ok(())
}
```

In practice the function is invoked by the bus when a `before_function_call` event
arrives; the snippet above is only useful to verify registration in a dev setup.

## Configuration

```yaml
topic: agent::before_function_call   # hook topic to subscribe to
denied_functions:                     # iii function ids to block (exact match)
  - "bash:rm -rf"
  - sudo
  - curl-pipe-bash
```

The key `denied_tools` is accepted as an alias for one release (serde).

If the engine wraps settings under a `config:` key, that nested block is
accepted as well. Other keys (and their defaults) live in
[`src/config.rs`](src/config.rs).

`POLICY_DENYLIST_TOPIC` and `POLICY_DENIED_FUNCTIONS` (comma-separated list; legacy
`POLICY_DENIED_TOOLS` still read) override the file when set.

## Workspace allowlist composition

Chat clients (e.g. `iii-console`) layer a workspace allowlist on top of
`policy-denylist`. The allowlist enforces:

- Filesystem writes restricted to a configured workspace root.
- Absolute paths outside the workspace are rejected by the SDK wrapper *before*
  the bus call is dispatched.

`policy-denylist` remains the second layer (deny by function id regardless of
arguments). The two layers compose:

1. SDK wrapper (chat client side) — workspace allowlist on path arguments.
2. `policy-denylist` (engine side) — deny by function id (exact match, case-sensitive).
3. `<ApprovalRow>` (chat UI) — per-call user approval surfaced inline before any write reaches disk.

## Registered functions

| Function | Description |
|---|---|
| `policy::denylist` | Subscriber bound to the configured topic. Replies `{ block: true, reason }` for matches; `{ block: false }` otherwise. |

## Runtime expectations

By default, the worker subscribes to `agent::before_function_call`. That topic is
published by `provider-router` while the agent loop is executing — for the
denylist to fire, `provider-router` (or any worker emitting the same topic) must
be running on the bus.
