# mcp

Model Context Protocol (MCP) server for the [iii engine](https://iii.dev), as a
single Rust binary worker. It serves MCP 2025-06-18 over **Streamable HTTP**
(`POST` and `DELETE` on `/mcp`, through the `http` worker) and gives any MCP
client (Claude Code, Codex, OpenCode, Cursor, ...) the whole engine through
eleven tools:

- **`iii_call`** calls any function by id;
- **`iii_secret_store`** stores a credential in the
  [`secrets`](../secrets) vault and returns a `secret://NAME` reference for
  the agent to pass on instead of the value;
- **`iii_observe`** observes a function, buffering every matching invocation
  in the client's MCP session, read with **`iii_events`**;
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
    // worker_names: mcp keeps its own name, so compose sees it start.
    return { allowed_functions: [], allow_function_registration: true, worker_names: ['mcp'] }
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
  in it for `iii_secret_store`.

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

Ask the agent for `iii_status`: it reports `"rbac": "rbac-proxy"` and the
proxy's policy.

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
| Behind rbac-proxy | Starts. If the proxy exposes every function **and** has no auth function, it logs a warning. |
| Not behind rbac-proxy | Refuses to start (exit 1), with the message below. |
| Not behind rbac-proxy, `III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC=true` or `1` | Starts, warns in the log, `initialize` instructions and `iii_status`. |
| `III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC` set to anything else | Refuses, naming the accepted values. |
| Behind rbac-proxy with the flag set | Starts; the flag is ignored with a note. |
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

## Tools

| Tool | What it does |
|---|---|
| `iii_call` | Call a function by id with a JSON `payload` (default `{}`). `timeout_ms`; `void: true` to fire and forget; results over `max_chars` (default 100000) come back cut, with `truncated` and `total_chars`. `mcp::` functions (this worker's internals) and `secrets::resolve` are refused. |
| `iii_secret_store` | Store a credential: `name` (`secret://<name>`), `value`, `consumers` (worker names allowed to resolve it, at least one, never `mcp`), `description`. Calls `secrets::set` and returns only `{ ref, hint, fingerprint, consumers }`. Storing an existing name rotates it. rbac-proxy must expose `secrets::set`. |
| `iii_observe` | Observe a function: `function_id`, `filters`, `args`, `capture`, `namespace`, `max_events`, `expires_in_ms`. Returns `{ observer_id, function_id, state }`. Needs a session. |
| `iii_events` | Read and remove buffered events, oldest first, for one `observer_id` or all of the session's; `max` (default 50). Returns `{ events, remaining, dropped }`. Needs a session. |
| `iii_observers` | The session's observers with `state`, `buffered`, `received`, `dropped`. Needs a session. |
| `iii_unobserve` | Remove one observer, discarding its unread events. Needs a session. |
| `iii_status` | Server version, session id, and `rbac` (`rbac-proxy` with its policy, or `disabled` with a warning). |
| `iii_list_agents`, `iii_get_agent` | Agent profiles from `directory::agents::*` (hidden ones with `include_hidden`). |
| `iii_list_skills`, `iii_get_skill` | Skills from `directory::skills::*` (`search`, `prefix`). |

Engine errors come back as tool results with `isError: true` and the engine's
code, so the model reads them. Unknown tool names are tool errors too.

## Secrets

An agent that holds a credential stores it with `iii_secret_store` and passes
the `secret://NAME` reference to the function that needs it, which resolves
it through `secrets::resolve` if it is a listed consumer. To keep the value
out of the engine's records:

- `mcp::handler`, which receives every request body, is registered
  `observable: false`, so no observer gets a copy;
- this worker turns off payload capture in its trace spans
  (`III_DISABLE_TRACE_PAYLOADS=1`);
- tool results and error messages never contain the value, and `iii_call`
  refuses `secrets::resolve`, so a value never goes back to a client.

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

An MCP server cannot wake its client mid-turn, so observing is buffered:

- every observer targets `mcp::observed`, one sink for the worker, which
  routes each delivery to its session by `observer_id`; deliveries for
  observers that are not live are dropped;
- each observer keeps up to `max_buffered_events` (default 500); beyond that
  the oldest are dropped and counted;
- a session idle for `session_idle_ttl_secs` (default 1800) is ended and its
  observers disposed; the worker also disposes every observer on shutdown;
- an observer of a function nobody hosts yet stays `pending`; a refused one
  (bad filter, unknown capture, RBAC) is reported with the engine's code.

`mcp::observed` is public because the engine refuses a private observer
target; it is marked internal and hidden from traces.

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
max_buffered_events: 500
```

| Environment | Meaning |
|---|---|
| `III_MCP_ENGINE_URL` | rbac-proxy's WebSocket URL (wins over `III_URL`; `--url` wins over both) |
| `III_TOKEN` | Bearer token sent to rbac-proxy's auth function |
| `III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC` | `true` or `1` to start without rbac-proxy (local development only) |

## Development

```bash
cargo test --lib                       # unit tests, no engine needed
III_ENGINE_WS_URL=ws://127.0.0.1:49134 \
III_ENGINE_HTTP_URL=http://127.0.0.1:3111/mcp \
  cargo test --test bdd                # end to end against a running engine
```

The BDD harness connects straight to the engine, so it runs in the mode
`III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC` would select.
