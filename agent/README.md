# iii-agent

Linear, PostHog, Attio — they all shipped the same thing: a chat bar as the primary interface. iii-agent brings this to the iii console. It dynamically discovers every function registered by every connected worker, lets users ask questions in natural language, and the LLM decides which functions to call. "What's slow in my system?" triggers `eval::analyze_traces`. "Show me the topology" triggers `introspect::diagram`. The agent composes the answer from real data, not hallucinations.

**Plug and play:** Build with `cargo build --release`, set `ANTHROPIC_API_KEY` in your environment, then run `./target/release/iii-agent --url ws://your-engine:49134`. It registers 7 functions, discovers all available tools from other workers, and starts accepting chat via `agent::chat`. Connect more workers and they're automatically available — no restart needed.

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

## iii Primitives Used

- **State** -- session history, cached tool definitions
- **Streams** -- streaming chat responses via `agent:events:{session_id}` group
- **Cron** -- hourly session cleanup
- **HTTP** -- chat, discovery, planning, and session management endpoints

## Prerequisites

- Rust 1.75+
- Running iii engine on `ws://127.0.0.1:49134`
- `ANTHROPIC_API_KEY` environment variable set

## Build

```bash
cargo build --release
```

## Usage

```bash
# Load the key from your secret manager (keychain, 1password, doppler, etc.)
# into the environment before launching the worker — never paste the literal
# key on the command line, since it lands in shell history and `ps` output.
export ANTHROPIC_API_KEY="$(security find-generic-password -s anthropic-api-key -w)"
./target/release/iii-agent --url ws://127.0.0.1:49134 --config ./config.yaml
```

```
Options:
  --config <PATH>    Path to config.yaml [default: ./config.yaml]
  --url <URL>        WebSocket URL of the iii engine [default: ws://127.0.0.1:49134]
  --manifest         Output module manifest as JSON and exit
  -h, --help         Print help
```

## Configuration

```yaml
anthropic_model: "claude-sonnet-4-20250514"  # model to use for chat
max_tokens: 4096                              # max tokens per LLM response
max_iterations: 10                            # max tool-use loops per message
session_ttl_hours: 24                         # session expiry
cron_session_cleanup: "0 0 * * * *"           # hourly cleanup schedule
```

## Tests

```bash
cargo test
```
