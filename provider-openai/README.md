# provider-openai

This worker registers **provider::openai::complete** on the iii bus: one-shot Chat
Completions that resolve credentials via `auth::get_token`, stream internally,
and return a final `AssistantMessage`. Agents and harness code call it with a
model id and message list; keys and OAuth tokens stay in **auth-credentials**.

## Install

```bash
iii worker add provider-openai
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next `iii start`.

You need **auth-credentials** (or equivalent) so `auth::get_token` can resolve
the `openai` provider:

```bash
iii worker add auth-credentials
```

## Quickstart

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://127.0.0.1:49134", InitOptions::default());

    let result = iii.trigger(TriggerRequest {
        function_id: "provider::openai::complete".into(),
        payload: json!({
            "model": "gpt-4o-mini",
            "system_prompt": "You are terse.",
            "messages": [{ "role": "user", "content": "Say hi in one word." }],
            "tools": [],
        }),
        action: None,
        timeout_ms: Some(60_000),
    }).await?;

    println!("{result:#?}");
    Ok(())
}
```

## Configuration

```yaml
default_max_tokens: 4096                                              # Completions cap unless overridden per request path
default_api_url: "https://api.openai.com/v1/chat/completions"        # OpenAI-compatible endpoint base
```

Other defaults live in [`src/config.rs`](src/config.rs).
