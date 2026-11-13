# MCP gateway behind rbac-proxy

The `mcp` worker becomes the way agents outside the engine (Claude Code,
Codex, OpenCode, Cursor, any MCP client) reach a running iii engine: a
Streamable HTTP MCP server offering a small, fixed set of tools, `iii_call` and
`iii_observe` above all, that must run behind `rbac-proxy`.

Status: implemented in `mcp` 0.7.0 (this PR) and `rbac-proxy`
(`rbac-proxy::session`, separate PR). This document is for review: it
describes the behaviour and contracts, the security model, and what is
deliberately left out.

## Why

Before this change `mcp` had three problems:

1. **One tool per function.** `tools/list` advertised every visible engine
   function as its own MCP tool. A real engine has hundreds of functions, so
   clients drowned in tools, and anything hidden by `hidden_prefixes`
   (`engine::`, `state::`) could not be reached at all, discovery included.
2. **No way to react.** MCP clients could call functions but not observe them;
   agents had to poll.
3. **No access control and a dead dependency.** Any client that could reach
   the HTTP endpoint (including a web page, via the browser) could call every
   advertised function, and resources/prompts delegated to the `skills` and
   `prompts` workers, which no longer exist.

The Claude Code integration (`iii` plugin) needs exactly `iii_call`,
`iii_observe` and profiles/skills from `iii-directory`, and every other
harness speaks HTTP MCP. One server, done properly, serves them all.

## Shape

```text
MCP client ──HTTP──▶ http worker ──http::on-request──▶ mcp::handler
 (Claude Code,        (POST/DELETE /mcp)                 │  gateway tools, sessions,
  Codex, ...)                                            │  Origin check
                                                         ▼
                                     mcp worker's engine connection
                                                         │  ws://…:49200 + Bearer III_TOKEN
                                                         ▼
                                                    rbac-proxy ── auth function, expose_functions,
                                                         │        observer gating, discovery filtering
                                                         ▼
                                                       engine ── every worker's functions
```

- `mcp` keeps one iii-sdk connection. In production it points at
  rbac-proxy, so **every** call, observer and discovery result the MCP
  server makes is decided by RBAC. `mcp` itself has no allow-list for the
  gateway tools.
- All MCP clients share that one proxied identity: RBAC governs "agents via
  MCP" as a whole (see [Open questions](#open-questions)).

## Startup: RBAC or nothing

Before registering any function or route, `mcp` calls
`rbac-proxy::session` on its connection.

### `rbac-proxy::session` (rbac-proxy)

`rbac-proxy::status` cannot answer "is *this* connection proxied": it is an
ordinary engine function, callable by workers connected straight to the
engine. `rbac-proxy::session` is answered by the proxy's interceptor itself,
for every proxied connection, **before** any RBAC gate, and is never forwarded.

| Connection | Result |
|---|---|
| through rbac-proxy | `{ proxied: true, proxy_version, session_id, function_registration_prefix, policy: { auth_function, expose_functions, exposes_everything } }` |
| straight to the engine | `function_not_found` |

`exposes_everything` is true when an id rule matches arbitrary ids in
`default` (e.g. `match("*")`). The proxy refuses to *register* a function with
this id, so a proxied client cannot plant a fake for direct connections.

### Decision

`III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC` (namespaced so it switches off nothing
else) accepts exactly `true` or `1`; unset or empty is off.

| Probe result | Flag | Outcome |
|---|---|---|
| proxied | off | start |
| proxied, `exposes_everything` and no auth function | off | start, warning: RBAC restricts nothing |
| proxied | on | start; note that the flag is ignored |
| `function_not_found` (or an answer without `proxied: true`) | off | **refuse**, exit 1 |
| `function_not_found` | on | start in *dangerous* mode: log warning, MCP clients told (below) |
| anything else (timeout, connection refused) | any | **refuse**: "could not check for rbac-proxy" |
| any | any other value | **refuse**: "… is not a recognised value; use true or 1, or unset it." |

Refusal text when pointed at the engine directly:

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

In dangerous mode, `initialize.instructions` starts with, and `iii_status`
returns as `warning`: "WARNING: this MCP server runs WITHOUT RBAC
(III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC). Every client can call every function
on this engine."

Credentials: iii-sdk sends `Authorization: Bearer $III_TOKEN` on the WebSocket
handshake; rbac-proxy's auth function receives it. No `mcp` code is involved.

An rbac-proxy with no `auth_function_id` refuses every upgrade, so in practice
a proxied `mcp` is always authenticated.

### Running under compose

rbac-proxy renames a proxied worker to `rbac-proxy:<session or prefix>:<name>`,
and compose waits for a worker by its exact name, so it would never see `mcp`
start, and would kill it when `startup_timeout` ran out. The auth function's
result can list `worker_names: ["mcp"]`: rbac-proxy then forwards that
declared name unchanged (exact match, no patterns) and `rbac-proxy::session`
reports the grant. Compose is unchanged. A granted name is shared with every
connection holding it, so the auth function grants it only to the credential
that runs `mcp`.

## HTTP and sessions

The worker observes `http::on-request` for `POST <api_path>` and
`DELETE <api_path>` (default `/mcp`). Each request is handled in this order:

1. **Origin.** No `Origin` header: allowed (CLI/SDK clients). Loopback origins
   (`localhost`, `*.localhost`, `127.0.0.1`, `[::1]`, any scheme and port):
   allowed. Exact matches in `allowed_origins`: allowed. Anything else,
   `null` and malformed origins included: **403**.
2. **Method.** `DELETE` with `Mcp-Session-Id` ends the session (**200**;
   **404** unknown; **400** without the header). Anything but `POST`/`DELETE`:
   **405**.
3. **JSON-RPC shape.** Batches and non-JSON-RPC bodies: -32600 (HTTP 200).
4. **Session.** `initialize` always opens a new session and returns its id in
   the `Mcp-Session-Id` response header. Any other request carrying an unknown
   or expired id gets **404** ("Session not found"), which tells the client to
   initialize again. Requests without the header are served; only the
   session-scoped tools need one.
5. **Dispatch.** Notifications: **202**, no body. Requests: one SSE `message`
   event with the JSON-RPC reply.

Refusals at steps 1, 2 and 4 carry a JSON-RPC error (`-32000`, id `null`) in
the body.

### Session lifecycle

| Event | Effect |
|---|---|
| `initialize` | new session (UUID v4) |
| any request with its id | refreshes `last_seen` |
| idle longer than `session_idle_ttl_secs` (default 1800) | ended by the reaper (runs every 60 s) |
| `DELETE` | ended |
| worker shutdown | every session ended |
| ended | all its observers disposed on the engine; buffers dropped |

Sessions are in memory: a restart of `mcp` loses them, and clients get 404
and re-initialize.

## Tools

`tools/list` offers exactly these eleven, in this order.

| Tool | Input | Output | Session |
|---|---|---|---|
| `iii_call` | `function_id` (required), `payload` (object, default `{}`), `timeout_ms`, `void`, `max_chars` (default 100000) | `{ result }`; `{ result: <cut JSON text>, truncated: true, total_chars }` when larger than `max_chars`; `{ result: null, void: true }` for `void` | no |
| `iii_secret_store` | `name`, `value`, `consumers` (all required), `description` | `{ ref, hint, fingerprint, consumers }` | no |
| `iii_observe` | `function_id` (required), `filters`, `args`, `capture`, `namespace`, `max_events`, `expires_in_ms` | `{ observer_id, function_id, state }` | yes |
| `iii_events` | `observer_id` (optional: all of the session's), `max` (default 50) | `{ events, remaining, dropped }` | yes |
| `iii_observers` | — | `[{ observer_id, function_id, filters?, state, buffered, received, dropped }]` | yes |
| `iii_unobserve` | `observer_id` (required) | `{ observer_id, removed: true, discarded }` | yes |
| `iii_status` | — | `{ server, version, session_id, rbac: "rbac-proxy" \| "disabled", policy?, warning? }` | no |
| `iii_list_agents` | `include_hidden` | profiles from `directory::agents::list` | no |
| `iii_get_agent` | `id` (required) | `directory::agents::get` | no |
| `iii_list_skills` | `search`, `prefix` | rows from `directory::skills::list` | no |
| `iii_get_skill` | `id` (required) | the skill's markdown body | no |

Rules shared by all tools:

- Engine errors are **tool results** with `isError: true` and the engine's
  `code: message`, so the model reads and reacts to them; they are not
  JSON-RPC errors.
- Argument errors (missing `function_id`, non-object `payload`) are tool
  errors and make no engine call.
- `iii_call` refuses `mcp::*` (the worker's own internals) and
  `secrets::resolve` (see Secrets). Nothing else is filtered locally: RBAC
  decides.
- A session tool without a session returns a tool error telling the client to
  initialize and send `Mcp-Session-Id`.
- An unknown tool name is a tool error ("unknown tool"); function-style
  names (`myworker__echo`) are **not** forwarded.

`initialize.instructions` tells the model to use `iii_call` instead of the
`iii` CLI, `iii_observe` instead of polling, and that `agent_call` /
`harness::observe` in harness-written profiles mean `iii_call` /
`iii_observe`.

Earlier versions offered one tool per visible function; 0.7 removes that
mode, with `hidden_prefixes` and `require_expose`.

## Secrets

An MCP client can hold credentials (a user pastes an API key into the chat;
another MCP server hands one over). Sent in an `iii_call` payload, a value
would be recorded in trace spans, copied to observers, and kept in the target
function's input. Instead:

- `iii_secret_store` validates `name` (`^[A-Za-z_][A-Za-z0-9_.-]{0,127}$`,
  not `III_SECRETS_KEY`), a non-empty string `value` (stored byte for byte)
  and at least one consumer, then calls `secrets::set` (vault store). It
  returns only `ref`, `hint`, `fingerprint` and `consumers`, whatever
  `secrets` answers. Storing an existing name rotates it.
- `mcp` is never a consumer (refused in `consumers`), and `iii_call` refuses
  `secrets::resolve`: references go out, values never come back to a client.
- `mcp::handler` receives every request body, so it is registered
  `observable: false`.
- `mcp` sets `III_DISABLE_TRACE_PAYLOADS=1` before connecting, as `secrets`
  does, so its spans carry no payloads.
- No tool result or error message contains the value. A missing `secrets`
  worker is reported with how to add it.

rbac-proxy's `expose_functions` must include `secrets::set`. The instructions
tell the model never to put a credential in an `iii_call` payload.

## Observe

An MCP server cannot wake its client mid-turn, so observing is buffered and
polled.

- `iii_observe` creates an engine observer whose **target is
  `mcp::observed`**, one sink function for the whole worker. The sink routes
  each delivery to its session by `observer_id`; deliveries for observers that
  are not live (unobserved, ended session, unknown id) are dropped and never
  resurrected.
- The sink is **public**: the engine refuses a private function as an observer
  target. It is marked `internal` and `trace_hidden`. Anyone who can call it
  can only inject events for an observer id they already know (UUIDs).
- Each observer keeps up to `max_buffered_events` (default 500); beyond that
  the oldest are dropped and counted. `iii_events` reports `dropped` once,
  then resets it.
- `iii_events` without `observer_id` merges the session's observers in arrival
  order.
- `iii_observe` waits up to 5 s for activation. An observer of a function no
  one hosts yet stays `pending` and is returned as such. A refused observer
  (bad filter, unknown `capture`, RBAC `FORBIDDEN`) is disposed and reported
  as `observe refused: <code>: <message>`. Unknown `capture` names are refused
  before the engine is asked.
- `max_events` maps to the observer's `max_fires`, `expires_in_ms` to its
  expiry; the engine enforces both.

## Resources and prompts

Skills are resources at `iii://<id>`, read from `iii-directory`:
`resources/list` (`directory::skills::list`), `resources/read`
(`directory::skills::get`), `resources/templates/list` (`iii://{id}`). An
unknown skill, a foreign URI, or a missing directory give empty envelopes; any
other engine error is a JSON-RPC internal error.

No prompts capability. `prompts/list` and `prompts/get` are method not
found (-32601). Profiles are reached through the gateway
tools and, in Claude Code, the `iii` plugin's `/iii:*` commands.

## Configuration

| Key | Default | Applies to |
|---|---|---|
| `allowed_origins` | `[]` | Origin check |
| `session_idle_ttl_secs` | `1800` | sessions |
| `max_buffered_events` | `500` | observe |
| `api_path` | `/mcp` | routes |

| Environment | |
|---|---|
| `--url`, then `III_MCP_ENGINE_URL`, then `III_URL` | engine URL: rbac-proxy's port in production. Compose sets `III_URL` to the engine itself, hence the dedicated variable. |
| `III_TOKEN` | bearer token for rbac-proxy's auth function |
| `III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC` | `true` or `1` to start without rbac-proxy |

Keys from earlier versions (`tools`, `hidden_prefixes`, `require_expose`) are
ignored. The manifest declares `http`, `rbac-proxy` and `secrets` as
dependencies.

## Security model

| Threat | Mitigation | Residual |
|---|---|---|
| MCP clients calling anything on the engine | rbac-proxy: auth function + `expose_functions` + observer gating + discovery filtering on `mcp`'s connection | one shared identity for all MCP clients |
| `mcp` accidentally deployed straight to the engine | boot refusal via `rbac-proxy::session` | the escape hatch exists, and says so to clients |
| A web page driving a localhost MCP server (DNS rebinding) | Origin check (MCP spec requirement) | non-browser local processes are trusted, as with any localhost service |
| Faking "behind the proxy" | probe answered only by the proxy, never forwarded; proxy refuses registering the id | anything that can register functions on the engine *directly* could register a fake; documented, it guards misconfiguration, not an attacker with engine access |
| Event injection into a session | sink only routes to live observer ids (UUIDv4) | an attacker who learns an observer id can inject events into that observer's buffer |
| Credentials in tool calls | `iii_secret_store`; handler not observable; no trace payloads; no value in results; `secrets::resolve` refused | the value crosses the `http` worker, whose own records may hold it, and the client's model has seen it |
| Resource exhaustion | per-observer buffer cap, idle session TTL, engine-side `max_fires` / expiry | no cap on sessions or observers per session |

## Compatibility

Breaking (hence 0.7.0):

- clients see the eleven gateway tools, not one tool per function; the
  per-function mode is gone.
- `mcp` will not start unless proxied (or the flag is set).
- no prompts capability (the methods are -32601); resources come from
  `iii-directory`.
- the engine URL is read from `III_MCP_ENGINE_URL` before `III_URL`.

The registry still shows 0.5.9's stale `skills` dependency; publishing 0.7.0
replaces it.

## Testing

- 93 unit tests in `mcp`, written first: the guard's decision table and
  messages, Origin parsing, the directory bridge, sessions (routing, buffering,
  drop counting, unobserve, end, reaping, close), the gateway tools, the HTTP
  handler (status codes, sessions, resources), `iii_secret_store` (stored
  payload, reference-only result, argument errors without the value, `mcp`
  refused as consumer, missing `secrets`), the `secrets::resolve` refusal,
  the SDK adapter's `capture` parsing, config, manifest, declared
  dependencies. The engine is a fake `Bus`.
- 6 tests in `rbac-proxy` for the probe (answered before RBAC, policy facts,
  void calls, refused registration) and 7 for `worker_names`.
- 26 BDD scenarios against a live engine, including `mcp::handler` listed
  as not observable, including observe end to end
  (`state::set` filtered by scope, buffered, read, session ended, then 404).
- Manually, on a compose project: `mcp` behind rbac-proxy with a bearer-token
  auth function granting `worker_names: ["mcp"]` (compose reports it ready),
  the same end-to-end path through the http worker, `iii_secret_store`
  through the endpoint (stored, reference only, `secrets::resolve` refused,
  deleted), and both refusal paths (unproxied; unrecognised flag value).

## Open questions

1. **Per-client identity.** All MCP clients share `mcp`'s proxied identity.
   Passing each MCP session's credentials through (one upstream connection
   per session, rbac-proxy-style) would give per-user RBAC. Worth doing before
   remote/shared engines?
2. **Telemetry through the proxy.** iii-helpers' OTel exporter connects to
   `<url>/otel`, which rbac-proxy does not forward yet, so a proxied `mcp`
   logs a 404 and exports no telemetry. Decided: rbac-proxy will forward
   `/otel`, authenticated like the main connection.
3. **Push instead of poll.** Streamable HTTP allows a `GET` SSE stream for
   server-initiated messages. Clients that hold it open could receive events
   (or `notifications/resources/updated`) instead of polling `iii_events`.
   Claude Code cannot be woken mid-turn either way.
4. **Limits.** Caps on sessions and observers per session, and whether
   `session_idle_ttl_secs` should be shorter by default.
5. **Durable sessions.** Sessions and buffers are in memory; should observers
   survive an `mcp` restart (state-backed)?
