# MCP gateway behind rbac-proxy

The `mcp` worker becomes the way agents outside the engine (Claude Code,
Codex, OpenCode, Cursor, any MCP client) reach a running iii engine: a
Streamable HTTP MCP server offering a small, fixed set of tools, `iii_call` and
`iii_observe` above all, that must run behind `rbac-proxy`.

Status: implemented in `mcp` 0.7.0 (this PR) and `rbac-proxy`
(`rbac-proxy::session`, `worker_names`, `expose_endpoints`, #1712). This document is for review: it
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
 (Claude Code,        (POST/GET/DELETE /mcp)             │  gateway tools, sessions,
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

`rbac-proxy::status` reports on the proxy as a whole: where it listens, its
upstream, whether an auth function is set, the exposed endpoints and the live
connection count. `rbac-proxy::session` reports on the connection that calls
it: the proxy's interceptor answers it for every proxied connection,
**before** any RBAC gate, and never forwards it.

| Connection | Result |
|---|---|
| through rbac-proxy | `{ proxied: true, proxy_version, session_id, function_registration_prefix, worker_names, policy: { auth_function, expose_functions, expose_endpoints } }` |
| straight to the engine | `function_not_found` |

`policy` carries the `expose_functions` and `expose_endpoints` rules as
written (`match("api::*")`, `{ match, namespace }`, `{ metadata }`), not a
verdict: a client reads the globs and decides what to warn its users about.
The proxy refuses to *register* a function with this id, so a proxied client
cannot plant a fake for direct connections.

### Decision

`III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC` accepts exactly `true` or `1`; unset or
empty is off.

| Probe result | Flag | Outcome |
|---|---|---|
| proxied | off | start |
| proxied, `expose_functions` include `match("*")` and no auth function | off | start, warning: RBAC restricts nothing |
| proxied | on | start; `info` log that the flag is ignored |
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
  Fix: install rbac-proxy (iii call compose::add worker=rbac-proxy), set its
  rbac.auth_function_id, and give mcp:
    III_MCP_ENGINE_URL=ws://127.0.0.1:49200   (rbac-proxy's port)
    III_TOKEN=<a token your auth function accepts>
  To run without RBAC anyway (local development only):
    III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC=true
```

In dangerous mode, `initialize.instructions` starts with, and `iii_status`
returns as `warning`: "WARNING: this MCP server runs WITHOUT RBAC
(III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC). Every MCP client can call every
function on this engine."

Credentials: iii-sdk sends `Authorization: Bearer $III_TOKEN` on the WebSocket
handshake; rbac-proxy's auth function receives it. No `mcp` code is involved.

An rbac-proxy with no `auth_function_id` refuses every upgrade, so in practice
a proxied `mcp` is always authenticated.

### Where the endpoint is reachable from

`mcp` serves through the http worker, whose default host is `0.0.0.0`. After
the RBAC check it reads `http::status`:

| http worker host | `III_MCP_DANGEROUSLY_ALLOW_PUBLIC_HTTP` | Outcome |
|---|---|---|
| loopback (`127.0.0.0/8`, `::1`, `localhost`) | any | serve |
| anything else (`0.0.0.0`, `::`, LAN, public, multicast) | off | **refuse**, with how to set the http worker's `host` to `127.0.0.1` |
| anything else | `true` / `1` | serve, with a warning |
| `http::status` unreadable | any | serve, with a warning |

Several MCP servers share one http worker on different `api_path`s; one
rbac-proxy per server gives each its own rules.

### Running under compose

Compose waits for a worker to connect under its exact name, but rbac-proxy
renames every proxied worker to `rbac-proxy:<session or prefix>:<name>`. So
compose would never see `mcp` connect, and would kill it when
`startup_timeout` ran out.

The fix is in the auth function: its result can list the names a connection
may keep, `worker_names: ["mcp"]`. rbac-proxy then forwards the declared name
unchanged (exact match, no patterns), compose sees `mcp`, and
`rbac-proxy::session` reports the grant. Compose itself is unchanged.

Grant a name only to the credential that runs that worker. Two connections
with the same plain name are the same worker to the engine: the newer one
replaces the older and takes over its suspended observers, as with direct
connections.

## HTTP and sessions

The worker observes `http::on-request` for `POST`, `GET` and `DELETE
<api_path>` (default `/mcp`). Each request is handled in this order:

1. **Origin.** No `Origin` header: allowed (CLI/SDK clients). Loopback origins
   (`localhost`, `*.localhost`, `127.0.0.1`, `[::1]`, any scheme and port):
   allowed. Exact matches in `allowed_origins`: allowed. Anything else,
   `null` and malformed origins included: **403**. This check is `mcp`'s own,
   as the MCP spec requires: the http worker's default CORS policy is
   permissive (any origin), and CORS would not stop a request from reaching
   the server anyway.
2. **Method.** `DELETE` with `Mcp-Session-Id` ends the session (**200**;
   **404** unknown; **400** without the header). `GET` with `Mcp-Session-Id`
   opens the session's SSE stream (see Observe; **404** unknown; **400**
   without the header). Anything else but `POST`: **405**.
3. **JSON-RPC shape.** Batches and non-JSON-RPC bodies: -32600 (HTTP 200).
4. **Session.** `initialize` opens a new session and returns its id in the
   `Mcp-Session-Id` response header, or gets **503** when `max_sessions`
   (default 16) are open. Any other request carrying an unknown
   or expired id gets **404** ("Session not found"), which tells the client to
   initialize again. Requests without the header are served; only the
   session-scoped tools need one.
5. **Dispatch.** Notifications: **202**, no body. Requests: one SSE `message`
   event with the JSON-RPC reply.

Refusals at steps 1, 2 and 4 carry a JSON-RPC error (`-32000`, id `null`) in
the body.

### Session lifecycle

| When | Effect |
|---|---|
| `initialize` | new session (UUID v4) |
| any request with its id | refreshes `last_seen` |
| idle longer than `session_idle_ttl_secs` (default 1800) | ended by the idle-session sweep: a tokio task in `mcp` that wakes every 60 s and ends every session idle past the TTL |
| `DELETE` | ended |
| worker shutdown | every session ended |
| ended | all its observers disposed on the engine; buffers dropped |

Sessions are in memory: a restart of `mcp` loses them, and clients get 404
and re-initialize.

## Tools

`tools/list` offers exactly these twelve, in this order.

| Tool | Input | Output | Session |
|---|---|---|---|
| `iii_call` | `function_id` (required), `payload` (object, default `{}`), `timeout_ms`, `void`, `max_chars` (default 100000) | `{ result }`; `{ result: <cut JSON text>, truncated: true, total_chars }` when larger than `max_chars`; `{ result: null, void: true }` for `void` | no |
| `iii_secret_store` | `name`, `value`, `consumers` (all required), `description` | `{ ref, hint, fingerprint, consumers }` | no |
| `iii_observe` | `function_id` (required), `filters`, `args`, `capture`, `namespace`, `max_observations`, `expires_in_ms`, `target_function_id` | `{ observer_id, function_id, state, delivery: "poll" \| "target", target_function_id? }` | yes |
| `iii_observations_sse` | as `iii_observe`, without `target_function_id` | `{ observer_id, function_id, state, delivery: "sse" }` | yes |
| `iii_observations_poll` | `observer_id` (optional: all poll observers), `max` (default 50) | `{ observations, remaining, dropped }` | yes |
| `iii_observers` | — | `[{ observer_id, function_id, delivery, target_function_id?, filters?, state, buffered, received, dropped }]` | yes |
| `iii_unobserve` | `observer_id` (required) | `{ observer_id, removed: true, discarded }` | yes |
| `iii_status` | — | `{ server, version, session_id, access_control: "rbac-proxy" \| "disabled", policy?, warning? }` | no |
| `iii_list_agents` | `include_hidden` | profiles from `directory::agents::list` | no |
| `iii_get_agent` | `id` (required) | `directory::agents::get` | no |
| `iii_list_skills` | `search`, `prefix` | rows from `directory::skills::list` | no |
| `iii_get_skill` | `id` (required) | the skill's markdown body | no |

Rules shared by all tools:

- Engine errors are **tool results** with `isError: true` and the engine's
  `code: message`. That is how the MCP spec (2025-06-18, *Tools › Error
  Handling*) reports tool execution errors, so the model reads and reacts to
  them; JSON-RPC errors are for protocol errors (unknown method, invalid
  params, a malformed request).
- Argument errors (missing `function_id`, non-object `payload`) are tool
  errors and make no engine call.
- `iii_call` refuses `mcp::*`, the worker's own internals. Nothing else is
  filtered locally: RBAC decides.
- A session tool without a session returns a tool error telling the client to
  initialize and send `Mcp-Session-Id`.
- An unknown tool name is a tool error ("unknown tool"); function-style
  names (`myworker__echo`) are **not** forwarded.

`initialize.instructions` tells the model to use `iii_call` instead of the
`iii` CLI and `iii_observe` instead of polling functions, that `agent_call` /
`harness::observe` in harness-written profiles mean `iii_call` /
`iii_observe`, and that the server cannot register functions: to add one,
the agent creates or changes a worker it controls and registers it with the
iii-sdk.

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
- **No value can be read back through the MCP server.** Every MCP client
  shares the server's one identity, so if one could resolve a secret, any
  agent on the server could read every secret. Two layers enforce it, neither
  in `mcp`: rbac-proxy's allow list keeps `secrets::resolve` out (the setup
  forbids it in the auth result, since `match("*")` and `match("secrets::*")`
  include it), and the `secrets` worker resolves only for the consumers a
  secret lists.
- `mcp::handler` receives every request body, so it is registered
  `observable: false`.
- `mcp` sets `III_DISABLE_TRACE_PAYLOADS=1` before connecting, as `secrets`
  does, so its spans carry no payloads.
- No tool result or error message contains the value. A missing `secrets`
  worker is reported with how to add it.

rbac-proxy's `expose_functions` must include `secrets::set`. The instructions
tell the model never to put a credential in an `iii_call` payload.

## Observe

Each matching invocation of an observed function becomes an **observation**.
An observer delivers its observations one of three ways:

| Created with | Delivery | Where observations go |
|---|---|---|
| `iii_observe` | poll | engine → `mcp::observed` → the session's buffer, read with `iii_observations_poll` |
| `iii_observations_sse` | sse | engine → `mcp::observed` → the session's `GET` stream, as `notifications/iii/observation` |
| `iii_observe` with `target_function_id` | target | the engine runs that function with each observation; nothing reaches the session |

Every observer belongs to its session whatever the delivery: `iii_observers`
lists it, `iii_unobserve` removes it, and ending the session disposes it.
`iii_unobserve` disposes the SDK observer handle, which sends the engine its
`unobserve` frame; no function is called.

- **Sink.** Poll and SSE observers target `mcp::observed`, one sink for the
  worker, which routes each observation to its session by `observer_id`.
  Observations for observers that are not live (unobserved, ended session,
  unknown id, a target observer) are dropped and never resurrected.
- **Who may deliver.** The sink is public (the engine refuses a private
  observer target). The engine stamps `_caller_worker_id` on every call a
  worker makes; its own deliveries carry no stamp. The sink admits an
  unstamped delivery, and a stamped one only from the worker hosting the
  observed function (a source that delivers its own observations), compared
  by worker name through `engine::workers::list` and
  `engine::functions::list`. Anything else is dropped with a warning.
- **Buffer.** Each session holds up to `max_buffered_observations` (default
  500) across its observers; beyond that the oldest are dropped and counted,
  per session (`iii_observations_poll` reports `dropped` once, then resets
  it) and per observer (`iii_observers`).
- **Poll.** `iii_observations_poll` without `observer_id` merges the
  session's poll observers in arrival order. An SSE or target observer's id
  is refused with where its observations go instead.
- **SSE.** `GET <api_path>` with `Mcp-Session-Id` keeps the response open:
  each SSE observation goes out as one SSE `message` event carrying the
  JSON-RPC notification and an `id`. The response is the request's
  `OutputStream` (call-native streams, iii-hq/iii#2561), kept open after the
  handler returns. Delivery is resumable, as the MCP Streamable HTTP spec
  allows: observations stay in the session after they are written (a write
  the socket took proves nothing about the client), so a `GET` with
  `Last-Event-ID: n` replays every SSE observation after `n`. A `GET`
  without it gets only those no stream wrote. A dropped connection therefore
  loses nothing; mcp notices it on the next write and stops, and the
  client's reconnect resumes. mcp writes nothing on a schedule: a quiet
  stream is ended by the http worker's idle timeout and the client
  reconnects. Observations the cap dropped before the client got them are
  announced first, as `notifications/iii/observations_dropped`
  `{ after_event_id, through_event_id }`. A new `GET` replaces the previous
  stream; ending the session ends it.
- **Caps.** At most `max_sessions` (default 16) sessions and
  `max_observers_per_session` (default 100) observers each; over the cap,
  `initialize` gets 503 and observe tools a tool error.
- **Activation.** Observe tools wait up to 5 s for activation. An observer of
  a function no one hosts yet stays `pending` and is returned as such. A
  refused observer (bad filter, unknown `capture`, RBAC `FORBIDDEN`) is
  disposed and reported as `observe refused: <code>: <message>`.
- `max_observations` maps to the observer's `max_fires`, `expires_in_ms` to
  its expiry; the engine enforces both.

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
| `api_path` | `/mcp` | routes; one per MCP server sharing an http worker |
| `allowed_origins` | `[]` | Origin check |
| `session_idle_ttl_secs` | `1800` | sessions |
| `max_sessions` | `16` | sessions |
| `max_observers_per_session` | `100` | observe |
| `max_buffered_observations` | `500` | observe, per session |

| Environment | |
|---|---|
| `--url`, then `III_MCP_ENGINE_URL`, then `III_URL` | engine URL: rbac-proxy's port in production. Compose sets `III_URL` to the engine itself, hence the dedicated variable. |
| `III_TOKEN` | bearer token for rbac-proxy's auth function |
| `III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC` | `true` or `1` to start without rbac-proxy |
| `III_MCP_DANGEROUSLY_ALLOW_PUBLIC_HTTP` | `true` or `1` to serve on an http worker not bound to loopback |

Keys from earlier versions (`tools`, `hidden_prefixes`, `require_expose`,
`max_buffered_events`) are ignored. The manifest declares `http`,
`rbac-proxy` and `secrets` as dependencies.

## Security model

| Threat | Mitigation | Residual |
|---|---|---|
| MCP clients calling anything on the engine | rbac-proxy: auth function + `expose_functions` + observer gating + discovery filtering on `mcp`'s connection | one shared identity for all MCP clients |
| `mcp` accidentally deployed straight to the engine | boot refusal via `rbac-proxy::session` | the escape hatch exists, and says so to clients |
| The endpoint reachable from other machines | boot refusal unless the http worker is on loopback | the escape hatch exists, and warns |
| A web page driving a localhost MCP server (DNS rebinding) | Origin check (MCP spec requirement) | non-browser local processes are trusted, as with any localhost service |
| Faking "behind the proxy" | probe answered only by the proxy, never forwarded; proxy refuses registering the id | anything that can register functions on the engine *directly* could register a fake; it guards misconfiguration, not an attacker with engine access |
| Forged observations injected into a session | sink admits only engine deliveries and the observed function's host (engine-stamped caller); only live observer ids route | the host of a function can deliver to observers of its own function, which it could fake by calling it anyway |
| Credentials in tool calls | `iii_secret_store`; handler not observable; no trace payloads; no value in results | the value crosses the `http` worker, whose own records may hold it, and the client's model has seen it |
| Reading secrets back | rbac-proxy allow list (`secrets::resolve` forbidden); `secrets` resolves only for listed consumers | a misconfigured allow list (`match("*")` without the forbid) lets clients call it; `secrets` still refuses unless `mcp` is a consumer |
| Resource exhaustion | caps on sessions and observers per session, per-session buffer, idle session TTL, engine-side `max_fires` / expiry | one client can still take every session slot |

## Compatibility

Breaking (hence 0.7.0):

- clients see the twelve gateway tools, not one tool per function; the
  per-function mode is gone.
- `mcp` will not start unless proxied (or the flag is set), nor serve on an
  http worker that is not on loopback (or the other flag is set).
- no prompts capability (the methods are -32601); resources come from
  `iii-directory`.
- the engine URL is read from `III_MCP_ENGINE_URL` before `III_URL`.
- it needs rbac-proxy with `expose_endpoints` (#1712): `/` to connect, `/otel`
  for telemetry.

The registry still shows 0.5.9's stale `skills` dependency; publishing 0.7.0
replaces it.

## Testing

- 122 unit tests in `mcp`, written first: the guard's decision table and
  messages (globs, info note), the bind guard, the sink caller guard, Origin
  parsing, the directory bridge, sessions (caps, poll/SSE/target delivery,
  the per-session buffer and drop counting, streams attached, waiting and
  replaced, unobserve, end, the idle sweep, close), the gateway tools and
  their descriptions (no tool says "event"), the HTTP handler (status codes,
  `GET` streams, 503 at the session cap, resources), `iii_secret_store`, the
  SSE frames, the SDK adapter's `capture` parsing, config, manifest, declared
  dependencies. The engine is a fake `Bus`.
- In `rbac-proxy`: the probe (answered before RBAC, the globs as written,
  void calls, refused registration), `worker_names`, `expose_endpoints` and
  `/otel`, and the allow-list scenarios for `secrets::resolve` (not listed;
  only `secrets::set`; `secrets::*`; `*` with and without the forbid).
- 26 BDD scenarios against a live engine, including `mcp::handler` listed
  as not observable and observe end to end (`state::set` filtered by scope,
  polled, session ended, then 404).
- Manually, on a compose project behind rbac-proxy (compose reports `mcp`
  ready): the tool list, `access_control` and the policy globs, a poll
  observer, an SSE observation received on a real `GET` stream through the
  http worker, a target observer running a function, `secrets::resolve`
  refused by RBAC, a delivery forged by another worker dropped, and both
  refusal paths.

## Open questions

1. **Per-client identity.** All MCP clients share `mcp`'s proxied identity.
   Passing each MCP session's credentials through would give per-user RBAC.
   A possible follow-up; for now, namespaces and several rbac-proxy + mcp
   pairs (each with its own token and rules) cover it without the extra
   complexity.
2. **Durable sessions.** Sessions, observers and buffers are in memory, so an
   `mcp` restart loses them. Decided: make them state-backed so observers
   survive a restart; not in this PR.
3. **Shorter idle TTL.** Whether `session_idle_ttl_secs` should default to
   less than 30 minutes now that sessions are capped.

Resolved since the first draft: telemetry (rbac-proxy relays `/otel` and any
endpoint listed in `expose_endpoints`), push delivery
(`iii_observations_sse`), and limits (16 sessions, 100 observers each).
