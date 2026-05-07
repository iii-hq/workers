# session-tree

Session storage as a parent-id tree of typed entries on the iii bus: agent
messages, custom payloads, branch summaries, and compaction markers. Each session
is a DAG keyed by parent id so forks share history; callers use
`session-tree::*` functions to create sessions, append messages, fork branches,
and export HTML views.

## Install

```bash
iii worker add session-tree
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next `iii start`.

## Quickstart

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let created = iii
        .trigger(TriggerRequest {
            function_id: "session-tree::create".into(),
            payload: json!({ "display_name": "demo" }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;

    let session_id = created["session_id"].as_str().unwrap();

    let appended = iii
        .trigger(TriggerRequest {
            function_id: "session-tree::append".into(),
            payload: json!({
                "session_id": session_id,
                "message": {
                    "role": "user",
                    "content": [{ "type": "text", "text": "hello" }],
                    "timestamp": 0,
                },
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;

    println!("entry_id = {:?}", appended["entry_id"]);
    Ok(())
}
```

Other surfaced calls include `session-tree::messages`, `session-tree::tree`,
`session-tree::fork`, `session-tree::clone`, `session-tree::compact`, and
`session-tree::export_html`.

## Configuration

```yaml
store_backend: iii_state   # iii_state (persist via iii-state) | memory (ephemeral)
```

Defaults live in [`config.yaml`](config.yaml); missing keys use the values in
[`src/config.rs`](src/config.rs).

### Storage layout (iii_state)

Scopes:

- `session_tree:<session_id>` — entry id → `SessionEntry`
- `session_tree_meta` — session id → `SessionMeta`

With `memory`, everything stays in-process and is lost when the worker exits.
