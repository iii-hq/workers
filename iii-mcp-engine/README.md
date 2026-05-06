# iii-mcp-engine

MCP-shaped proxy worker. Exposes the iii engine's read allowlist as MCP tools so
external agents (Cursor, Claude Code) can consume the same surface the
iii-console chat panel uses.

Each entry in `ENGINE_TOOLS` is registered twice:

1. As an iii bus function `iii-mcp-engine::<engine_fn_id>` that proxies to the
   underlying engine read and applies a per-tool `max_bytes` contract via
   `truncate_result` so a runaway response can't blow the LLM context.
2. As a tool descriptor on `skills::register` so `/skills` and the chat panel
   discover it.

## Install

```bash
iii worker add iii-mcp-engine
```

## Run

```bash
iii-mcp-engine --engine-url ws://127.0.0.1:49134
```

`III_URL` overrides the default engine URL.

## Allowlist

Read-only engine functions only. Update `src/tools.rs` and the matching console
list together when the engine grows new read functions:

- `engine::traces::list`, `engine::traces::tree`
- `engine::logs::list`
- `engine::workers::list`
- `engine::functions::list`, `engine::triggers::list`
- `engine::queue::dlq_messages`
