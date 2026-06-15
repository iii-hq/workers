# acp

**iii as a first-class agent in any ACP-speaking editor.**

`acp` is a stdio JSON-RPC adapter that exposes the iii engine — and every
brain worker on it — through the [Agent Client Protocol](https://agentclientprotocol.com).
Editors and clients that already speak ACP launch `acp` as a subprocess
and drive it through their native agent UI. No editor plugin, no fork, no
bespoke per-client integration.

> Status: 0.1.0. Server-side only (`acp-client` for consuming external ACP
> agents ships separately). All eleven client→agent methods are implemented
> (`initialize`, `authenticate`, `session/{new,load,resume,list,prompt,cancel,close,set_mode,set_config_option}`)
> plus the `session/update` agent→client notification. Reverse-RPC paths
> (`session/request_permission`, `fs/{read,write}_text_file`,
> `terminal/{create,kill,output,release,wait_for_exit}`) remain deferred —
> internal iii brains use iii primitives directly for filesystem and
> terminal access. Full per-method status in the [Methods](#methods) table.

## Why this exists (vs MCP, skills, agent workers)

These are stack-able layers, not alternatives. ACP fills the slot that the
others don't.

| | What it does | When you reach for it |
|---|---|---|
| **MCP server** (`iii-mcp`) | Exposes iii functions as **tools** to an external agent | You're already running Claude Code / Cursor / etc. and want to give it iii tools |
| **Skill bundles** | Curated prompts + tools loaded into an agent host | You're inside a skill-aware host (Claude Code, Cursor) and want a preset toolset |
| **Agent workers** (`turn-orchestrator`, `agent`, `coding`, …) | The brain itself — registers `run::start_and_wait`, runs LLM turns, executes tools | You're calling iii from your own code (`iii.trigger("run::start_and_wait", …)`) or backend automation |
| **`acp` (this worker)** | Editor → iii. Translates ACP `session/*` JSON-RPC into the canonical iii brain contract; turns iii's `agent::events` stream into ACP `session/update` notifications | You want iii to **be** the agent in an editor users already opened today |

ACP is the **north** edge of the stack. MCP is the **south** edge. They
coexist:

```
Editor (Zed, VS Code, Neovim, …)
  ↓ ACP ─ session/prompt, session/update          ← acp
iii engine + brain workers (turn-orchestrator,
  provider-router, guardrails, llm-budget,
  audit-log, dlp-scrubber, policy-denylist, …)
  ↓ MCP ─ tools/list, tools/call                  ← iii-mcp / mcp-client
External tool servers (filesystem, browser, …)
```

What iii brings to an editor session that a vanilla agent host doesn't:

- **Provider routing** — switch Claude/GPT/local model per session via `provider-router`, no editor restart
- **Budgets** — hard token/dollar caps per session via `llm-budget`
- **Guardrails** — input + output PII/secret scrub via `guardrails`, `dlp-scrubber`
- **Audit trail** — every turn durably logged via `audit-log`
- **RBAC** — per-session tool gating via `iii-worker-manager`
- **Observability** — full distributed trace from editor click to provider API call via `engine::traces::*`
- **Durable sessions** — `session/load` replays history straight from iii state

## Supported clients

ACP is an open spec. Any client that speaks it works with `acp`. As of
this writing the public client list ([agentclientprotocol.com/get-started/clients](https://agentclientprotocol.com/get-started/clients))
includes:

**Editors / IDEs**

| Client | How to wire acp |
|---|---|
| [Zed](https://zed.dev) | `agent_servers` block in `~/.config/zed/settings.json` (snippet below) |
| Visual Studio Code | ACP Client extension |
| JetBrains | ACP plugin |
| Neovim | [CodeCompanion](https://github.com/olimorris/codecompanion.nvim), [carlos-algms/agentic.nvim](https://github.com/carlos-algms/agentic.nvim), or [yetone/avante.nvim](https://github.com/yetone/avante.nvim) |
| Emacs | [agent-shell.el](https://github.com/xenodium/agent-shell) |
| Obsidian | Agent Client plugin |
| Unity | Unity ACP Client / Unity Agent Client |
| Chrome | Chrome ACP |

**CLIs / Apps / Notebooks / Mobile**

`acpx` (CLI), `Agent Studio`, `AionUi`, `aizen`, `DeepChat`, `gemini-cli-desktop`,
`Harnss`, `iflow-cli`, `Jockey`, `Lody`, `Minion Mind`, `Mitto`, `Nori CLI`,
`Ngent`, `pool`, `RayClaw`, `RLM Code`, `Sidequery`, `Tidewave`, `Toad`,
`Web Browser with AI SDK`, `agent-client-kernel` (Jupyter), DuckDB
(via `sidequery/duckdb-acp`), `marimo`, `Agmente` (iOS), `Ferngeist` (Android),
`Happy`, `Mobvibe` (mobile), `OpenACP` (Telegram/Discord/Slack), and others.

Setup pattern is the same everywhere: point the client at the `acp`
binary, set the `IIIACP_*` env vars below.

## Prerequisites

`acp` needs an iii engine plus a brain. Minimum stack:

```bash
# 1. Engine builtins acp uses directly. iii-state holds session
#    records + history; iii-stream carries the agent::events tape;
#    iii-queue backs durable cancel topics.
iii worker add iii-state iii-stream iii-queue

# 2. acp itself.
iii worker add acp

# 3. The brain stack. turn-orchestrator drives the loop;
#    provider-router routes assistant turns to provider-anthropic
#    (or any other provider worker); auth-credentials stores the
#    Anthropic API key. session-inbox / llm-budget / hook-fanout
#    are pulled in transitively.
iii worker add turn-orchestrator provider-router provider-anthropic auth-credentials \
                session-inbox llm-budget hook-fanout

# 4. (Optional but recommended) — iii's distinctive primitives.
iii worker add guardrails dlp-scrubber audit-log policy-denylist context-compaction
```

Store the Anthropic API key once:

```bash
iii trigger \
  --function-id auth::set_token \
  --payload '{"provider":"anthropic","credential":{"type":"api_key","key":"sk-ant-..."}}'
```

Verify the brain runs end-to-end before plugging in an editor:

```bash
iii trigger --function-id run::start_and_wait --payload '{
  "session_id": "smoke",
  "messages": [{"role":"user","content":[{"type":"text","text":"reply with hi"}],"timestamp":0}],
  "model": "claude-sonnet-4-5-20250929",
  "provider": "anthropic"
}' --timeout-ms 30000
```

A `messages` array ending in an assistant message with `"text":"hi"` means
the stack is healthy.

## Spawn

`acp` is a stdio agent. The client launches it as a subprocess and
exchanges JSON-RPC frames over stdin/stdout. **stderr is reserved for
logs; stdout is reserved for ACP frames.**

```bash
acp --use-canonical-brain --model claude-sonnet-4-5-20250929 --provider anthropic
```

## Configuration

| Flag / env | Effect |
|---|---|
| `--engine-url` (`-e`, `IIIACP_ENGINE_URL`) | iii engine WebSocket URL. Default `ws://localhost:49134`. |
| `--debug` (`-d`) | Verbose tracing on stderr. |
| `--brain-fn` (`IIIACP_BRAIN_FN`) | iii function id that runs the prompt turn. Falls back to a built-in echo brain when unset. Canonical value is `run::start_and_wait` (turn-orchestrator). |
| `--use-canonical-brain` (`IIIACP_USE_CANONICAL_BRAIN`) | Shortcut for `--brain-fn run::start_and_wait`. |
| `--model` (`IIIACP_MODEL`) | Model id forwarded to the brain (e.g. `claude-sonnet-4-5-20250929`). |
| `--provider` (`IIIACP_PROVIDER`) | Provider id forwarded to the brain (e.g. `anthropic`). Routes to `provider::<provider>::complete`. |
| `--system-prompt` (`IIIACP_SYSTEM_PROMPT`) | System prompt prepended to every turn. |
| `--rbac-tag` | Forwards `x-iii-rbac-tag` on the worker WebSocket so `iii-worker-manager`'s `auth_function_id` can apply policy. |

## Editor wiring

### Zed

`~/.config/zed/settings.json`:

```jsonc
{
  "agent_servers": {
    "acp": {
      "type": "custom",
      "command": "/path/to/acp",
      "args": [],
      "env": {
        "IIIACP_ENGINE_URL": "ws://localhost:49134",
        "IIIACP_USE_CANONICAL_BRAIN": "true",
        "IIIACP_MODEL": "claude-sonnet-4-5-20250929",
        "IIIACP_PROVIDER": "anthropic",
        "IIIACP_SYSTEM_PROMPT": "You are an iii expert. Answer in iii primitives only."
      }
    }
  }
}
```

Restart Zed → Agent panel → `+` → pick **acp** → type a prompt.

### VS Code

Install the ACP Client extension. Add `acp` as a custom agent in the
extension's settings, pointing `command` at the binary and replicating the
`env` block above.

### Neovim

Pick one of the ACP plugins listed under "Supported clients" and follow its
docs. Each one exposes a `command` + `env` config the same way Zed does;
the same env vars work.

### JetBrains / Emacs / Obsidian / Unity / Chrome

Same pattern: client config takes a command path and env map. Point at
`acp` with the env vars above. The protocol is the same on all sides.

### CLIs (`acpx`, `Nori CLI`, …)

Most CLI ACP clients accept `--agent <command>` or a config file. Point them
at `acp` directly.

## Methods

| Method | Direction | Status |
|---|---|---|
| `initialize` | client → agent | implemented |
| `authenticate` | client → agent | no-op success |
| `session/new` | client → agent | implemented |
| `session/load` | client → agent | implemented; replays history as `session/update` notifications |
| `session/resume` | client → agent | implemented; refreshes cwd + mcpServers, no history replay |
| `session/list` | client → agent | implemented; dedupes index on read |
| `session/prompt` | client → agent | implemented; routes to brain fn or echo |
| `session/cancel` | client → agent | implemented; flips in-process abort + publishes cancel topic |
| `session/close` | client → agent | implemented; full cleanup of record + history + index |
| `session/set_mode` | client → agent | implemented; persists `modeId` on session record for the brain |
| `session/set_config_option` | client → agent | implemented; persists `configId`/`value` pairs on session record |
| `session/update` | agent → client | streamed during prompt turn |
| `session/request_permission` | agent → client | deferred — needs reverse-RPC framer |
| `fs/read_text_file`, `fs/write_text_file` | agent → client | deferred — internal iii brains use iii fs primitives directly |
| `terminal/create`, `terminal/kill`, `terminal/output`, `terminal/release`, `terminal/wait_for_exit` | agent → client | deferred — internal iii brains use iii terminal primitives directly |

`agentCapabilities.sessionCapabilities` advertises `list`, `close`, and
`resume` on `initialize`. The `loadSession` capability (top-level for now,
unifying with sessionCapabilities is on the ACP roadmap) is also advertised.

`session/set_mode` and `session/set_config_option` ship without an explicit
capability flag because the version of the ACP schema in this repo doesn't
include those slots in `SessionCapabilities` yet — clients that try them
get a successful response; clients that don't try are unaffected. Both
methods persist their values on the session record (`mode: Option<String>`
and `config_options: Map<String, Value>`). Brains opt in to honoring them
on the next `session/prompt` turn; acp just stores.

## Brain contract

acp talks to the canonical iii brain shape used by `turn-orchestrator`
and every provider worker. Any function with this input contract drops in
as a brain — no adapter required.

**Input** (forwarded as `iii.trigger` payload):

```jsonc
{
  "session_id": "sess_...",
  "messages": [
    {
      "role": "user",
      "content": [{"type": "text", "text": "..."}],
      "timestamp": 1234567890
    }
  ],
  "model": "claude-sonnet-4-5-20250929",
  "provider": "anthropic",
  "system_prompt": "You are ...",
  "timeout_ms": 600000
}
```

**Output:**

```jsonc
{
  "session_id": "sess_...",
  "messages": [...],
  "turn_count": 1
}
```

acp picks the ACP `stopReason` from the final assistant message's
`stop_reason` field (`end` → `end_turn`, `length` → `max_tokens`,
`aborted` → `cancelled`, `error` → `refusal`).

**Streaming.** The brain emits `AgentEvent` frames into `agent::events`
(group_id = session_id). acp registers one stream subscriber per
connection at startup and translates each event:

| `AgentEvent` | ACP `sessionUpdate` |
|---|---|
| `message_update { llm_event: text_delta }` | `agent_message_chunk` |
| `message_update { llm_event: thinking_delta }` | `agent_thought_chunk` |
| `message_complete` (assistant role, full text) | `agent_message_chunk` (one shot) |
| `tool_execution_start` | `tool_call` (status: `in_progress`) |
| `tool_execution_end` | `tool_call_update` (status: `completed`/`failed`) |
| other | dropped |

This is the same stream `context-compaction` and every provider worker
already subscribe to. **No bespoke acp publish protocol.**

## State layout

All keys live in scope `acp`. `connId` is regenerated per subprocess so
concurrent editors don't collide.

```
<connId>:sessions:_index           = ["sess_a", "sess_b", ...]
<connId>:sessions:<sessId>         = { sessionId, connId, cwd, mcpServers, created_at_ms, last_activity_ms }
<connId>:sessions:<sessId>:history = [ session/update entries ... ]
```

Streaming wire: `agent::events` (per-session events), per-connection topic
`acp:<connId>:session:<sessId>:cancel` (best-effort cancel signal).

## Wire example (raw stdio)

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"demo","version":"0"}}}' \
  | acp --use-canonical-brain --model claude-sonnet-4-5-20250929 --provider anthropic
```

Replies stream on stdout, one JSON frame per line.

## Tests

```bash
cargo test
```

17 lib + 7 protocol envelope tests. Integration smoke against a live engine
lives in the iii test harness.
