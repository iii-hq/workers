# Authoring a worker Quickstart

The `## Quickstart` section is the meat of a worker README. It moves the reader from `the worker is running` to `I made a first useful call` in a single fenced code block.

## Constraints

- Aim for thirty lines of code or fewer in the primary example.
- Pick one audience-appropriate language (Rust, Node, or Python) — not a polyglot wall.
- Show one to three functions, chosen for introductory value, not breadth. A worker with twenty functions still shows two.
- Each shown call demonstrates three things:
  - The function id, e.g., `textstats::analyze`.
  - A realistic payload — the kind a caller would actually send, not `{}` or `{ "key": "value" }`.
  - The expected output shape, in a comment, a `println!`, or a follow-up paragraph.

## Skeleton

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let result = iii
        .trigger(TriggerRequest {
            function_id: "<worker>::<verb>".into(),
            payload: json!({ /* realistic input here */ }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;

    println!("{result:#?}");
    Ok(())
}
```

## Cross-worker handshakes

When a worker is most useful as part of a multi-worker flow (skills + mcp, fts + agentmemory, llm-budget + provider-anthropic, …), the Quickstart can be a walkthrough of the handshake instead of a single function call. Tell it as one end-to-end story: each worker registers its piece, then a final trigger exercises the composition.

A longer Quickstart earns its lines this way — but the thirty-line constraint still applies per code block. Split into two or three blocks separated by one-paragraph framing if needed.

## What does not go in Quickstart

- Every function the worker registers. The seventeen you do not show live in source under `RegisterFunction::new("…")` and are auto-discoverable from the API surface generator.
- Tips on installing other workers — that is `## Install` plus `docs/companions.md`.
- Configuration tuning — that is `## Configuration`.
- Migration from a prior version — that is `## Migration notes`.

The Quickstart is the introductory call. Anything else has its own section.
