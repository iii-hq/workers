# turn-orchestrator — architecture and operator notes

The published `README.md` and `skill.md` for this worker are rendered from `docs/`. This file holds the operator/contributor material that does not belong in the published surfaces — full payload reference, CLI flags, and the `agent::events` stream contract.

## CLI flags

```text
--config <PATH>    Path to config.yaml [default: ./config.yaml] [env: TURN_ORCHESTRATOR_CONFIG]
--url <URL>        WebSocket URL of the iii engine [default: ws://127.0.0.1:49134] [env: III_URL]
--manifest         Output the module manifest as JSON and exit
-h, --help         Print help
```

If the config file is missing or malformed the worker logs a warning and falls back to defaults — boot is never blocked by a bad config path.

## `run::start` payload reference

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `session_id` | string | yes | — | Stable id for this run; re-using it resumes from persisted state. |
| `provider` | string | yes | — | Provider name, e.g. `anthropic`, `openai`. |
| `model` | string | yes | — | Model id passed to the provider. |
| `system_prompt` | string | no | `""` | System prompt. |
| `messages` | array | no | `[]` | Initial conversation history (`AgentMessage` array). |
| `max_turns` | integer | no | unlimited | Stop after this many assistant turns. |
| `approval_required` | string[] | no | `[]` | Tool names requiring human approval before execution. |
| `image` | string | no | `"python"` | Sandbox image for `shell::*` tools. |
| `idle_timeout_secs` | integer | no | `300` | Sandbox idle timeout. |
| `cwd` | string | no | null | Working-directory path for the sandbox. |
| `cwd_hash` | string | no | null | SHA hash of `cwd`; used to resume sessions sharing the same environment. |

`run::start` returns `{ "session_id": "..." }`. `run::start_and_wait` additionally accepts `timeout_ms` (default `120000`) and returns `{ "session_id", "messages", "turn_count" }` once the run terminates.

## Output stream: `agent::events`

Every state transition emits an `AgentEvent` frame to the `agent::events` stream via `stream::set`. Item ids use the format `<session_id>-<seq:08>` (zero-padded 8-digit sequence number). Consumers subscribe via:

```rust
iii.register_trigger(RegisterTriggerInput {
    trigger_type: "stream:join".into(),
    function_id: "myworker::on_agent_event".into(),
    config: json!({ "stream_name": "agent::events", "group_id": session_id }),
    metadata: None,
})?;
```

| Event type | Emitted when |
|---|---|
| `agent_start` | First step of a new session |
| `turn_start` | Each new assistant turn begins |
| `message_start` / `message_end` | Each message in the conversation |
| `turn_end` | Assistant turn completes (with tool results) |
| `agent_end` | Session reaches `stopped` state |
