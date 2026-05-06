# acp

Agent Client Protocol surface for the iii engine. Exposes iii agents to any
ACP-speaking client (editors, harnesses) over stdio JSON-RPC. Mirrors the role
that `iii-mcp` plays for tools and `iii-a2a` plays for peer agents.

> Status: 0.1.0 — server-side only. `acp-client` (consume external ACP
> agents) ships separately. Reverse-RPC paths (`session/request_permission`,
> `fs/*`, `terminal/*`) are stubbed for v0; agents use iii primitives directly
> for filesystem and terminal.

## Install

```bash
iii worker add acp
```

## Spawn

`iii-acp` is a stdio agent. The client (editor or harness) launches it as a
subprocess and exchanges JSON-RPC frames over stdin/stdout.

```bash
iii-acp --engine-url ws://localhost:49134
```

stderr is reserved for logs. stdout is reserved for ACP frames.

## Configuration

| Flag / env | Effect |
|---|---|
| `--engine-url` (`-e`, `IIIACP_ENGINE_URL`) | iii engine WebSocket URL. Default `ws://localhost:49134`. |
| `--debug` (`-d`) | Verbose tracing on stderr. |
| `--brain-fn` (`IIIACP_BRAIN_FN`) | iii function id that runs the prompt turn. Falls back to a built-in echo brain. Canonical value is `run::start_and_wait` (turn-orchestrator). |
| `--use-canonical-brain` (`IIIACP_USE_CANONICAL_BRAIN`) | Shortcut for `--brain-fn run::start_and_wait`. |
| `--model` (`IIIACP_MODEL`) | Model id forwarded to the brain (e.g. `claude-opus-4-7`). |
| `--provider` (`IIIACP_PROVIDER`) | Provider id forwarded to the brain (e.g. `anthropic`). Routes to `provider::<provider>::complete`. |
| `--system-prompt` (`IIIACP_SYSTEM_PROMPT`) | System prompt prepended to every turn. |
| `--rbac-tag` | Forwards `x-iii-rbac-tag` on the worker WebSocket so `iii-worker-manager`'s `auth_function_id` can apply policy. |

## Methods

| Method | Direction | Status |
|---|---|---|
| `initialize` | client → agent | implemented |
| `authenticate` | client → agent | no-op success |
| `session/new` | client → agent | implemented |
| `session/load` | client → agent | implemented (replays history as `session/update`) |
| `session/list` | client → agent | implemented |
| `session/prompt` | client → agent | implemented; routes to brain fn or echo |
| `session/cancel` | client → agent | implemented; flips in-process abort + publishes cancel topic |
| `session/close` | client → agent | implemented |
| `session/update` | agent → client | streamed during prompt turn |
| `session/request_permission` | agent → client | not in v0 |
| `fs/*`, `terminal/*` | agent → client | not in v0 — agents use iii primitives directly |

## State layout

All keys live in scope `acp`.

```
<connId>:sessions:_index           = ["sess_a", "sess_b", ...]
<connId>:sessions:<sessId>         = { sessionId, connId, cwd, mcpServers, created_at_ms, last_activity_ms }
<connId>:sessions:<sessId>:history = [ session/update entries ... ]
```

`connId` is regenerated per subprocess. State is always namespaced by
connection so concurrent editors don't read each other's sessions.

## Wire example

Pipe a JSON-RPC frame on stdin, read the reply on stdout:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"demo","version":"0"}}}' \
  | iii-acp
```

Streamed reply on stdout (one frame per line):

```json
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{...},"agentInfo":{"name":"iii-acp","version":"0.1.0"}}}
```

## Plugging a real brain

iii-acp talks to the canonical iii brain shape used by `turn-orchestrator`
and every provider worker. Any function with this input contract works
as a drop-in brain — no adapter required.

**Brain function input** (forwarded as `iii.trigger` payload):

```jsonc
{
  "session_id": "sess_...",          // ACP sessionId reused as run id
  "messages": [
    {
      "role": "user",
      "content": [{"type": "text", "text": "..."}, ...],
      "timestamp": 1234567890
    }
  ],
  "model": "claude-opus-4-7",        // when --model is set
  "provider": "anthropic",           // when --provider is set
  "system_prompt": "You are ...",    // when --system-prompt is set
  "timeout_ms": 600000
}
```

**Brain function output:**

```jsonc
{
  "session_id": "sess_...",
  "messages": [...],                 // full transcript including assistant tail
  "turn_count": 1
}
```

iii-acp picks the ACP `stopReason` from the final assistant message's
`stop_reason` field (`end` → `end_turn`, `length` → `max_tokens`,
`aborted` → `cancelled`, `error` → `refusal`).

**Streaming.** While the brain runs, it emits `AgentEvent` frames into the
canonical `agent::events` stream (group_id = session_id). iii-acp registers
**one** stream subscriber per connection at startup and translates each
event:

| `AgentEvent` | ACP `session/update.update.sessionUpdate` |
|---|---|
| `message_update { llm_event: text_delta }` | `agent_message_chunk` |
| `message_update { llm_event: thinking_delta }` | `agent_thought_chunk` |
| `tool_execution_start` | `tool_call` (status: `in_progress`) |
| `tool_execution_end` | `tool_call_update` (status: `completed`/`failed`) |
| other | dropped (no ACP equivalent) |

This is the same stream `context-compaction`, every provider worker, and
any other observer subscribes to. **No bespoke iii-acp publish protocol.**

### Wire to turn-orchestrator

```bash
iii worker add turn-orchestrator provider-router provider-anthropic auth-credentials
iii-acp --use-canonical-brain --model claude-opus-4-7 --provider anthropic
```

Or from Zed `agent_servers`:

```jsonc
{
  "agent_servers": {
    "iii-acp": {
      "type": "custom",
      "command": "/path/to/iii-acp",
      "env": {
        "IIIACP_USE_CANONICAL_BRAIN": "1",
        "IIIACP_MODEL": "claude-opus-4-7",
        "IIIACP_PROVIDER": "anthropic"
      }
    }
  }
}
```

Then point acp at it:

```bash
iii-acp --brain-fn agent::run
```

## Tests

```bash
cargo test
```

Unit + protocol envelope tests. Integration tests against a live engine
live in the iii test harness.
