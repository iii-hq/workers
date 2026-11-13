# mcp

Model Context Protocol (MCP) server for the [iii engine](https://iii.dev), as a
single Rust binary worker. It serves MCP 2025-06-18 over **Streamable HTTP**
(`POST`, `GET` and `DELETE` on `/mcp`, through the `http` worker) and gives
any MCP client (Claude Code, Codex, OpenCode, Cursor, ...) the whole engine
through twelve tools:

- **`iii_call`** calls any function by id;
- **`iii_secret_store`** stores a credential in the
  [`secrets`](../secrets) vault and returns a `secret://NAME` reference for
  the agent to pass on instead of the value;
- **`iii_observe`** observes a function: each matching invocation becomes an
  observation, buffered in the client's MCP session and read with
  **`iii_observations_poll`**, or handed to a target function the engine
  runs; **`iii_observations_sse`** streams observations on the session's
  `GET` stream instead;
- agent profiles and skills come from the `iii-directory` worker.

It runs **behind [`rbac-proxy`](../rbac-proxy)**, so RBAC decides what MCP
clients may call and observe, and refuses to start otherwise.

## Setup

Five steps. The first four are one-time; commands assume a compose project in
the `default` namespace.

### 1. Add the workers

```bash
iii call compose::add worker=mcp
```

`mcp` declares `http`, `rbac-proxy` and `secrets` as dependencies, so they
come with it.

The http worker listens on every interface (`0.0.0.0`) by default, and `mcp`
refuses to serve an endpoint other machines can reach. Bind it to loopback
(its configuration entry is `default-http`; `iii call http::configuration-id`
names it in other namespaces):

```bash
iii call configuration::get --json '{"id": "default-http"}'
# then set the same value with "host": "127.0.0.1"
iii call configuration::set --json '{"id": "default-http", "value": { ..., "host": "127.0.0.1" }}'
```
`mcp` will not start yet: it is not behind rbac-proxy until step 4.

### 2. Provide an auth function

rbac-proxy authenticates every connection by calling **your** auth function
(`rbac.auth_function_id`); with none configured it refuses everything. It
receives `{ headers, query_params, ip_address }` and returns the session's
grants, or throws to refuse. The smallest useful one checks a shared bearer
token. As a Node worker (`workers/mcp-auth/index.mjs`, `npm i iii-sdk`):

```js
import { timingSafeEqual } from 'node:crypto'
import { registerWorker } from 'iii-sdk'

const iii = registerWorker(process.env.III_URL, { workerName: 'mcp-auth' })
const expected = Buffer.from(process.env.MCP_TOKEN ?? '')

iii.registerFunction(
  'mcp-auth::authenticate',
  async ({ headers = {} }) => {
    const auth = Object.entries(headers).find(([k]) => k.toLowerCase() === 'authorization')?.[1]
    const token = Buffer.from(String(auth ?? '').replace(/^Bearer\s+/i, ''))
    if (expected.length < 32 || token.length !== expected.length || !timingSafeEqual(token, expected)) {
      throw new Error('invalid token')
    }
    return {
      allowed_functions: [],
      allow_function_registration: true,
      // mcp keeps its own name, so compose sees it start.
      worker_names: ['mcp'],
      // Agents store secrets; nothing on the MCP server may read one back.
      forbidden_functions: ['secrets::resolve'],
    }
  },
  // It sees bearer tokens: nothing may observe it.
  { observable: false },
)
```

Put a long random token in your project's `.env` (`openssl rand -hex 32`):

```bash
MCP_TOKEN=<token>
```

and add the worker to `worker-compose.yaml`:

```yaml
  mcp-auth:
    worker: path://./workers/mcp-auth
    env_file: [./.env]
    scripts:
      run: node index.mjs
```

### 3. Point rbac-proxy at it

rbac-proxy's settings live in the `configuration` worker. In a compose project
the entry is `<namespace>-rbac-proxy` (`default-rbac-proxy`):

```bash
iii call configuration::set --json '{
  "id": "default-rbac-proxy",
  "value": {
    "host": "127.0.0.1",
    "port": 49200,
    "engine_url": "ws://127.0.0.1:49134",
    "rbac": {
      "auth_function_id": "mcp-auth::authenticate",
      "expose_endpoints": ["match(\"/\")", "match(\"/ws/channels/*\")", "match(\"/otel\")"],
      "expose_functions": ["match(\"*\")"]
    }
  }
}'
```

rbac-proxy applies it live. Two lists decide what gets through:

- `expose_endpoints` is which of the engine's endpoints rbac-proxy serves.
  **It serves none until you list them.** `mcp` needs `/` to connect at
  all; `/otel` carries its logs and traces (without it the log shows
  `HTTP error: 403 Forbidden` and no telemetry is exported); keep
  `/ws/channels/*` for functions that stream.
- `expose_functions` is what MCP clients may reach:
  `match("*")` is everything; narrow it (`match("orders::*")`,
  `metadata: { public: true }`, ...) to limit agents. Keep `secrets::set`
  in it for `iii_secret_store`, and keep `secrets::resolve` out: it returns a
  secret's value, so any agent on the MCP server could read every secret.
  `match("*")` and `match("secrets::*")` both include it, which is why the
  auth function above forbids it.

See [rbac-proxy](https://iii.dev/registry/workers/rbac-proxy) for the rules,
scoped grants and hooks. Check it with `iii call rbac-proxy::status`:
`rbac_enabled: true` and the three endpoints in `expose_endpoints`.

### 4. Point mcp at rbac-proxy

In `worker-compose.yaml`, give `mcp` the proxy's address and the token:

```yaml
  mcp:
    worker: package://api.workers.iii.dev/mcp
    env_file: [./.env]
    environment:
      III_MCP_ENGINE_URL: ws://127.0.0.1:49200
```

and in `.env`, `III_TOKEN` is the token mcp sends (the same value as
`MCP_TOKEN`):

```bash
III_TOKEN=<token>
```

Then restart it: `iii call compose::restart container=mcp`.

Use `III_MCP_ENGINE_URL`, not `III_URL`: compose sets `III_URL` to the engine
itself for every worker.

### 5. Connect a client

The endpoint is the http worker's URL plus `/mcp`
(`iii call http::status` reports the base, by default
`http://127.0.0.1:3111`). For Claude Code:

```bash
claude mcp add --transport http iii http://127.0.0.1:3111/mcp
```

Ask the agent for `iii_status`: it reports `"access_control": "rbac-proxy"`
and the rules in force.

Several MCP servers can share one http worker: give each `mcp` its own
`api_path` (and its own rbac-proxy and token, to give each different rules).

### Local development without RBAC

On a machine you alone use, you can skip steps 2 to 4:

```yaml
  mcp:
    worker: package://api.workers.iii.dev/mcp
    environment:
      III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC: "true"
```

Every MCP client can then call every function on the engine; the server logs
a warning and tells every client in its `initialize` instructions and
`iii_status`.

## The RBAC check

Before registering anything, `mcp` calls `rbac-proxy::session`, which
rbac-proxy answers itself for every proxied connection and never forwards; a
connection straight to the engine gets `function_not_found`.

| Situation | Outcome |
|---|---|
| Behind rbac-proxy | Starts. If the proxy's `expose_functions` include `match("*")` **and** it has no auth function, it logs a warning. |
| Not behind rbac-proxy | Refuses to start (exit 1), with the message below. |
| Not behind rbac-proxy, `III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC=true` or `1` | Starts, warns in the log, `initialize` instructions and `iii_status`. |
| `III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC` set to anything else | Refuses, naming the accepted values. |
| Behind rbac-proxy with the flag set | Starts; logs at `info` that the flag is ignored. |
| The probe cannot be made (engine or proxy down, connection refused by auth, timeout) | Refuses to start. |

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

The check guards against a misconfigured URL, not an attacker: anything that
can register functions on the engine directly could register a fake
`rbac-proxy::session`. All MCP clients share the worker's one proxied
identity, so RBAC decides what "agents via MCP" may do as a whole.

Next, `mcp` reads `http::status`. If the http worker listens anywhere but
loopback (`0.0.0.0`, `::`, a LAN or public address), it refuses to start and
says how to bind it to `127.0.0.1`; `III_MCP_DANGEROUSLY_ALLOW_PUBLIC_HTTP=true`
(or `1`) serves anyway, with a warning. If `http::status` cannot be read, it
starts with a warning.

## Tools

| Tool | What it does |
|---|---|
| `iii_call` | Call a function by id with a JSON `payload` (default `{}`). `timeout_ms`; `void: true` to fire and forget; results over `max_chars` (default 100000) come back cut, with `truncated` and `total_chars`. `mcp::` functions (this worker's internals) are refused; everything else is RBAC's decision. |
| `iii_secret_store` | Store a credential: `name` (`secret://<name>`), `value`, `consumers` (worker names allowed to resolve it, at least one), `description`. Calls `secrets::set` and returns only `{ ref, hint, fingerprint, consumers }`. Storing an existing name rotates it. rbac-proxy must expose `secrets::set`. |
| `iii_observe` | Observe a function: `function_id`, `filters`, `args`, `capture`, `namespace`, `max_observations`, `expires_in_ms`, and optionally `target_function_id`. Without a target, observations are buffered for `iii_observations_poll`. With one, the engine runs that function with each observation and nothing comes back to the session. Returns `{ observer_id, function_id, state, delivery, target_function_id? }`. Needs a session. |
| `iii_observations_sse` | Same arguments without `target_function_id`: each observation is sent on the session's `GET` stream as a `notifications/iii/observation` message, and waits in the session while no stream is open. Needs a session. |
| `iii_observations_poll` | Read and remove buffered observations of poll observers, oldest first, for one `observer_id` or all of them; `max` (default 50). Returns `{ observations, remaining, dropped }`. Needs a session. |
| `iii_observers` | The session's observers with `delivery`, `state`, `buffered`, `received`, `dropped` (and `target_function_id`). Needs a session. |
| `iii_unobserve` | Remove one observer, discarding its unread observations. Needs a session. |
| `iii_status` | Server version, session id, and `access_control`: `rbac-proxy` with its `policy` (the exposed globs), or `disabled` with a warning. |
| `iii_list_agents`, `iii_get_agent` | Agent profiles from `directory::agents::*` (hidden ones with `include_hidden`). `iii_get_agent` without an `id` returns the `default` profile. |
| `iii_list_skills`, `iii_get_skill` | Skills from `directory::skills::*` (`search`, `prefix`). |

The `initialize` instructions tell the agent, in this order, to:

1. load the default profile with `iii_get_agent` (no arguments; it reads
   `directory::agents::get {"id": "default"}` from iii-directory) and follow
   it, reading its `agent_call` as `iii_call` and `harness::observe` as
   `iii_observe`;
2. discover the system with `iii_call`: `directory::search_functions` for
   candidate ids by capability, `engine::functions::list`,
   `engine::workers::list`, `directory::skills::index`, and
   `engine::functions::info` for a contract before the first call;
3. use higher-order functions, which call other functions for it: `fp::pipe`
   chains calls worker-side (large values never pass through the agent) and
   `fp::batch` runs independent calls at once.

They also say the server cannot register functions: to add one, the agent
creates or changes a worker it controls and registers it with the iii-sdk.

Engine errors come back as tool results with `isError: true` and the engine's
code, as the MCP spec prescribes for tool execution errors, so the model reads
them; JSON-RPC errors are kept for protocol errors. Unknown tool names are
tool errors too.

## Secrets

An agent that holds a credential stores it with `iii_secret_store` and passes
the `secret://NAME` reference to the function that needs it, which resolves
it through `secrets::resolve` if it is a listed consumer. A value is never
retrievable through the MCP server: otherwise any agent connected to it could
read every secret. rbac-proxy keeps `secrets::resolve` out (see setup step 2),
and the `secrets` worker only resolves for listed consumers. To keep the value
out of the engine's records:

- `mcp::handler`, which receives every request body, is registered
  `observable: false`, so no observer gets a copy;
- this worker turns off payload capture in its trace spans
  (`III_DISABLE_TRACE_PAYLOADS=1`);
- tool results and error messages never contain the value.

The value still crosses the `http` worker, and anything that records
request bodies there (its own traces, a proxy in front of it) can see it.
The agent's model has seen it too.

## Sessions and observe

`initialize` opens a session and returns its id in the `Mcp-Session-Id`
response header; clients send it back on every request. An unknown or expired
id gets **404**, which tells the client to initialize again. `DELETE /mcp`
with the header ends the session (200; 404 if unknown, 400 without the
header). Requests without the header work for every tool except the observe
ones.

An observer delivers in one of three ways:

- **poll** (`iii_observe`): the engine sends each observation to
  `mcp::observed`, one sink for the worker, which routes it to its session by
  `observer_id`; the client reads it with `iii_observations_poll`;
- **sse** (`iii_observations_sse`): through the same sink, then out on the
  session's `GET /mcp` stream, each with an SSE event id. They stay in the
  session after they are written, so a client that reconnects with
  `Last-Event-ID` gets every observation after that id: a dropped connection
  loses nothing. A `GET` without it gets only what no stream wrote yet.
  Nothing is written on a schedule: a quiet stream is ended by the http
  worker's idle timeout (`default_timeout`, 30 s), and the client reconnects.
  If the cap dropped observations the client never got, the replay starts
  with a `notifications/iii/observations_dropped` message naming the ids;
- **target** (`iii_observe` with `target_function_id`): the engine runs that
  function with each observation; nothing reaches the session.

Limits and cleanup:

- at most `max_sessions` sessions (default 16; `initialize` beyond it gets
  **503**) and `max_observers_per_session` observers (default 100);
- each session holds up to `max_buffered_observations` (default 500) across
  its observers; beyond that the oldest are dropped and counted;
- the idle-session sweep, a task in `mcp` that runs once a minute, ends every
  session idle for `session_idle_ttl_secs` (default 1800) and disposes its
  observers; the worker also disposes every observer on shutdown;
- an observer of a function nobody hosts yet stays `pending`; a refused one
  (bad filter, unknown capture, RBAC) is reported with the engine's code.

`mcp::observed` is public because the engine refuses a private observer
target; it is marked internal and hidden from traces. The engine stamps every
call a worker makes with the caller's worker id, and its own deliveries carry
no stamp, so the sink accepts a stamped delivery only from the worker that
hosts the observed function. Another worker calling it with a forged
`observer_id` is dropped.

## Origin

The MCP spec requires validating `Origin`, so a web page cannot drive a local
MCP server through the browser: no `Origin` header (CLI and SDK clients) and
loopback origins (`localhost`, `*.localhost`, `127.0.0.1`, `[::1]`, any port)
pass, exact entries in `allowed_origins` pass, anything else gets **403**.

## Resources

Skills are resources at `iii://<id>` (`resources/list`, `resources/read`,
`resources/templates/list`), read from `iii-directory`. The server offers no
prompts.

## Configuration

`config.yaml` (optional; a missing or malformed file falls back to the
defaults with a warning):

```yaml
api_path: "/mcp"
allowed_origins: []
session_idle_ttl_secs: 1800
max_sessions: 16
max_observers_per_session: 100
max_buffered_observations: 500
```

| Environment | Meaning |
|---|---|
| `III_MCP_ENGINE_URL` | rbac-proxy's WebSocket URL (wins over `III_URL`; `--url` wins over both) |
| `III_TOKEN` | Bearer token sent to rbac-proxy's auth function |
| `III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC` | `true` or `1` to start without rbac-proxy (local development only) |
| `III_MCP_DANGEROUSLY_ALLOW_PUBLIC_HTTP` | `true` or `1` to serve on an http worker that is not bound to loopback |

## Development

```bash
cargo test --lib                       # unit tests, no engine needed
III_ENGINE_WS_URL=ws://127.0.0.1:49134 \
III_ENGINE_HTTP_URL=http://127.0.0.1:3111/mcp \
  cargo test --test bdd                # end to end against a running engine
```

The BDD harness connects straight to the engine, so it runs in the mode
`III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC` would select.
