# context-manager

Turns a raw conversation history plus a target model into a **model-ready
context**: a system prompt and an ordered message list that fits inside the
model's usable token budget. It owns *what the model sees this turn* — token
counting, function-result pruning, and history compaction (summarisation) —
and nothing else. It is stateless with respect to conversation storage: pass
messages in, get results back, persist them yourself (or with
[`session-manager`](../session-manager/)). Summarisation and model-limit
lookups go through `llm-router` when installed; token counting and pruning
work standalone.

## Install

```bash
iii worker add context-manager
```

`iii worker add` fetches the binary and the engine starts the worker on the
next `iii start`. Runtime configuration lives in the `configuration` worker
(enabled by default in the engine): at boot the worker registers its schema and
fetches the authoritative value, and it hot-reloads on change — every field,
with no restart (`summarizer_timeout_ms` is read per call; a `lease_dir` change
rebuilds and swaps the lease store). An optional `--config <file>.yaml` only
seeds the first registration.

## Quickstart

Build a budgeted context before every model call:

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let result = iii.trigger(TriggerRequest {
        function_id: "context::assemble".into(),
        payload: json!({
            "messages": [
                { "role": "user", "content": [{ "type": "text", "text": "hi" }], "timestamp": 1 }
            ],
            // Inline limits keep the call standalone; pass only `id`/`provider`
            // to resolve limits through llm-router instead.
            "model": { "id": "claude-sonnet-4",
                       "limits": { "context_window": 200000, "max_output_tokens": 8000 } },
            "system_prompt": "You are a helpful assistant."
        }),
        action: None,
        timeout_ms: Some(30_000),
    }).await?;

    // -> { system_prompt, messages, token_count, usable, model_resolved,
    //      applied: { pruned, pruned_tokens, compacted, summary?, tail_start_index? } }
    println!("{result:#?}");
    Ok(())
}
```

The compaction round trip: when `applied.compacted` is true, **persist
`applied.summary`** and the boundary your storage maps `applied.tail_start_index`
to. On later calls pass only the post-compaction window as `messages` plus the
stored summary as `options.previous_summary` — the summary is rendered into the
system prompt under `# Conversation summary`, and any further compaction
*updates* it instead of starting over. Callers that skip persistence stay
correct at the cost of one summariser call per over-budget request.

The other three functions: `context::count-tokens` (estimate messages + tools +
system prompt vs a model), `context::prune` (replace verbose function outputs
with `[output pruned: was ~N tokens]` placeholders, no LLM involved), and
`context::compact` (summarise the head, keep a recent tail verbatim — returns
`ok | busy | empty | overflow`).

## Configuration

```yaml
reserved_tokens_cap: 20000     # default reserve = min(cap, pct% of context window)
reserved_pct: 10
tail_turns: 2                  # user+assistant pairs kept verbatim by compaction
protect_recent_tokens: 40000   # newest function-output tokens never pruned
min_free_tokens: 20000         # skip pruning when it would free less
max_output_chars: 2000         # outputs at or under this size are never pruned
lease_ttl_secs: 300            # compaction mutual-exclusion lease TTL
allow_fallback_limits: true    # conservative 8192/1024 when limits can't resolve
summarizer_timeout_ms: 320000  # outer budget for one router::chat summariser call
```

Other details (and the defaults' single source of truth) live in
[`src/config.rs`](src/config.rs).

## Local development & testing

```bash
cargo test                          # unit + BDD (engine scenarios soft-skip)
cargo test --test bdd -- --tags @pure    # no engine required
UPDATE_GOLDENS=1 cargo test --test schemas   # regenerate wire-schema goldens
```
