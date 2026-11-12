# mcp

Model Context Protocol (MCP) server for the [iii engine](https://iii.dev), as a
single Rust binary worker. It serves MCP 2025-06-18 over **Streamable HTTP**
(`POST` and `DELETE` on `/mcp`, through the `http` worker) and gives any MCP
client (Claude Code, Codex, OpenCode, Cursor, ...) the whole engine through a
small, fixed set of tools:

- **`iii_call`** calls any function by id;
- **`iii_observe`** observes a function, buffering every matching invocation
  in the client's MCP session, read with **`iii_events`**;
- agent profiles and skills come from the `iii-directory` worker.

It runs **behind [`rbac-proxy`](../rbac-proxy)**: RBAC decides what MCP
clients may call and observe. It refuses to start otherwise, unless you set
`III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC=true`.

## Install

```bash
iii call compose::add worker=mcp
```

`http` and `rbac-proxy` are declared dependencies. Configure rbac-proxy's
`rbac` block (an `auth_function_id` and `expose_functions`), then start `mcp`
with its engine URL pointing at the proxy and a token the auth function
accepts:

```bash
III_TOKEN=<token> mcp --url ws://127.0.0.1:49200   # rbac-proxy's port
```

iii-sdk sends `Authorization: Bearer $III_TOKEN` on the WebSocket handshake;
that is what rbac-proxy's auth function receives.

Clients connect to the http worker's URL plus `api_path`, e.g.
`http://127.0.0.1:3111/mcp` (`http::status` reports the base URL). For Claude
Code:

```bash
claude mcp add --transport http iii http://127.0.0.1:3111/mcp
```

## RBAC requirement

`iii_call` and `iii_observe` can reach every function on the engine, so before
registering anything the worker calls `rbac-proxy::session`. rbac-proxy
answers that probe itself for every proxied connection and never forwards it;
a connection made straight to the engine gets `function_not_found`.

| Situation | Outcome |
|---|---|
| Behind rbac-proxy | Starts. If the proxy exposes every function **and** has no auth function, it logs a warning. |
| Not behind rbac-proxy | Refuses to start (exit 1) with the message below. |
| Not behind rbac-proxy, `III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC=true` or `1` | Starts, logs a warning, and tells every MCP client (initialize `instructions`, `iii_status`). |
| `III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC` set to anything else | Refuses to start, naming the accepted values. |
| Behind rbac-proxy with the flag set | Starts; the flag is ignored with a note. |
| The probe cannot be made (engine or proxy down, timeout) | Refuses to start. |

The refusal you will see when the URL points at the engine directly:

```text
mcp: refusing to start: not behind rbac-proxy.
  The MCP server exposes iii_call and iii_observe, which can reach every function
  on this engine. It must connect through rbac-proxy so RBAC decides what MCP
  clients may do.
  Connected to: ws://127.0.0.1:49134 (rbac-proxy::session: function_not_found)
  Fix: install rbac-proxy (iii call compose::add worker=rbac-proxy), configure its
  rbac block, and start mcp with --url ws://<rbac-proxy host>:<port> (default 49200)
  and III_TOKEN set to a token its auth function accepts.
  To run without RBAC anyway (local development only):
    III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC=true
```

The check guards against a misconfigured URL, not an attacker: anything that
can register functions on the engine directly could register a fake
`rbac-proxy::session` there. rbac-proxy refuses to register that id for the
connections it serves.

All MCP clients share the worker's one proxied identity: RBAC decides what
"agents via MCP" may do as a whole.

## Tools

`tools: gateway` (the default) offers exactly these:

| Tool | What it does |
|---|---|
| `iii_call` | Call a function by id with a JSON `payload` (default `{}`). `timeout_ms`; `void: true` to fire and forget; results over `max_chars` (default 100000) come back cut, with `truncated` and `total_chars`. `mcp::` functions (this worker's internals) are refused. |
| `iii_observe` | Observe a function: `function_id`, `filters`, `args`, `capture`, `namespace`, `max_events`, `expires_in_ms`. Returns `{ observer_id, function_id, state }`. Needs a session. |
| `iii_events` | Read and remove buffered events, oldest first, for one `observer_id` or all of the session's; `max` (default 50). Returns `{ events, remaining, dropped }`. Needs a session. |
| `iii_observers` | The session's observers with `state`, `buffered`, `received`, `dropped`. Needs a session. |
| `iii_unobserve` | Remove one observer, discarding its unread events. Needs a session. |
| `iii_status` | Server version, session id, and `rbac` (`rbac-proxy` with its policy, or `disabled` with a warning). |
| `iii_list_agents`, `iii_get_agent` | Agent profiles from `directory::agents::*` (hidden ones with `include_hidden`). |
| `iii_list_skills`, `iii_get_skill` | Skills from `directory::skills::*` (`search`, `prefix`). |

Engine errors come back as tool results with `isError: true` and the engine's
code, so the model reads them. What a call may reach is decided by RBAC, not
by this worker.

`tools: functions` restores the earlier behaviour (one MCP tool per visible
function, `myworker::echo` as `myworker__echo`, filtered by `hidden_prefixes`
and `require_expose`); `tools: both` offers the gateway tools first, then
those.

## Sessions and observe

`initialize` opens a session and returns its id in the `Mcp-Session-Id`
response header; clients send it back on every request. A request carrying an
unknown or expired id gets **404**, which tells the client to initialize
again. `DELETE /mcp` with the header ends the session (200; 404 if unknown,
400 without the header). Requests without the header still work for every
tool except the observe ones.

An MCP server cannot wake a client mid-turn, so observing is buffered:

- every observer a session creates targets `mcp::observed`, one sink function
  for the worker, which routes each delivery to its session by `observer_id`;
  deliveries for observers that are not live are dropped;
- each observer buffers up to `max_buffered_events` (default 500); beyond
  that the oldest are dropped and counted;
- a session idle for `session_idle_ttl_secs` (default 1800) is ended and its
  observers disposed; the worker checks every minute and disposes all
  observers on shutdown;
- an observer of a function nobody hosts yet stays `pending`; `iii_observe`
  waits up to 5 s for activation and returns the state either way. A refused
  observer (bad filter, unknown capture, RBAC) is reported with the engine's
  code and never kept.

`mcp::observed` is public because the engine refuses a private function as an
observer target; it is marked internal and hidden from traces.

## Origin

The MCP spec requires validating `Origin` so a web page cannot drive a local
MCP server through the browser (DNS rebinding included):

- no `Origin` header (CLI and SDK clients): allowed;
- loopback origins (`localhost`, `*.localhost`, `127.0.0.1`, `[::1]`, any
  scheme or port): allowed;
- origins listed exactly in `allowed_origins`: allowed;
- anything else, `null` and malformed origins included: **403**.

## Resources and prompts

Skills are resources at `iii://<id>` (`resources/list`, `resources/read`,
`resources/templates/list`), read from `iii-directory`. An unknown skill, a
foreign URI or a missing directory give empty envelopes. The server offers no
prompts: `prompts/list` answers `[]` for clients that ask anyway, and
`prompts/get` is invalid params.

## Method coverage

| Method | Behaviour |
|---|---|
| `initialize` | `{ protocolVersion: "2025-06-18", capabilities: { tools, resources }, serverInfo, instructions }`; opens a session |
| `ping` | `{}` |
| `notifications/*` | 202, no body |
| `tools/list`, `tools/call` | per `tools` mode, above |
| `resources/list`, `resources/read`, `resources/templates/list` | iii-directory skills |
| `prompts/list` / `prompts/get` | `[]` / invalid params |

Responses are one SSE `message` event carrying the JSON-RPC reply. Batch
requests are refused (-32600). Out of scope: `resources/subscribe`,
notifications fan-out, server-initiated requests, the stdio transport.

## Configuration

```yaml
tools: gateway                 # gateway | functions | both
allowed_origins: []            # exact origins allowed besides loopback
session_idle_ttl_secs: 1800
max_buffered_events: 500
hidden_prefixes:               # functions mode only
  - "engine::"
  - "state::"
  - "stream::"
  - "iii."
  - "iii::"
  - "mcp::"
  - "a2a::"
require_expose: false          # functions mode only
api_path: "/mcp"
```

A missing or malformed config file logs a warning and falls back to the
defaults.

| Environment | Meaning |
|---|---|
| `III_URL` / `--url` | Engine WebSocket URL: rbac-proxy's port in production |
| `III_TOKEN` | Bearer token sent to rbac-proxy's auth function |
| `III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC` | `true` or `1` to start without rbac-proxy (local development only) |

## Functions

| Function | What it is |
|---|---|
| `mcp::handler` | The endpoint. Observes `http::on-request` with `{ method: "POST", path: <api_path> }` and `{ method: "DELETE", path: <api_path> }`. |
| `mcp::observed` | The target of every session observer; internal. |

## Development

```bash
cargo test --lib                       # unit tests, no engine needed
III_ENGINE_WS_URL=ws://127.0.0.1:49134 \
III_ENGINE_HTTP_URL=http://127.0.0.1:3111/mcp \
  cargo test --test bdd                # end to end against a running engine
```

The BDD harness connects straight to the engine (no rbac-proxy), so it runs
in the mode `III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC` would select.
