# RBAC contract

The `rbac-proxy` worker enforces the **same** role-based access control as an
engine `iii-worker-manager` RBAC listener — the contract documented in
[`worker-rbac.mdx`](../../../iii/docs/0-11-0/how-to/worker-rbac.mdx) and
[`iii-worker-manager/README.md`](../../../iii/engine/src/workers/worker/README.md).
This file mirrors that how-to's structure (auth function, `expose_functions`,
middleware, registration hooks, channels, full config, types reference) and
states where the proxy implementation differs from the in-engine one.

The behavioural promise is: **a connection through `rbac-proxy` is gated exactly
as the same connection through an engine RBAC listener would be.** The decision
logic is vendored verbatim from the engine (`rbac_config.rs`), so there is no
second, drifting copy of the rules — only a second *home* for them.

## The proxy has no engine `Session`

This is the load-bearing fact that determines the whole design.

Inside the engine, RBAC state lives in an in-process `Session` struct
(`engine/src/workers/worker/rbac_session.rs`) that the engine threads into the
invocation path and into the two session-aware discovery functions
(`engine::functions::list` / `::info`) via an in-process `SessionHandler`
closure. A worker connected over WebSocket — which is all `rbac-proxy` is from
the engine's point of view — **never receives that object.** When the proxy
calls `engine::functions::list` on its control or data connection, the engine
filters against *the proxy's own* session (which is unrestricted on the trusted
internal listener), not the downstream caller's.

Two consequences:

1. The proxy must **derive and hold each downstream connection's boundaries
   itself** — run the auth function, keep the resulting
   allowed/forbidden/expose/prefix/context for the life of the connection.
2. The proxy must **enforce every decision and rewrite every response
   client-side.** It cannot delegate filtering to the engine for *any* of the
   six discovery functions — not even the ones that are session-aware
   in-process. This is why [engine-overrides.md](engine-overrides.md) exists.

The per-connection session the proxy holds mirrors the engine's `Session`
fields:

```rust
// Held by the proxy for the lifetime of each downstream connection.
struct ProxySession {
    session_id: Uuid,
    ip_address: String,
    namespaces: HashMap<String, Vec<String>>,     // scoped grants; keys = declarable namespaces
    allowed_functions: Vec<String>,               // apply in `default` only
    forbidden_functions: Vec<String>,
    allow_function_registration: bool,            // default true; event sources are functions
    context: Value,                               // forwarded to middleware + hooks
    function_registration_prefix: Option<String>, // private namespace
}
```

## Authentication

When `rbac.auth_function_id` is configured, the proxy invokes it **once per
WebSocket upgrade**, before it opens the upstream connection to the engine. The
function receives an `AuthInput` built from the upgrade request and returns an
`AuthResult`.

```mermaid
sequenceDiagram
  participant W as downstream worker
  participant P as rbac-proxy (upgrade handler)
  participant A as auth function (engine)
  W->>P: GET / Upgrade: websocket<br/>headers, ?query, src ip
  P->>P: AuthInput { headers, query_params, ip_address }
  P->>A: iii.call(auth_function_id, AuthInput)  [control connection]
  alt throws or empty result
    P-->>W: error frame + Close (never opens upstream)
  else AuthResult
    P->>P: ProxySession from AuthResult
    P->>P: open upstream WS, begin pumping
  end
```

- `AuthInput.headers` — HTTP headers from the upgrade (e.g. `authorization`).
- `AuthInput.query_params` — `Record<string, string[]>` (repeated keys
  preserved).
- `AuthInput.ip_address` — the connecting client's IP, as the proxy sees it. In
  a deployment where the proxy is itself behind a load balancer, this is the
  peer address; honour `X-Forwarded-For` in the auth function if needed.

If `auth_function_id` is **unset**, the connection is allowed with a permissive
default session (`allowed_functions: []`, `forbidden_functions: []`,
`allow_function_registration: true`, `context: {}`, no prefix) and `expose_functions` alone gates access — identical
to the engine default. A proxy facing an untrusted network **must** set
`auth_function_id`; see [Security](#security).

**Fail closed on a configured-but-unresolvable function.** "Unset" (permissive
default) is distinct from "set but the function can't be reached" (a typo, a
not-yet-registered or removed function, or an `iii.call` error). The latter is
treated as a failure, never as "allow":

- **Auth** unresolvable/errors → **reject the upgrade** (error frame + Close), as
  if it threw.
- **Middleware** unresolvable/errors → return an `InvocationResult{ error }` to
  the caller (**deny** the call); never silently forward to the target.
- **Registration hook** unresolvable/errors → **deny** the registration.

This is the only safe default for a security boundary: a broken policy function
denies, it does not open the door.

The auth function is a normal engine function the operator registers (from any
SDK). The TypeScript/Python/Rust examples in
[`worker-rbac.mdx` §2](../../../iii/docs/0-11-0/how-to/worker-rbac.mdx) apply
unchanged — they connect to the engine's **internal** listener, not to the proxy
port.

## `expose_functions`

`rbac.expose_functions` is a list of filters; a function is exposed if **any**
filter matches. An **empty** list exposes nothing *via the filter* — but
`allowed_functions` (rule 2) and the infrastructure carve-out (rule 3) still
admit their members, so an empty `expose_functions` is not "deny everything", it
is "expose only what the auth result explicitly allows plus the carve-out". Two
filter shapes, mixable in one list:

```yaml
rbac:
  expose_functions:
    - match("api::*")        # wildcard, anchored at both ends, * = any run of chars
    - match("*::public")
    - metadata:              # all keys must match (AND); filters are OR'd
        public: true
    - metadata:
        tier: free
        name: match("*public*")
    - match: "orders::*"     # map form, scoped to a namespace
      namespace: orders
```

A filter answers in the namespace it names, and in `default` when it names
none: `match("api::*")` does not expose `api::x` reached in `orders`. The map
form takes `match` (a raw pattern) or `metadata`, plus an optional
`namespace`; any other key is refused, so a typo cannot silently rescope a
rule.

The proxy vendors the engine matcher so the semantics are identical:

- **Wildcard** (`rbac_config.rs:46-82`) — `*` matches any run of characters,
  the pattern is anchored at both ends, case-sensitive. `*` alone matches
  everything.
- **Metadata** — exact JSON equality per key, or a nested `match("...")` for
  string values; a function with no metadata never matches a metadata filter.

Metadata matching requires the proxy to know a function's registered metadata.
It learns this from `engine::functions::list` (which carries `metadata` per
entry) over its control connection; see
[engine-overrides.md § Catalog & binding caches](engine-overrides.md#catalog--binding-caches).

## Access resolution order

Every `InvokeFunction` frame from a downstream connection walks the exact engine
decision flow (`rbac_config.rs`, `is_function_allowed_ref`), evaluated by the
proxy for `function_id` reached in the call's `namespace` (absent = `default`):

1. `function_id` in `forbidden_functions` → **deny** (in every namespace).
2. a grant in `namespaces[namespace]` covers it (exact id or wildcard) → **allow**.
3. `function_id` in `allowed_functions` and `namespace` is `default` → **allow**.
4. `function_id` in the infrastructure carve-out → **allow**.
5. an `expose_functions` filter scoped to `namespace` matches → **allow**.
6. otherwise → **deny**.

A function the session registered on this connection is its own only in the
namespace the connection declared; the same id elsewhere is somebody else's.
When `namespaces` is non-empty, its keys are also the namespaces the connection
may declare on `engine::workers::register`; declaring another one closes the
connection, as the engine does.

The **infrastructure carve-out** is vendored verbatim and is part of the
contract (additive-only within a major version):

```
engine::channels::create   engine::log::info    engine::baggage::get
engine::workers::register   engine::log::warn    engine::baggage::set
                            engine::log::error   engine::baggage::get_all
                            engine::log::debug
                            engine::log::trace
```

These ten ids keep connection setup, channel creation, logging, and context
propagation working regardless of the operator's `expose_functions`. The **six
discovery** functions (`engine::functions::list/info`, `engine::workers::list/info`,
`engine::observers::list/get`) are **not** in the carve-out — they are gated by `expose_functions` like any other
function, and when reachable their results are rewritten to the caller's
boundaries (see [engine-overrides.md](engine-overrides.md)).

On **deny**, the proxy does not forward the frame. It synthesizes the engine's
`FORBIDDEN` reply directly back to the downstream connection. SDKs key the
rejection off the `code`, not the message, so matching `code: "FORBIDDEN"` is
what makes the pending call reject as it would against a real engine; reproduce
the engine's message and remediation branch too for parity:

```jsonc
{
  "type": "invocationresult",
  "invocation_id": "<echoed, or a fabricated UUID for a void action — see below>",
  "function_id": "<denied id>",
  // engine message shape: "function '<id>' not allowed (<remediation>)"
  // remediation = "remove from rbac.forbidden_functions" when the id was explicitly
  //               forbidden (rule 1), else "add to rbac.expose_functions" (rule 5).
  "error": { "code": "FORBIDDEN", "message": "function '<id>' not allowed (add to rbac.expose_functions)" }
}
```

The engine **always** sends this reply on deny — even for a `void` action,
where it fabricates an `invocation_id` because the frame carried `null`
(`engine/mod.rs`). The proxy mirrors that: fabricate a UUID for a denied `void`
call rather than dropping it silently (the worker has no pending entry for the
fabricated id, so it is harmless, and it keeps the proxy byte-compatible with the
engine's deny path).

## Middleware

When `middleware_function_id` is configured, every **allowed, non-`engine::`**
invocation is routed to the middleware instead of being forwarded to the engine.
The middleware receives a `MiddlewareFunctionInput` and **its return value
becomes the `InvocationResult`** the downstream caller receives.

> **The middleware contract, stated once and precisely** (it reads ambiguously
> across the engine code and docs): the middleware function receives
> `MiddlewareFunctionInput`; whatever it returns is returned to the caller as the
> result. **If the middleware wants the target function to actually run, it must
> invoke it itself** (`iii.call({ function_id, payload })`). `engine::*` calls
> **bypass middleware entirely** (so connection setup, channel creation, and the
> discovery overrides are never wrapped). This matches the engine
> (`engine/mod.rs:940-981`).

```mermaid
sequenceDiagram
  participant W as downstream worker
  participant P as rbac-proxy
  participant M as middleware fn (engine)
  participant T as target fn (engine)
  W->>P: InvokeFunction{ fn=api::users::list, data }
  P->>P: allowed? yes. engine::? no. middleware set? yes.
  P->>M: iii.call(middleware_id, { function_id, payload, action, context, namespace })
  M->>T: iii.call(api::users::list, enrichedPayload)   %% middleware's choice
  T-->>M: result
  M-->>P: result (becomes the InvocationResult)
  P-->>W: InvocationResult{ invocation_id, result }
```

`MiddlewareFunctionInput`:

| Field | Type | Description |
|---|---|---|
| `function_id` | string | The function the worker wants to invoke. |
| `payload` | object | The payload the worker sent. |
| `action` | CallAction or omitted | Routing action (`enqueue`, `void`), if any. |
| `context` | object | `AuthResult.context` for this session (`{}` when no auth fn). |
| `namespace` | string | The namespace the call was made in (`default` when it named none). |

The middleware runs on the proxy's **control connection** (`iii.call`),
exactly as the engine runs it via `engine.call`: resolved in the caller's
namespace, and given the call's metadata sidecar. Keep middleware idempotent —
retries must not double-charge or double-log.

Two kinds of call never reach the middleware: a call through an observer
(the host delivering a call, as an activation does), and the connection's
call of its own private function (only its host may call it, and the
middleware runs on the control connection), which therefore may also run
in-process: its `FunctionRegistered` keeps `local_dispatch`. Every other
function's `FunctionRegistered` says `local_dispatch: false` while middleware
is configured, since a call that runs in-process never reaches it.

## Observer registration RBAC

Workers connecting through the proxy may observe functions:
`iii.observe(source, target)` creates an observer that runs `target` with
each call of `source`. That activation is **engine-internal** — it does not
pass back through the proxy's `InvokeFunction` gate — and an observer reads
every call of the function it observes (input, output, caller). So the proxy
gates an observer at registration, like a call of both functions.

After the optional `on_observer_registration` hook (which sees the
worker-supplied bare ids), the proxy:

1. **Resolves the engine ids** of the source and the target using the same
   own-vs-foreign [prefix resolution](protocol-interception.md#prefix-resolution)
   as `InvokeFunction`, each in the namespace it names — not the blind
   `{prefix}::{id}` prepend used for `RegisterFunction`.
2. **Refuses what no proxied session may observe:** an `engine::*` target (an
   observer's target runs with no session behind it), and as the source the
   infrastructure carve-out, the discovery functions, `engine::observers::*`,
   and the engine events whose calls list functions, workers or observers
   (`engine::on-functions-changed`, `engine::on-workers-changed`,
   `engine::on-observers-changed`): their calls carry what the
   [discovery overrides](engine-overrides.md) filter.
3. **Checks access to both functions:**
   - a function this session registered on this connection → **allow**
     (observing its own function, or targeting its own handler);
   - else run [access resolution](#access-resolution-order) on the resolved id
     → **allow** or **deny**.
4. On **deny**, reply `ObserveResult{ id, state: "error", error: { code:
   "FORBIDDEN", message } }` with the same remediation wording as an invoke
   deny (`"function '<id>' not allowed (add to rbac.expose_functions)"`
   unless rule 1 forbids it). On **allow**, forward `Observe` with both ids set
   to their engine ids.

The registration hook remains the place for additional policy (audit,
narrower allowlists, stamping args such as a tenant). The access check is
**always on** — it is not optional and does not require configuring
`on_observer_registration`.

Hosting observable functions is gated too: the frames a host exchanges about
the observers of its functions, and the calls it makes through them, pass
only for the functions the connection registered
([protocol-interception.md § Observers](protocol-interception.md#observers)).
Registering an event source is registering a function, so
`allow_function_registration` and the `on_function_registration` hook gate it;
the hook sees and may replace `private`, `observable` and `args_format`.

## Registration hooks

For fine-grained control, configure hook functions invoked **before** each
registration. Each receives the registration details plus `AuthResult.context`;
it returns a result object with the (possibly mapped) fields to allow, or throws
to deny. Omitted result fields keep the original value.

```yaml
rbac:
  auth_function_id: my-project::auth-function
  on_function_registration_function_id: my-project::on-function-reg
  on_observer_registration_function_id: my-project::on-observer-reg
  expose_functions:
    - match("api::*")
```

| Hook | Fires on | Input | Result (omitted ⇒ unchanged) |
|---|---|---|---|
| `on_function_registration_function_id` | `RegisterFunction` | `{ function_id, description?, metadata?, namespace, private, observable, args_format?, context }` | `{ function_id?, description?, metadata?, private?, observable?, args_format? }` |
| `on_observer_registration_function_id` | `Observe` | `{ observer_id, function_id, namespace, args, filters, target_function_id, target_namespace, metadata, context }` | `{ function_id?, target_function_id?, args?, filters?, metadata? }` |

`rbac` keys an earlier proxy read are ignored with one `warn` each at load,
naming what replaces them, so a stored entry that sets one still loads (the
engine's own list).

The TypeScript/Python/Rust hook examples in
[`worker-rbac.mdx` §6](../../../iii/docs/0-11-0/how-to/worker-rbac.mdx) port over
unchanged.

**Failure semantics** (matching the engine, `engine/mod.rs:663-867,1216-1355`):
a hook that throws or returns a non-object **denies** the registration. A
denied `RegisterFunction` is dropped silently, as the engine drops it (there
is no ack frame for function registration). A denied `Observe` is answered as
the engine answers it, `ObserveResult{ state: "error", error: { code:
"FORBIDDEN", message } }`. In an observer hook's answer, `args: null` means no
args (`{}`), and `filters: null` or `metadata: null` removes them.

The interaction with `function_registration_prefix` and the exact wire rewrites
live in
[protocol-interception.md § Registration frames](protocol-interception.md#registration-frames).

## Function registration prefix

When `AuthResult.function_registration_prefix` is set, the session gets a private
namespace. Because the proxy — not the engine's internal listener — is the RBAC
boundary here, **the proxy owns the prefix**:

- On a `RegisterFunction` from the worker, the proxy rewrites
  `id` → `{prefix}::{id}` before forwarding to the engine. On an `Observe`, it
  resolves the source and the target with the same own-vs-foreign rules as
  `InvokeFunction` (see [Observer registration RBAC](#observer-registration-rbac)),
  not the blind prepend used for `RegisterFunction`.
- When the engine dispatches an `InvokeFunction` **back down** to the worker
  (an observer's activation, or another caller invoked the worker's function), the proxy
  strips `{prefix}::` from `function_id` before forwarding, so the worker SDK
  finds its locally-registered (bare) handler. The worker never sees the prefix.

This is the engine's transparent-namespace behaviour
(`engine/mod.rs:817-823` apply, `worker_connections/traits.rs` strip), relocated
to the proxy. The matching/resolution rules — which id `expose_functions` runs
against, and how the proxy resolves a worker invoking its *own* prefixed
function — are specified in
[protocol-interception.md § Prefix resolution](protocol-interception.md#prefix-resolution),
and surfaced in discovery results in
[engine-overrides.md § Prefix in results](engine-overrides.md#prefix-in-results).

## Channels

Channels work through the proxy port exactly as on the engine. The proxy mounts
`/ws/channels/{channel_id}` on its own RBAC port and bridges frames to the
engine's `/ws/channels/{channel_id}`. `engine::channels::create` is in the
infrastructure carve-out, so a worker can create channels even with empty
`expose_functions`. A `StreamChannelRef` is `{ channel_id, access_key,
direction }` and carries **no host**, so no ref rewriting is needed — the SDK
builds the channel URL from the address the worker connected to (the proxy
port), and the proxy forwards to the engine. Full mechanics:
[protocol-interception.md § Channel bridge](protocol-interception.md#channel-bridge).

## Full example config

The proxy's config lives in the `configuration` worker under id `rbac-proxy`
(not a `config.yaml`). Shown here as the equivalent YAML an operator would
`configuration::set` or seed:

```yaml title="configuration: rbac-proxy"
host: 0.0.0.0
port: 49200                       # the public RBAC port (req #2)
engine_url: ws://127.0.0.1:49134  # the trusted internal engine listener
expose_worker_internals: false    # strip pid/ip/metrics from engine::workers::* results
middleware_function_id: my-project::middleware-function
rbac:
  auth_function_id: my-project::auth-function
  on_function_registration_function_id: my-project::on-function-reg
  on_observer_registration_function_id: my-project::on-observer-reg
  expose_functions:
    - match("api::*")
    - match("*::public")
    - metadata:
        public: true
```

See [rbac-proxy.md § Configuration](rbac-proxy.md#configuration) for the schema,
defaults, and hot-reload behaviour (a `port`/`host`/`engine_url` change rebinds
the listener; everything else takes effect on the next connection).

## Types reference

All types are wire-compatible with the engine's RBAC types
(`sdk/.../helpers/src/worker_connection_manager.rs`); the SDKs already export
them.

### AuthInput

| Field | Type | Description |
|---|---|---|
| `headers` | `Record<string, string>` | HTTP headers from the upgrade request. |
| `query_params` | `Record<string, string[]>` | Query params; repeated keys preserved. |
| `ip_address` | `string` | Connecting client IP as the proxy sees it. |

### AuthResult

| Field | Type | Default | Description |
|---|---|---|---|
| `namespaces` | `Record<string, string[]>` | `{}` | Grants scoped to a namespace (`{ "orders": ["svc::*"] }`); the keys are the namespaces the connection may declare. Empty: declare any. |
| `allowed_functions` | `string[]` | `[]` | Allow beyond `expose_functions`, in `default` only. |
| `forbidden_functions` | `string[]` | `[]` | Deny even if exposed. Highest precedence. |
| `allow_function_registration` | `boolean` | `true` | May register new functions, event sources included. |
| `function_registration_prefix` | `string` or omitted | omitted | Private namespace prefix. |
| `context` | `object` | `{}` | Forwarded to middleware + hooks. |

A result that sets a field an earlier proxy read still parses; the field is
ignored with one `warn` per process (the engine's `REMOVED_FIELDS`).

### MiddlewareFunctionInput

| Field | Type | Description |
|---|---|---|
| `function_id` | `string` | Function being invoked. |
| `payload` | `object` | Caller payload. |
| `action` | CallAction or omitted | `enqueue` / `void`, if any. |
| `context` | `object` | Session auth context. |
| `namespace` | `string` | The caller's namespace. |

### Registration hook I/O

`OnFunctionRegistrationInput` `{ function_id, description?, metadata?, namespace, private, observable, args_format?, context }`
→ `OnFunctionRegistrationResult` `{ function_id?, description?, metadata?, private?, observable?, args_format? }`.

`OnObserverRegistrationInput` `{ observer_id, function_id, namespace, args, filters, target_function_id, target_namespace, metadata, context }`
→ `OnObserverRegistrationResult` `{ function_id?, target_function_id?, args?, filters?, metadata? }`.

All result fields are optional; omitting one keeps the original value. Throwing
denies the registration.

## Intentional divergences from the engine

The proxy aims for parity with a `worker-gateway` listener, but a few behaviours
are deliberately different. They are collected here so the parity claim elsewhere
in this spec is not read as absolute. Each is optional — a deployment that wants
strict parity can turn it off.

| Divergence | Engine behaviour | Proxy behaviour | Why |
|---|---|---|---|
| **Self-invoke under a prefix** | A prefixed worker invoking its own bare-named `foo` gets `NOT_FOUND` — the engine never re-prefixes invocation targets ([protocol-interception.md § Prefix resolution](protocol-interception.md#prefix-resolution)). | The proxy resolves the bare id to the session's own `{prefix}::foo` so the call succeeds. | A worker should be able to call what it registered. Opt-out: disable own-id re-prefixing for strict parity. |
| **Denied-discovery error code** | `engine::functions::info` distinguishes `FORBIDDEN` (denied) from `NOT_FOUND` (missing), leaking existence. | Defaults to engine parity (`FORBIDDEN`); a deployment may opt into collapsing denied → `NOT_FOUND` to also hide existence ([engine-overrides.md](engine-overrides.md)). | Hardening for hostile multi-tenant discovery. |
| **Observer targets and sources** | The engine checks a session's access to an observer's function and target. | The proxy runs the same checks and also refuses `engine::*` targets and the sources whose calls carry what discovery filters ([Observer registration RBAC](#observer-registration-rbac)). | A proxied connection has no engine session: an `engine::*` target would run unrestricted, and those sources would deliver the unfiltered discovery view. |
| **Call-chain claims** | The engine keeps the `caller` an SDK sets on a call or a report. | The proxy keeps it only when it names a call the engine routed to that connection and it has not answered, and writes it from that call ([protocol-interception.md § Downstream](protocol-interception.md#downstream--engine-frames-the-worker-sends)). | Observers filter on `caller`; a worker must not place its calls in another worker's call chain. |

Everything else — the access-resolution order, the carve-out, `AuthResult`
semantics, the middleware contract, the prefix apply/strip on `RegisterFunction`
and inbound dispatch — is byte-for-byte the engine's, by vendoring its decision
code.

## Security

- Only the proxy's RBAC port should face untrusted networks. The engine's
  internal listener (`engine_url`) and any other engine ports stay internal,
  reachable only by the proxy and trusted co-located workers. Enforce with
  firewall rules / network policy.
- Always set `auth_function_id` on a proxy facing an untrusted network. With no
  auth function, `expose_functions` alone gates access and every connection is
  admitted.
- Prefer narrow `expose_functions` patterns over `match("*")`. Audit the list
  when a new engine namespace appears.
- `forbidden_functions` is the hard-deny: per-user/per-role denylists the
  operator's `expose_functions` cannot override.
- The middleware is the place for request validation, rate limiting, and audit
  logging.
- `Observe` always verifies the observed function and the target against the
  session's access boundaries (see [Observer registration RBAC](#observer-registration-rbac)).
  Do not rely on the optional registration hook alone to block observers of
  privileged functions.
- Every function is observable unless registered `observable: false`. The auth
  function receives the upgrade's headers (bearer tokens included), and the
  middleware and hooks receive payloads and session context: register them
  with `observable: false`.
- Functions and observers registered through a proxied connection are scoped to
  that engine connection and cleaned up when the downstream worker disconnects —
  the proxy closes the upstream WS on downstream close, and the engine's existing
  per-connection cleanup does the rest.
