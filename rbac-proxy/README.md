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
- **gates trigger bindings** to functions the session may actually invoke,
- **namespaces** a session's registrations behind `function_registration_prefix`,
- runs **middleware** and the three **registration hooks**, and
- **rewrites the results of the eight `engine::*` discovery functions** so a
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
iii trigger compose::add worker=rbac-proxy
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
  on_trigger_registration_function_id: my-project::on-trigger-reg
  on_trigger_type_registration_function_id: my-project::on-trigger-type-reg
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
`default` only. `on_observer_registration_function_id` is accepted (so an
engine `rbac` block copies over unchanged) but not called yet; see below.

A call through the middleware waits up to 610 s for it (a middleware that
forwards with its own timeout should use 600 s or less), and a connection that
does not finish its upgrade headers within 10 s is closed.

Hot reload: a `host`/`port` change rebinds the public listener (last-good on
bind failure — the previous listener *and* config are kept); everything else
(`engine_url`, `rbac.*`, `middleware_function_id`, `expose_worker_internals`)
takes effect on the next connection.

## iii 0.25 observe

The proxy does not support observers yet, and the engine behind it gates none
of them for a proxied connection, so it keeps them off:

- it removes `observe.v1` / `local-dispatch.v1` from what the worker declares
  and what the engine answers, so a 0.25 SDK speaks the 0.24 protocol
  (`observe()` fails with `UNSUPPORTED`, nothing runs in-process);
- it refuses `observe` (`UNSUPPORTED`) and owner deliveries (`activation`),
  drops the other observe messages, removes the SDK's `caller` /
  `observe_depth` claims from calls and the `observe` descriptor from
  registrations (keeping only `allow: false`), and refuses
  `engine::observers::*`.

Workers behind the proxy keep `registerTrigger`, which the 0.25 engine still
serves. Details: [`protocol-interception.md § iii 0.25 observe`](https://github.com/iii-hq/workers/blob/main/tech-specs/2026-06-rbac-proxy-worker/protocol-interception.md#iii-025-observe).

> **Security.** Only the proxy's RBAC port should face untrusted networks.
> Set `auth_function_id`: with none, every upgrade is refused. The stored
> default has none, so a proxy that lost its configuration stays closed.

## Functions

- `rbac-proxy::status` — read-only health/identity probe (bound host/port, the
  credential-redacted engine URL, whether RBAC is enabled, live connection
  count, version).
- `rbac-proxy::on-config-change`, `rbac-proxy::on-functions-available` —
  internal trigger handlers (config hot reload and catalog-cache invalidation);
  never agent-callable (denied in `iii-permissions.yaml`).

## Local development & testing

```bash
cargo test                  # engine-free unit tests + golden schema + manifest
UPDATE_GOLDENS=1 cargo test  # regenerate tests/golden/ after an intentional surface change
```

`tests/integration.rs` boots a real `iii` engine + the proxy and drives a
downstream worker through the proxy port (exposed call, forbidden call,
forbidden trigger binding, channel round-trip, filtered discovery, a call in
another namespace, `observe`). The engine is `III_BIN` or the `iii` on `PATH`,
started from a temporary directory on its own port (49381; the proxy on
49382), never the default 49134. It **self-skips** when there is no `iii`
binary, a port is taken, or the proxy does not come up (e.g. the
`configuration` worker is not deployed).

`tests/observe_protocol.rs` runs iii 0.25's observe wire fixtures (vendored in
`tests/fixtures/protocol/observe/`) through the interceptor.
