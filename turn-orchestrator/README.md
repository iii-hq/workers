# turn-orchestrator

Durable `run::start` state machine on the iii bus. Drives each agent
turn through provisioning → assistant → functions → steering → tearing-down,
checkpointing the session record on every step so a process crash or
restart resumes from the last persisted node rather than restarting the
run from scratch. Most users install this worker via the
[`harness`](../harness) meta-worker, which pulls it in as a dependency.

## Install

```bash
iii worker add turn-orchestrator
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next
`iii start`. The `dependencies:` field in `iii.worker.yaml` ensures
`session-inbox`, `hook-fanout`, and `provider-router` are installed
alongside this worker automatically.

## Quickstart

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    // Fire-and-forget: returns immediately with the session id.
    let started = iii.trigger(TriggerRequest {
        function_id: "run::start".into(),
        payload: json!({
            "session_id": "my-session-01",
            "provider": "anthropic",
            "model": "claude-sonnet-4-5",
            "system_prompt": "You are a helpful assistant.",
            "messages": [
                { "role": "user", "content": [{ "type": "text", "text": "What is 2 + 2?" }] }
            ],
            "max_turns": 5,
        }),
        action: None,
        timeout_ms: Some(5_000),
    }).await?;

    println!("session started: {}", started["session_id"]);

    // For testing: start a run and block until it reaches a terminal state.
    let result = iii.trigger(TriggerRequest {
        function_id: "run::start_and_wait".into(),
        payload: json!({
            "session_id": "my-session-02",
            "provider": "anthropic",
            "model": "claude-sonnet-4-5",
            "system_prompt": "You are a helpful assistant.",
            "messages": [
                { "role": "user", "content": [{ "type": "text", "text": "What is 2 + 2?" }] }
            ],
            "max_turns": 3,
            "timeout_ms": 60000,
        }),
        action: None,
        timeout_ms: Some(65_000),
    }).await?;

    // { "session_id": "...", "messages": [...], "turn_count": 1 }
    println!("{result:#?}");
    Ok(())
}
```

### `run::start` payload

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

`run::start` returns `{ "session_id": "..." }`. `run::start_and_wait`
additionally accepts `timeout_ms` (default: `120000`) and returns
`{ "session_id", "messages", "turn_count" }` once the run terminates.

## Configuration

```yaml
sync_default_timeout_ms: 120000   # how long run::start_and_wait polls before timing out
sync_poll_interval_ms: 50          # how frequently run::start_and_wait checks for terminal state
```

CLI flags:

```text
--config <PATH>    Path to config.yaml [default: ./config.yaml] [env: TURN_ORCHESTRATOR_CONFIG]
--url <URL>        WebSocket URL of the iii engine [default: ws://127.0.0.1:49134] [env: III_URL]
--manifest         Output the module manifest as JSON and exit
-h, --help         Print help
```

If the config file is missing or malformed the worker logs a warning and
falls back to defaults — boot is never blocked by a bad config path.

## Output stream: `agent::events`

Every state transition emits an `AgentEvent` frame to the
`agent::events` stream via `stream::set`. Item ids use the format
`<session_id>-<seq:08>` (zero-padded 8-digit sequence number). Consumers
subscribe via:

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
