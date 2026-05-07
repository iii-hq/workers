# shell-filesystem

Sandboxed filesystem tools on the iii bus. Each tool is a function id under `shell::filesystem::*` (for example `shell::filesystem::read` and `shell::filesystem::write`) so agents can read, write, list, and mutate files on the host within your policy boundaries.

## Install

```bash
iii worker add shell-filesystem
```

`iii worker add` fetches the binary, writes a config block into `~/.iii/config.yaml`, and the engine starts the worker on the next `iii start`.

## Quickstart

Call one of the registered function ids with the same shape the worker expects:

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let result = iii
        .trigger(TriggerRequest {
            function_id: "shell::filesystem::read".into(),
            payload: json!({ "path": "/tmp/example.txt" }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;

    println!("{result:#?}");
    Ok(())
}
```

Other verbs include `shell::filesystem::write`, `shell::filesystem::ls`, `shell::filesystem::stat`, `shell::filesystem::mkdir`, `shell::filesystem::rm`, `shell::filesystem::mv`, `shell::filesystem::chmod`, `shell::filesystem::grep`, `shell::filesystem::sed`, and `shell::filesystem::edit`.

## Configuration

```yaml
max_inline_bytes: 262144   # max bytes returned inline for shell::filesystem::read
```

The worker loads this file via `--config` (default `./config.yaml` when you run the binary locally). On config errors it falls back to these defaults.

`SHELL_FILESYSTEM_MAX_INLINE_BYTES` still overrides `max_inline_bytes` at process start when set.

Other keys (if added later) and their defaults live in [`src/config.rs`](src/config.rs).
