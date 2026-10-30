# rbac-proxy

A standalone iii worker that puts the engine's role-based access control **in
front of** the engine instead of inside it. It opens its own configurable
WebSocket port, speaks the iii worker protocol verbatim, and transparently
reverse-proxies every frame — functions **and** channels — to a trusted engine
listener, while at the boundary it:

- **authenticates** each connection (the operator's `auth_function_id`),
- **gates** every invocation in the namespace it is made in (vendored
  `is_function_allowed`: forbidden > scoped grant > allowed > infrastructure
  carve-out > `expose_functions` > deny), and the namespace a connection may
  declare,
- **gates observers**: an observer needs the session to be allowed to call
  both the function it observes and its target, and the frames a worker
  exchanges about the observers of its own functions pass for those functions
  only ([Observers](#observers)),
- **namespaces** a session's registrations behind `function_registration_prefix`,
- runs **middleware** and the two **registration hooks** (functions and
  observers), and
- **rewrites the results of the six `engine::*` discovery functions** so a
  caller only ever sees the surface it is allowed to invoke,
- **pins identities**: a proxied worker registers as `rbac-proxy:<declared
  name>`, and an `engine::*` call is forwarded only as a direct call (a queued
  one is refused), so a session never runs anything as a trusted worker, and
- **forwards only the frame types it knows, and only text frames from the
  client**: anything else, a binary frame included, is dropped, so a protocol
  addition stays closed until the proxy learns to gate it.

It is the [`console`](https://github.com/iii-hq/workers/tree/main/ade) reverse-proxy pattern (`src/proxy.rs`: one
outbound engine WebSocket per inbound connection) with an RBAC interceptor
spliced into the pump and a second proxied route for `/ws/channels/{id}`.

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
namespaces a connection may declare, and `allowed_functions` applies in
`default` only.

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

A call through the middleware waits up to 610 s for it (a middleware that
forwards with its own timeout should use 600 s or less), and a connection that
does not finish its upgrade headers within 10 s is closed.

Hot reload: a `host`/`port` change rebinds the public listener (last-good on
bind failure — the previous listener *and* config are kept); everything else
(`engine_url`, `rbac.*`, `middleware_function_id`, `expose_worker_internals`)
takes effect on the next connection.

## Observers

Workers behind the proxy observe functions and host observable functions as
they do against the engine. The engine runs observers for a proxied
connection with no RBAC session, so the proxy gates every observe frame:

- **Features.** `observe.v1` and `observe.v2` are kept in what the worker
  declares and what the engine answers. `local-dispatch.v1` is kept only when
  no `middleware_function_id` is set: a call that runs in-process never
  reaches the middleware. With middleware set, `functionregistered` reaches
  the worker with `local_dispatch: false`. `large-messages.v1` is removed (the
  proxy forwards text frames only).
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
  as the host sent it.
- **Hosting.** `functionregistered`, `observeradded`, `observerremoved`,
  `observerack`, `invocationsobserved` and `localinvocationstats` pass for the
  functions the connection registered, with the session prefix removed toward
  the worker and applied toward the engine. A call through an observer
  (`iii.call({ observer, payload })`) passes for an observer the engine
  announced to the connection as the host of its function; any other gets
  `NOT_FUNCTION_OWNER`. `invocationresult.observed` and `output_is_input`
  pass as sent, and the engine checks them.
- **Registrations.** `private`, `observable` and `args_format` on
  `registerfunction` pass to the engine. The connection's calls to its own
  private functions go straight to the engine: the middleware runs on the
  proxy's control connection, which a private function refuses.
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
| `rbac-proxy::status` | `{}` | `{ host, port, engine_url, rbac_enabled, active_connections, version }` |

`rbac-proxy::status` is a read-only health and identity probe: the bound
host and port, the upstream engine URL with any `user:pass@` redacted,
whether an auth function is configured, the live connection count and the
worker version.

The proxy hosts no observable event source. Two internal functions are the
targets of its own observers; both are never agent-callable (denied in
`iii-permissions.yaml`) and answer `{ ok: true }`:

| Function | Observes | Receives | Does |
|---|---|---|---|
| `rbac-proxy::on-config-change` | `configuration::on-updated`, filters `{ "input.id": { "$eq": <config id> }, "input.event_type": { "$eq": "configuration:updated" } }`, `deliver: output` | the `ConfigurationEvent` | re-reads the entry with `configuration::get` (the event itself is ignored) and applies it; the proxy also reloads once when the observer becomes active, for an update made during boot |
| `rbac-proxy::on-functions-changed` | `engine::on-functions-changed`, `capture: []` | the `Observed` envelope, without payloads | invalidates the discovery catalog cache (a 5 s TTL bounds staleness either way) |

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
be observed). The engine is `III_BIN` or the `iii` on `PATH`,
started from a temporary directory on its own port (49381; the proxy on
49382), never the default 49134. It **self-skips** when there is no `iii`
binary, a port is taken, or the proxy does not come up (e.g. the
`configuration` worker is not deployed).

`tests/observe_protocol.rs` runs iii's observe wire fixtures (vendored in
`tests/fixtures/protocol/observe/`) through the interceptor.
