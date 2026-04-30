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
| `--engine-url` (`-e`) | iii engine WebSocket URL. Default `ws://localhost:49134`. |
| `--debug` (`-d`) | Verbose tracing on stderr. |
| `--brain-fn` (`IIIACP_BRAIN_FN`) | iii function id that processes `session/prompt`. Receives `{ sessionId, connId, prompt, respondTopic }` and returns `{ stopReason }`. Falls back to a built-in echo brain. |
| `--publish-updates` (`IIIACP_PUBLISH_UPDATES`) | Also publish `session/update` notifications to the durable topic `acp:<connId>:session:<sessId>:updates` for external observers. Stdout delivery is always on; this is opt-in fan-out. |
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

```bash
iii-acp & ; pid=$!
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"demo","version":"0"}}}' >&2
```

Streamed reply on stdout (one frame per line):

```json
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{...},"agentInfo":{"name":"iii-acp","version":"0.1.0"}}}
```

## Plugging a real brain

Register a worker that exposes a function with this contract:

```jsonc
// payload
{
  "sessionId": "sess_...",
  "connId": "<connId>",
  "prompt": [ { "type": "text", "text": "..." }, ... ],
  "respondTopic": "acp:<connId>:session:<sessId>:updates"
}
// response
{ "stopReason": "end_turn" | "max_tokens" | "refusal" | "cancelled" }
```

Stream `session/update` notifications by publishing to the `respondTopic` via
`iii::durable::publish`. The acp worker forwards anything published to that
topic out to its stdout when `--publish-updates` is set, or skips it when
your brain runs in the same process and writes notifications inline via the
trigger response.

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
