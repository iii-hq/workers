# provider-anthropic

Native Anthropic Messages API streaming provider on the iii bus. Exposes
`provider::anthropic::complete` (same wire shape as other streaming providers).

Companion [`auth-credentials`](../auth-credentials) resolves API keys and OAuth tokens via
`auth::get_token`.

## Install

```bash
iii worker add provider-anthropic
```

```bash
iii worker add auth-credentials
```

## Quickstart

From a client connected to the same iii engine:

```rust
use iii_sdk::{III, TriggerRequest};
use serde_json::json;

async fn probe(iii: &III) {
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "provider::anthropic::complete".into(),
            payload: json!({
                "config": { "model": "claude-sonnet-4-6" },
                "system_prompt": "You are concise.",
                "messages": [],
                "tools": [],
            }),
            action: None,
            timeout_ms: Some(120_000),
        })
        .await;
}
```

Responses are `{ "events": [ … AssistantMessageEvent … ] }` with a terminal
`done` or `error` event.

## Configuration

Committed defaults live in [`config.yaml`](config.yaml). Override at runtime with:

- `--config <path>` (default `./config.yaml`)
- `--url <ws-url>` (default `ws://127.0.0.1:49134`)

| Field | Meaning |
| --- | --- |
| `default_max_tokens` | `max_tokens` on the Messages request when not overridden per call |
| `default_api_url` | Anthropic Messages endpoint (default `https://api.anthropic.com/v1/messages`) |

Registry / CI: `iii-provider-anthropic --manifest` prints JSON for publish.

## Worker dependencies

| Worker | Range | Reason |
| --- | --- | --- |
| `auth-credentials` | `^0.1.0` | Resolves Anthropic API key or OAuth token via `auth::get_token`. |

## Registered functions

`provider::anthropic::complete` plus the deprecated alias
`provider::anthropic::stream_assistant` for one release.
