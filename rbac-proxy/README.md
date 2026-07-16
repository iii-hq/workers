# rbac-proxy

A standalone iii worker that puts the engine's role-based access control **in
front of** the engine instead of inside it. It opens its own configurable
WebSocket port, speaks the iii worker protocol verbatim, and transparently
reverse-proxies every frame — functions **and** channels — to a trusted engine
listener, while at the boundary it:

- **authenticates** each connection (the operator's `auth_function_id`),
- **gates** every invocation (vendored `is_function_allowed`: forbidden >
  allowed > infrastructure carve-out > `expose_functions` > deny),
- **gates trigger bindings** to functions the session may actually invoke,
- **namespaces** a session's registrations behind `function_registration_prefix`,
- runs **middleware** and the three **registration hooks**, and
- **rewrites the results of the eight `engine::*` discovery functions** so a
  caller only ever sees the surface it is allowed to invoke.

It is the [`console`](https://github.com/iii-hq/workers/tree/main/console) reverse-proxy pattern (`src/proxy.rs`: one
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
iii worker add rbac-proxy
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
```

Hot reload: a `host`/`port` change rebinds the public listener (last-good on
bind failure — the previous listener *and* config are kept); everything else
(`engine_url`, `rbac.*`, `middleware_function_id`, `expose_worker_internals`)
takes effect on the next connection.

> **Security.** Only the proxy's RBAC port should face untrusted networks.
> Always set `auth_function_id` on a public proxy — with no auth function,
> `expose_functions` alone gates access and every connection is admitted.

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
forbidden trigger binding, channel round-trip, filtered discovery). It
**self-skips** when the `iii` binary is absent or the proxy does not come up
(e.g. the `configuration` worker is not deployed).
