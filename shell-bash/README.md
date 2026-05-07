# shell-bash

Sandboxed shell helpers on the iii bus under `shell::bash::*` (`exec`, `which`, `detect_clis`).
They complement the engine `sandbox::exec` workflow; there is no silent host-shell fallback for callers that expected pure sandbox routing.

## Install

```bash
iii worker add shell-bash
```

`iii worker add` fetches the binary, writes a config block into `~/.iii/config.yaml`, and the engine starts the worker on the next `iii start`.

## Quickstart

Call `shell::bash::exec` with a bash snippet; timeouts and output truncation follow `config.yaml` unless overridden per payload or via environment variables below.

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let result = iii
        .trigger(TriggerRequest {
            function_id: "shell::bash::exec".into(),
            payload: json!({ "command": "echo hello" }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;

    println!("{result:#?}");
    Ok(())
}
```

Other entry points:

- `shell::bash::which` — `{ "name": "git" }`
- `shell::bash::detect_clis` — `{}`

## Configuration

```yaml
default_timeout_ms: 30000    # milliseconds when payload omits timeout_ms
trigger_timeout_ms: 35000    # milliseconds reserved for cron/http-style bindings
max_output_bytes: 30000       # truncate combined stdout/stderr text in the tool payload
```

`SHELL_BASH_DEFAULT_TIMEOUT_MS`, `SHELL_BASH_TRIGGER_TIMEOUT_MS`, and `SHELL_BASH_MAX_OUTPUT_BYTES` override file defaults at process start.

Other defaults (including manifest `default_config`) live in [`src/config.rs`](src/config.rs) and [`src/manifest.rs`](src/manifest.rs).

## Registered functions

- `shell::bash::exec`
- `shell::bash::which`
- `shell::bash::detect_clis`

## Engine surface

Consumers rely on host `bash` for these helpers. For the sandbox primitive itself, see the engine `sandbox::exec` tool surface (distinct from legacy `iii-exec` pipeline naming in some docs).
