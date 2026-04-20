# iii-agent Worker Specification

## Overview

The iii-agent is the chat orchestrator for the iii console. It dynamically discovers functions from connected workers and orchestrates them via LLM (Anthropic Claude) to answer user questions.

## Functions

| Function ID | Description |
|---|---|
| `agent::chat` | Send a message and get a structured JSON-UI response |
| `agent::chat_stream` | Send a message with streaming response via iii Streams |
| `agent::discover` | List all available functions the agent can orchestrate |
| `agent::plan` | Generate an execution plan DAG without executing |
| `agent::session_create` | Create a new chat session |
| `agent::session_history` | Retrieve conversation history for a session |
| `agent::session_cleanup` | Clean up expired sessions (cron-triggered) |

## State Scopes

| Scope | Key | Value |
|---|---|---|
| `agent:sessions` | `{session_id}` | Conversation history (messages array) |
| `agent:tools` | `cached` | Cached tool definitions from last discovery |

## Triggers

| Type | Config | Function |
|---|---|---|
| `http` POST | `agent/chat` | `agent::chat` |
| `http` POST | `agent/chat/stream` | `agent::chat_stream` |
| `http` GET | `agent/discover` | `agent::discover` |
| `http` POST | `agent/plan` | `agent::plan` |
| `http` POST | `agent/session` | `agent::session_create` |
| `http` POST | `agent/session/history` | `agent::session_history` |
| `cron` | `0 0 * * * *` | `agent::session_cleanup` |
| `engine::functions-available` | `{}` | Tool cache refresh |

## Chat Flow

1. User sends message via `agent::chat` or `agent::chat_stream`
2. Agent discovers available tools via `iii.list_functions()`
3. Builds system prompt with capabilities summary
4. Loads conversation history from state
5. Sends message + tools to Anthropic Claude API
6. If LLM returns text -> done, return JSON-UI response
7. If LLM returns tool_use -> call `iii.trigger(function_id, payload)`
8. Feed tool result back to LLM as tool_result message
9. Repeat (max 10 iterations)
10. Save conversation to state

## JSON-UI Response Format

```json
{
  "elements": [
    {"type": "text", "content": "..."},
    {"type": "chart", "chart_type": "bar", "data": [...]},
    {"type": "table", "headers": [...], "rows": [...]},
    {"type": "diagram", "format": "mermaid", "content": "..."},
    {"type": "action", "label": "...", "function_id": "...", "payload": {...}}
  ],
  "usage": {
    "input_tokens": 1234,
    "output_tokens": 567
  }
}
```

## Streaming Events

Stream group: `agent:events:{session_id}`

| Event Type | Fields |
|---|---|
| `text_delta` | `text` |
| `tool_use` | `name`, `input` |
| `tool_result` | `name`, `result` |
| `error` | `message` |
| `done` | (empty) |

## Configuration

```yaml
anthropic_model: "claude-sonnet-4-20250514"
max_tokens: 4096
max_iterations: 10
session_ttl_hours: 24
cron_session_cleanup: "0 0 * * * *"
```

## Environment Variables

- `ANTHROPIC_API_KEY` - Required. Anthropic API key for Claude access.
- `RUST_LOG` - Optional. Log level filter (default: `info`).

## Discovery Filter

The agent excludes these function prefixes from LLM tool building:
- `agent::*` - prevents self-invocation loops
- `state::*` - internal state operations
- `stream::*` - internal stream operations
- `engine::*` - internal engine operations

## Running

```bash
ANTHROPIC_API_KEY=sk-ant-... cargo run --release -- --url ws://127.0.0.1:49134
```
