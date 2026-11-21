# rbac-proxy

A standalone iii worker that puts the engine's role-based access control **in
front of** the engine instead of inside it. It opens its own configurable
WebSocket port, speaks the iii worker protocol verbatim, and transparently
reverse-proxies every frame — functions **and** channels — to a trusted engine
listener, while at the boundary it:

- **serves only the engine endpoints you list** in `rbac.expose_endpoints`
  (`/`, `/ws/channels/*`, `/otel`; `/` alone without the key, nothing with an
  empty list), and no other path at all,
- **authenticates** each worker (`/`) and telemetry (`/otel`) connection
  (the operator's `auth_function_id`, or, on a local project without one, the
  built-in [`loopback_auth`](#local-projects-loopback_auth)); channel sockets
  (`/ws/channels/*`) are relayed to the engine, which checks their access
  keys,
- **gates** every invocation in the namespace it is made in (vendored
  `is_function_allowed`: forbidden > scoped grant > allowed > infrastructure
  carve-out > `expose_functions` > deny), and the namespace a connection may
  declare,
- **gates observers**: an observer needs the session to be allowed to call
  both the function it observes and its target, and the frames a worker
  exchanges about the observers of its own functions pass for those functions
  only ([Observers](#observers)),
- **namespaces** a session's registrations behind `function_registration_prefix`,
  or pins them to one engine namespace (`registration_namespace`),
- runs **middleware** and the two **registration hooks** (functions and
  observers), and
- **rewrites the results of the six `engine::*` discovery functions** so a
  caller only ever sees the surface it is allowed to invoke,
- **pins identities**: a proxied worker registers as
  `rbac-proxy:<prefix>::<declared name>` (`rbac-proxy:<namespace>::<declared
  name>` for a session pinned to a namespace, `rbac-proxy:<session id>:<declared
  name>` for a session with neither), unless the
  auth function (or `loopback_auth`) granted that name in `worker_names` (one live session at a
  time, never an `rbac-proxy:` name), and an
  `engine::*` call is
  forwarded only as a direct call (a queued one is refused), so a session
  never runs anything as a trusted worker or as another prefix's worker, and
- **forwards only the frame types it knows, and only text frames from the
  client**: anything else, a binary frame included, is dropped, so a protocol
  addition stays closed until the proxy learns to gate it.

It is the [`ade`](https://github.com/iii-hq/workers/tree/main/ade) reverse-proxy pattern (`src/proxy.rs`: one
outbound engine WebSocket per inbound connection) with an RBAC interceptor
spliced into the pump, plus relays for `/ws/channels/{id}` and `/otel`.

The RBAC decision logic is **vendored verbatim** from the engine
(`engine/src/workers/worker/rbac_config.rs`), so a connection through
`rbac-proxy` is gated byte-for-byte as the same connection through an engine
`iii-worker-manager` RBAC listener would be — the proxy is just a different,
out-of-process *home* for the same contract (it can front a remote or managed
engine, scale and harden independently, and shrink an auth bug's blast radius
to the proxy alone).

Full design: [`tech-specs/2026-06-rbac-proxy-worker/`](https://github.com/iii-hq/workers/blob/main/tech-specs/2026-06-rbac-proxy-worker/README.md).

## Install

```bash
iii call compose::add worker=rbac-proxy
```

Requires a trusted engine listener with **no** `rbac` block (the proxy is the
public door; the engine port stays internal) and the `configuration` worker
(required boot dependency).

## Run

```bash
rbac-proxy --url ws://127.0.0.1:49134
```

- `--url` — the engine WebSocket URL for the proxy's control connection, and
  the default upstream for the data plane.
- `--config <file.yaml>` — optional one-time seed installed as the
  `configuration` entry's `initial_value` when none exists yet.
- `--manifest` — print the publish manifest as JSON and exit.

## Before you start: expose the endpoints

rbac-proxy serves the engine endpoints listed in `rbac.expose_endpoints`.
Without the key it serves the worker endpoint `/` only, as earlier versions
did (minus channels), and logs a warning; with an empty list it serves
nothing. The warning repeats at startup and on every config change.

Most setups want all three of the engine's endpoints:

```yaml
rbac:
  expose_endpoints:
    - match("/")              # workers: the iii protocol, gated by RBAC
    - match("/ws/channels/*") # channels (streams between functions)
    - match("/otel")          # telemetry: worker logs, traces and metrics
```

| Endpoint | What it carries | Leave it out and |
|---|---|---|
| `/` | The worker protocol. Each connection is authenticated by `auth_function_id`, or, when that is unset, admitted by `rbac.loopback_auth` if it comes from this machine; then every frame is checked against the RBAC rules. | No worker can connect. |
| `/ws/channels/{id}` | Channel sockets. Relayed as is; the engine checks each channel's access key, as it does for direct connections. | Functions that stream through channels fail. |
| `/otel` | Telemetry from the SDK. Authenticated the same way as `/` before the upgrade, then relayed as is (see below). | Workers log `Failed to connect to engine WebSocket` and their telemetry is not exported. |

Rules use the `expose_functions` wildcard syntax over the request path:
`match("/otel")` or `{ match: "/otel" }`, where `*` matches any run of
characters and the pattern must match the whole path. A bare `"/otel"`
is an error that tells you to write `match("/otel")`. Only these three
endpoints are ever served: any other path (`/ws/channels/a/b`, `/otel/v2`,
`/admin`) is a `404`, whatever the rules match, so an endpoint the engine
adds later stays closed until the proxy learns to handle it. A refused
request gets a bare `404 not found`; the rule that would expose it is logged
for the operator, not returned. `rbac-proxy::status` lists the rules in force.

**Telemetry is not attributed.** The engine names relayed spans, logs and
metrics by the `service.name` inside them, and through the proxy every sender
has the proxy's address. So a client accepted on `/otel` can write telemetry
under any service name, another worker's or another tenant's included, and
send messages up to tungstenite's limits (64 MiB per message, 16 MiB per
frame). Expose `/otel` only to clients you trust with that: the auth
function is told the endpoint (`endpoint: "/otel"`, see below) and can refuse
it per credential.

**Upgrading from an earlier version:** earlier versions always served `/`
and `/ws/channels/*` and never `/otel`, and refuse the `expose_endpoints`
key, so set it after upgrading. Until you do, the new version serves `/`, so
workers stay connected; channels need `match("/ws/channels/*")`.

## Configuration

Config lives in the `configuration` worker under id `rbac-proxy` (no committed
`config.yaml`; `WorkerConfig::default()` seeds first boot, with `engine_url`
defaulting to `--url`). Equivalent YAML an operator would `configuration::set`:

```yaml
host: 0.0.0.0
port: 49200                       # the public RBAC port
engine_url: ws://127.0.0.1:49134  # the trusted internal engine listener
expose_worker_internals: false    # strip pid/ip/metrics from engine::workers::* results
middleware_function_id: my-project::middleware-function
rbac:
  auth_function_id: my-project::auth-function
  # loopback_auth: {}        # a local project without an auth function: admits
  #                          # this machine only (see Local projects: loopback_auth)
  expose_endpoints:          # nothing is served without these (see above)
    - match("/")
    - match("/ws/channels/*")
    - match("/otel")
  on_function_registration_function_id: my-project::on-function-reg
  on_observer_registration_function_id: my-project::on-observer-reg
  expose_functions:
    - match("api::*")        # wildcard, anchored both ends, * = any run of chars
    - match("*::public")
    - metadata:              # all keys AND'd; filters OR'd
        public: true
    - match: "orders::*"     # scoped to a namespace (the others answer in `default`)
      namespace: orders
```

Namespaces work as in the engine's RBAC: a rule answers in the namespace it
names, `default` when it names none. The auth function may return scoped
grants (`namespaces: { orders: ["svc::*"] }`); their keys are then the only
namespaces a connection with no `registration_namespace` may declare, and
`allowed_functions` applies in `default` only.

The auth function may return `registration_namespace: "laptop"`: the one
namespace the session registers in. The proxy declares it to the engine and
waits for the engine's answer before it forwards the session's first frame
(no answer within 10 s, or a refusal, closes the session with 1011), closes a
session that declares another one
(`NAMESPACE_FORBIDDEN`, grant keys included), names its workers
`rbac-proxy:<namespace>::<name>`, and refuses to register the middleware,
auth and hook ids there. It is never `default`, never the proxy's own
namespace and never set together with `function_registration_prefix`: such a
result fails the authentication. A call or an observe that names no namespace
is in `default` for a pinned session too, never in its pin.

The auth function receives `{ headers, query_params, ip_address, endpoint,
registration_namespace: true }`, `endpoint` being the path being opened
(`"/"` or `"/otel"`), so it can grant an endpoint per credential;
`registration_namespace: true` says this proxy pins a session to the
namespace its result names.

The auth function may also return `worker_names: ["mcp-server"]`, which rbac-proxy
adds on top of the engine's auth result. A worker that declares one of these
names keeps it, instead of getting `rbac-proxy:<...>:<name>`. Names must match
exactly; patterns are not supported, and names starting with `rbac-proxy:` are
ignored (those are the names the proxy gives sessions). Grant a name when
something waits for the worker by name, such as compose readiness. One live
session at a time holds a granted name; another session declaring it
meanwhile gets its own `rbac-proxy:<...>:<name>`. A worker connected to the
engine directly under the same name would share it, so only grant names no
trusted worker uses, to credentials you trust to be that worker:

```js
// my-project::auth-function
if (headers.authorization === `Bearer ${process.env.MCP_TOKEN}`)
  // allowed_functions matches exact ids; a pattern goes in namespaces.
  return { namespaces: { default: ["*"] }, worker_names: ["mcp-server"] }
```

The registration hooks are the engine's, with the engine's input and answer:

| Hook | Input | An object answer may replace |
|---|---|---|
| `on_function_registration_function_id` | `{ function_id, description, metadata, namespace, private, observable, args_format?, context }` | `function_id`, `description`, `metadata`, `private`, `observable`, `args_format` |
| `on_observer_registration_function_id` | `{ observer_id, function_id, namespace, args, filters, target_function_id, target_namespace, metadata, context }` | `function_id`, `target_function_id`, `args`, `filters`, `metadata` |

A hook that throws or answers anything but an object denies: a denied
function registration is dropped, as the engine drops it, and a denied
observer is refused with `FORBIDDEN`. Hooks see bare ids (before the session
prefix). `rbac` keys of entries stored before 0.26 are ignored with one
`warn` each, naming the setting to use, so such an entry loads.

### Local projects: `loopback_auth`

Without `auth_function_id` every worker (`/`) and telemetry (`/otel`)
connection is refused. A local project that
wants rbac-proxy (for example in front of `mcp-server`) without writing an
auth function or minting a per-project secret can set `rbac.loopback_auth`
instead: a connection whose peer address is loopback gets these grants,
exactly as if an auth function had returned them; a connection from any other
address is refused, as without the key. Loopback means exactly `127.0.0.1`,
`::1` or `::ffff:127.0.0.1`, not the rest of `127.0.0.0/8`: a service-mesh
sidecar such as Istio's connects from `127.0.0.6` on behalf of remote
clients.

```yaml
rbac:
  loopback_auth:
    worker_names: [mcp-server]
    forbidden_functions: [secrets::resolve]
    allow_function_registration: true
    function_registration_prefix: mcp
  expose_endpoints:
    - match("/")
  expose_functions:
    - match("api::*")
```

It takes the fields an auth function may return but
`registration_namespace`, with the same defaults except one: `namespaces`, `allowed_functions`, `forbidden_functions`,
`allow_function_registration`, `context` (default `{}`),
`function_registration_prefix` and `worker_names`.
`allow_function_registration` defaults to `false` here (an auth function's
result defaults it to `true`), and setting it to `true` requires a non-empty
`function_registration_prefix`, so what local processes register stays apart
from everything else: `allow_function_registration: true` without a prefix is
a configuration error, refused by `configuration::set` and failing the load of
a stored value. Every other rule applies
unchanged: `expose_functions`, the registration hooks, the middleware, the
`worker_names` rules above (one live session per granted name, never an
`rbac-proxy:` name) and `forbidden_functions` winning over everything.
Unknown keys are refused, so a typo cannot drop a restriction.

Two more keys refuse the grant to a loopback peer whose upgrade shows signs of
a non-local client; such an upgrade is refused exactly like one from a
non-loopback address (same response, same log). Both apply to `/` and `/otel`:

- `allowed_origins` (default `[]`): an upgrade that carries an `Origin`
  header, which a browser page adds, gets the grant only when the value is in
  this list (exact match, scheme and host compared case-insensitively, for
  example `http://localhost:3000`). Every `Origin` header must match, and one
  whose value is not text is refused. An upgrade without `Origin`, which is what
  SDK workers send, is unaffected. With the default every `Origin` is refused,
  so a web page open in a local browser cannot borrow the grant.
- `allow_forwarded` (default `false`): an upgrade carrying any of
  `Forwarded`, `X-Forwarded-For`, `X-Forwarded-Host`, `X-Forwarded-Proto`,
  `X-Real-IP` or `Via` came through a proxy and is refused unless this is
  `true`, whatever the header's value (even one that is not text).

These checks catch the well-behaved cases; a forwarder that adds none of these
headers is not detected (see the warnings below).

- **It applies only without `auth_function_id`.** When both are set the auth
  function authenticates every worker and telemetry connection,
  `loopback_auth` is ignored, and a
  warning naming both keys is logged each time such a configuration is
  applied.
- **Only use it where every local process is trusted.** On such a machine any
  local process can already reach the engine port directly, so admitting
  loopback connections gives away nothing a shared token would protect. Do
  not use it on a shared host.
- **Anything that forwards to a local port makes its clients look local.** A
  tunnel, an ingress or nginx forwarding to rbac-proxy connects from
  `127.0.0.1`, so every client behind it gets the loopback grants. Front such
  a setup with an `auth_function_id`.

In the iii ecosystem, for example (these are examples, not an exhaustive
list):

- **`sandbox-libkrun` VMs with networking.** A VM reaches host loopback ports
  through its gateway, and `iii-network` rewrites the gateway address to
  `127.0.0.1`, so code running in a sandbox (for example through
  `sandbox-code-runner`) is a loopback peer.
- **`tailscale::serve::add` (and Funnel)** publishes a local port to the
  tailnet (or, with Funnel, to the internet).
- **`quick-tunnel`** publishes a local port through a cloudflared Quick
  Tunnel.
- **`kubectl port-forward` and service-mesh sidecars** deliver remote
  clients from a local address.

The `Origin` and forwarding-header checks above catch some of these, not all:
a forwarder that adds no such header, or a sandbox that connects directly, is
indistinguishable from a local process. "Every local process is trusted" stops
holding as soon as any such path is in play; use an `auth_function_id` then.

`rbac-proxy::status` reports which authentication is in force as `auth`:
`"function"`, `"loopback"` or `"none"` (every worker and telemetry
connection refused), next to `rbac_enabled` (still: an `auth_function_id` is
set). The mode governs `/` and `/otel` only: in every mode, `/ws/channels/*`
sockets go to the engine without the proxy authenticating them, and the engine
checks each channel's access key, so `none` does not refuse channel sockets. The session probe a
client sends through the proxy (`rbac-proxy::session`, used by mcp-server's
`iii_status`) reports the same `auth` in its `policy`. Like the other
`rbac.*` keys, a change applies to new connections.

A call through the middleware waits up to 610 s for it (a middleware that
forwards with its own timeout should use 600 s or less), and a connection that
does not finish its upgrade headers within 10 s is closed.

Hot reload: a `host`/`port` change rebinds the public listener (last-good on
bind failure — the previous listener *and* config are kept); everything else
(`engine_url`, `rbac.*`, `middleware_function_id`, `expose_worker_internals`)
takes effect on the next connection. One exception: a reload that makes the
proxy call a new middleware, auth or hook id closes the open sessions pinned
to a namespace (close 1012), which may hold that id there; they reconnect
under the new ids.

## Observers

Workers behind the proxy observe functions and host observable functions as
they do against the engine. The engine runs observers for a proxied
connection with no RBAC session, so the proxy gates every observe frame:

- **Features.** `observe.v1`, `observe.v2` and `local-dispatch.v1` are kept
  in what the worker declares and what the engine answers. With
  `middleware_function_id` set, `functionregistered` reaches the worker with
  `local_dispatch: false` for every function except the ones the connection
  registered `private: true`: a call that runs in-process never reaches the
  middleware, and the connection's own calls of a private function skip it
  (see **Registrations**). `large-messages.v1` is removed (the proxy forwards
  text frames only).
- **Observing.** An `observe` runs `on_observer_registration_function_id`,
  then needs the session to be allowed to call both the observed function and
  the target, each in the namespace it names. The session's own
  registrations pass and resolve to their prefixed ids. Refused with
  `FORBIDDEN`: an `engine::*` target (an observer's target runs with no
  session behind it), and as the observed function the infrastructure
  carve-out, the discovery functions, `engine::observers::*` and the engine
  events whose calls list functions, workers or observers
  (`engine::on-functions-changed`, `engine::on-workers-changed`,
  `engine::on-observers-changed`). The `Observed` envelope iii delivers for
  an observer the connection created names the observed function without the
  prefix; a call through that observer reaches the target with the payload
  as the host sent it. A refusal of an `observe` under an id whose earlier
  `observe` waits for the engine's answer goes out right after that answer,
  so the worker hears the answers in the order it sent the observes. Each
  `observeresult` answers the oldest unanswered `observe` under its id, and a
  refusal is that `observe`'s final answer. An observer is the connection's
  once the engine accepts it, unless the connection's `unobserve` of the id
  followed its `observe`. A refusal also sends the engine an `unobserve` of
  the id, unless the worker sent one after that `observe` or observes the id
  again: a `Reattach` gives the connection the observers of the worker's
  previous connection with no RBAC check, and the replay of one the session
  may no longer have is refused here (or, rewritten otherwise by the
  registration hook, by the engine).
- **Hosting.** `functionregistered`, `observeradded`, `observerremoved`,
  `observerack`, `invocationsobserved` and `localinvocationstats` pass for the
  functions the connection registered, with the session prefix removed toward
  the worker and applied toward the engine. A call through an observer
  (`iii.call({ observer, payload })`) passes for an observer the engine
  announced to the connection as the host of its function; any other gets
  `NOT_FUNCTION_OWNER`. `invocationresult.matched` and `delivered`, and
  `output_is_input`, pass as sent, and the engine checks them.
- **Registrations.** `private`, `observable` and `args_format` on
  `registerfunction` pass to the engine. The connection's calls of its own
  private functions skip the middleware, which runs on the proxy's control
  connection, a connection a private function refuses: they run in-process
  once `functionregistered` offers `local_dispatch`, and go straight to the
  engine otherwise.
- **Back on a new connection.** An `observe` that names the id of a
  suspended observer replaces it when it comes from a connection under the
  same worker name in the same namespace (the observe v2 rule for a worker
  that restarts). The worker name carries the session prefix or pin, so a
  restarted worker under the same prefix or pin takes its observers back (a
  queue consumer keeps its backlog), and an `observe` of the id from a session
  under another prefix or pin gets `OBSERVER_ID_CONFLICT` until the grace
  period ends. A session with neither declares
  `rbac-proxy:<session id>:<declared name>`, with the id the proxy gives each connection: nothing else tells two such
  sessions apart, so a session declaring another's name neither shuts it out
  (one live worker per name) nor takes its suspended observers. Its own
  reconnect keeps them by `Reattach`; an `observe` under its id from a new
  session is another worker's.
- **Call chains.** A `caller` a worker sets on a call or a report is kept only
  when it names a call the engine routed to that connection and the
  connection has not answered yet; the proxy writes it from that call, so a
  worker cannot place its calls in another worker's call chain.
- **Discovery.** `engine::observers::list` and `engine::observers::get` show
  the observers the connection created and the ones whose observed function
  and target it may call, with the prefix removed. `engine::observers::dispose`
  disposes only observers the connection created.

Frame types the iii protocol does not have, including the binding messages
of SDKs before 0.26, are dropped with one `warn` per connection and frame
type, naming the replacement.

Every function is observable unless it is registered `observable: false`.
The auth function receives the upgrade's headers, bearer tokens included, and
the middleware and the hooks receive payloads and session context: register
them with `observable: false`. Details:
[`protocol-interception.md § Observers`](https://github.com/iii-hq/workers/blob/main/tech-specs/2026-06-rbac-proxy-worker/protocol-interception.md#observers).

> **Security.** Only the proxy's RBAC port should face untrusted networks.
> Set `auth_function_id`: with none, every upgrade is refused. The stored
> default has none, so a proxy that lost its configuration stays closed.

## Functions

| Function | Input | Output |
|---|---|---|
| `rbac-proxy::status` | `{}` | `{ host, port, engine_url, rbac_enabled, auth, expose_endpoints, active_connections, version }` |
| `rbac-proxy::session` | `{}` | `{ proxied: true, session_id, function_registration_prefix, worker_names, policy: { auth_function, auth, expose_functions, expose_endpoints } }` |

`rbac-proxy::status` is a read-only health and identity probe: the bound
host and port, the upstream engine URL with any `user:pass@` redacted,
whether an auth function is configured, which authentication is in force
for worker and telemetry connections (`auth`: `"function"`, `"loopback"` or
`"none"`; channel sockets are checked by the engine in every mode), the live
connection count and the worker version.

`rbac-proxy::session` answers "is **this** connection behind rbac-proxy?",
while `rbac-proxy::status` reports on the proxy as a whole. The proxy's
interceptor answers the session probe for every proxied connection and never
forwards it, so a worker connected straight to the engine gets
`function_not_found`. No expose rule applies to it (it is not a function on
the engine), but `forbidden_functions` does: an auth function that lists
`rbac-proxy::session` denies it with `FORBIDDEN`. `policy` is the
configuration the session runs under: whether an auth function is
configured, which authentication admitted it (`auth`: `"function"` or
`"loopback"`), the `expose_functions` rules as written (they apply to every
session, in every namespace) and the endpoint rules in force. A client reads
the globs to decide what to warn its users about, for example a `match("*")`
without an auth function.

A worker that must only run behind the proxy (the `mcp` worker does) calls
it at boot and refuses to start on `function_not_found`. This guards against
a misconfigured URL, not against an attacker: anything that can register
functions on the engine directly could register a fake `rbac-proxy::session`
there. The proxy refuses to register that id for the connections it serves.

The proxy hosts no observable event source. Two internal functions are the
targets of its own observers; both are never agent-callable (denied in
`iii-permissions.yaml`) and answer `{ ok: true }`:

| Function | Observes | Receives | Does |
|---|---|---|---|
| `rbac-proxy::on-config-change` | `configuration::on-updated`, filters `{ "input.id": { "$eq": <config id> }, "input.event_type": { "$eq": "configuration:updated" } }`, `capture: [output]` | the `Observed` envelope, the `ConfigurationEvent` at `output` | re-reads the entry with `configuration::get` (the event itself is ignored) and applies it; the proxy also reloads once when the observer becomes active, for an update made during boot |
| `rbac-proxy::on-functions-changed` | `engine::on-functions-changed`, `capture: []` | the `Observed` envelope, without payloads | invalidates the discovery catalog cache (a 5 s TTL bounds staleness either way) |

`rbac-proxy::on-config-change` also answers `registration_namespace: true`,
so a caller that needs the pin can refuse a proxy whose answer lacks it.

The config id is `rbac-proxy`, or `III_CONFIG_NAME` when set.

## Local development & testing

```bash
cargo test                  # engine-free unit tests + golden schema + manifest
UPDATE_GOLDENS=1 cargo test  # regenerate tests/golden/ after an intentional surface change
```

`tests/integration.rs` boots a real `iii` engine + the proxy and drives a
downstream worker through the proxy port (exposed call, forbidden call, an
observer with a forbidden target, channel round-trip, filtered discovery, a
call in another namespace, an accepted observer, an engine event that cannot
be observed; for a pinned session the grace window, a re-declaration, the
middleware id and an SDK worker). The engine is `III_BIN` or the `iii` on `PATH`,
started from a temporary directory with a private `HOME`, on free ports picked
at boot, never the default 49134. It **self-skips** when there is no `iii`
binary or the proxy does not come up (e.g. the
`configuration` worker is not deployed).

`tests/observe_protocol.rs` runs iii's observe wire fixtures (vendored in
`tests/fixtures/protocol/observe/`) through the interceptor.
