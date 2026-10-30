# http

Exposes registered functions as HTTP endpoints. A function becomes a route
when it observes `http::on-request` with the route as args: external clients
then call it with a plain HTTP request, which is what you want for webhooks,
browser clients, or any caller outside the engine's WebSocket protocol.
Routing, path params, per-route and global middleware, CORS, and chunked
streaming responses are handled by this worker; the function receives an
`HttpRequest` and returns the response.

## Install

```bash
iii call compose::add worker=http
```

`iii call compose::add` resolves the worker and its dependencies, writes
exact declarations to `worker-compose.yaml`, and reconciles the Compose project.

## Quickstart

Register a function and observe `http::on-request` with it, passing the route
(`method` and `path`) as args:

```typescript
iii.registerFunction('orders::get', async (req: HttpRequest) => ({
  status_code: 200,
  body: { order_id: req.path_params.id },
}))

const route = iii.observe(
  { function_id: 'http::on-request', args: { method: 'GET', path: '/orders/:id' } },
  'orders::get',
)
await route.ready()   // rejected with HTTP_ROUTE_CONFLICT if the shape is taken
// later: await route.dispose() removes the route
```

The same in Rust:

```rust
use iii_http::types::HttpRequest;
use iii_sdk::{errors::Error, register_worker, InitOptions, RegisterFunction, Source};
use serde_json::{json, Value};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    iii.register_function(
        "orders::get",
        RegisterFunction::new_async(|req: HttpRequest| async move {
            Ok::<Value, Error>(json!({
                "status_code": 200,
                "body": { "order_id": req.path_params.get("id") },
            }))
        }),
    );

    iii.observe(
        Source::function("http::on-request").args(json!({ "method": "GET", "path": "/orders/:id" })),
        "orders::get",
    );

    tokio::signal::ctrl_c().await?;
    Ok(())
}
```

```bash
curl http://localhost:3111/orders/42
# {"order_id":"42"}
```

The function's return value becomes the response: `status_code` (default
200), `headers` (an object, or a list of `"Name: value"` lines), and `body`.
Path segments prefixed with `:` (e.g. `:id`) land in `req.path_params`; the
query string and headers arrive as `req.query_params` / `req.headers`.

## Functions

| Function | Args or input | What it is |
|---|---|---|
| `http::on-request` | args `{ method, path, public_webhook?, middleware_function_ids? }` | One call per HTTP request the server receives. Each observer serves one route; its target receives the `HttpRequest`. |
| `http::status` | none | Where the server listens: `{ host, port, url, last_reload_error }`. |
| `http::webhook-listener::status` | none | Internal: the restricted listener this process bound, and the last reload error. |
| `http::configuration-id` | none | Internal: this instance's configuration entry id. |
| `http::on-config-change` | the `configuration::on-updated` event | Internal: reloads the configuration. |

### `http::on-request`

| Arg | Required | Default | Description |
|---|---|---|---|
| `method` | yes | — | HTTP method to match (case insensitive). |
| `path` | yes | — | Route path, e.g. `/orders/:id` (or `/orders/{id}`). `:name` segments become `path_params`. |
| `public_webhook` | no | `false` | Also expose this route on the restricted listener. |
| `middleware_function_ids` | no | `[]` | Per-route middleware, run in list order after any global middleware and before the target. |

Unknown args are refused with `INVALID_ARGS`. Two observers cannot claim the
same method and path shape (`/a/:x` and `/a/:y` are one shape): the later one
is rejected with `HTTP_ROUTE_CONFLICT`.

A route lasts while its observer's worker is connected. When that worker's
connection goes away, the engine withdraws its observers from the http worker:
their routes answer `404`, and another observer can claim the shape. A worker
that restarts under the same worker name and observes again under the same
observer id within the engine's `observers.observer_grace_ms` (30 s by
default) replaces its earlier observer and gets the route back; for any other
worker that id is taken (`OBSERVER_ID_CONFLICT`) until the grace period ends:

```typescript
const route = iii.observe(
  { function_id: 'http::on-request', args: { method: 'GET', path: '/orders/:id' } },
  'orders::get',
  { id: 'orders-get-route' },   // stable across restarts of this worker
)
```

The server calls `http::on-request` once for every request it receives (its
CORS layer answers `OPTIONS` requests). The call's input is the request as the
server received it:

```jsonc
{
  "method": "GET",
  "path": "/orders/42",              // the request's path
  "query_params": { "expand": "items" },
  "headers": { "content-type": "application/json" },
  "body": null                       // the body parsed as JSON, else null
}
```

The worker delivers each call itself:

1. a path with no route gets `404`, and a path whose routes are on other
   methods gets `405` with `Allow`;
2. a body over 16 MiB gets `413`;
3. the observer's `filters` select requests by `input` (the request above);
   a request they reject gets `422 CONDITION_NOT_MET`;
4. the global middleware runs, then the route's middleware; a middleware
   that responds ends the request with its response;
5. the target runs with the `HttpRequest`, and its return value (or its
   writes to the response channel) is the response.

Steps 1 to 4 answer the call without calling the target. The middleware and
the target are called from the call's invocation, so a trace shows each
request as one call of `http::on-request` with them under it. What the
route's target receives:

```jsonc
{
  "method": "GET",
  "path": "/orders/:id",             // the matched route's template
  "path_params": { "id": "42" },
  "query_params": { "expand": "items" },
  "headers": { "content-type": "application/json" },
  "body": null,                      // the body parsed as JSON, else null
  "request_body": { /* channel ref: the exact request bytes */ },
  "response": { /* channel ref: write a streamed response here */ }
}
```

The target always receives the `HttpRequest`, whatever the observer's
`deliver` option says, and the observer's `metadata` arrives as the target's
metadata sidecar:

```typescript
iii.observe(
  {
    function_id: 'http::on-request',
    args: { method: 'POST', path: '/webhooks/stripe', public_webhook: true },
    filters: { 'input.headers.stripe-signature': { $exists: true } },
  },
  'billing::on-stripe-event',
)
```

`http::on-request` is private: only this worker calls it, and it cannot be an
observer's target. It has one host per namespace: a second `http` worker in
the same namespace cannot register it.

### Streaming responses

A target can stream its response: write to `req.response` (a
`StreamChannelRef`) with a `ChannelWriter` to send `set_status` /
`set_headers` control frames and body chunks for a chunked HTTP response, and
return `null`. Returning a non-null value instead yields a regular buffered
response built from `{ status_code, headers, body }`.

## Configuration

| Field | Default | Description |
|---|---|---|
| `port` | `3111` | TCP port the HTTP server binds to. `0` binds an OS-assigned ephemeral port. |
| `host` | `0.0.0.0` | Host/interface to bind. |
| `webhook_listener` | `null` (disabled) | Optional restricted listener; only explicitly opted-in routes. |
| `webhook_listener.host` | `127.0.0.1` | Restricted listener interface; does not inherit `host`. |
| `webhook_listener.port` | `3112` | Restricted listener port; separate from the normal/admin listener on 3111. |
| `default_timeout` | `30000` (ms) | Per-request timeout; on expiry the server returns `504`. |
| `cors.allowed_origins` | `[]` (permissive) | Allowed CORS origins. An empty list allows any origin. |
| `cors.allowed_methods` | `[]` (permissive) | Allowed CORS methods. An empty list allows any method. |
| `concurrency_request_limit` | `1024` | Maximum in-flight requests; requests over the limit wait for a slot. |
| `middleware[]` | `[]` | Global middleware, each `{ function_id, phase, priority }`, run on every route in ascending `priority` order before the route's own middleware. |

Configuration is owned by the `configuration` worker — edit it from the
console (**Configuration → Workers → http**) or seed it once via
`--config <file>.yaml` on first boot. The worker observes
`configuration::on-updated` for its entry. `middleware` and `default_timeout`
hot-reload without a restart, as do `cors` and `concurrency_request_limit`.
Changing `host`/`port` or `webhook_listener` binds the new addresses before
stopping the old listeners. All required binds must succeed; otherwise both
old listeners and their live configuration remain unchanged. Set
`webhook_listener: null` to disable it live. Shutdown drains both listeners
and serializes with reload; a late reload cannot restart a stopped server.

`http::status` reports where the server actually listens: `{ host, port,
url, last_reload_error }`, where `url` is the base URL to open from this
machine (`http://127.0.0.1:<port>` when it listens on every interface). It
is read-only and open to agents, so code and agents build page URLs from it
instead of assuming port 3111.

### Restricted webhook listener

Enable the optional listener in the existing `http` configuration:

```yaml
host: 127.0.0.1
port: 3111
webhook_listener:
  host: 127.0.0.1
  port: 3112
```

The internal `http::webhook-listener::status` function distinguishes saved
configuration from a listener that this process actually opened. Its `applied`
field contains the host from the applied configuration and the actual port from
the bound socket (including an OS-assigned port when configured with `0`), or
`null` when the listener is off. The
`last_reload_error` field reports the most recent failed reload and is cleared
by the next successful one. The function waits for an in-progress reload
before responding and exposes no other HTTP configuration.

Opt a route in with `public_webhook` in its observer's args:

```json
{"method":"POST","path":"/webhooks/github","public_webhook":true}
```

Only that opted-in route is eligible on 3112; it remains available on 3111.
Do not expose 3111 through a public proxy/tunnel. The restricted listener
mounts no Console, engine, or admin endpoints. Its private paths return 404
(including OPTIONS); methods not opted in for an eligible path return 405
with only public methods in `Allow`. The listener admits an OPTIONS request
before CORS; every other request is a call of `http::on-request`, whose
deliver hook matches it against the public routes only. Matching performs
no URL decoding, repeated/trailing-slash normalization, HEAD-to-GET aliasing,
or method-override handling. Parameter routes are still supported: opting
one in deliberately exposes every value matched by its parameter segments,
never a more specific private handler. Disposing a route's observer, or
observing it again without `public_webhook`, applies to subsequent requests.

Both listeners share the route table, configuration and layered router;
global and per-route middleware, timeouts, concurrency and the 16 MiB body
limit apply to both. A bind collision, including using the normal address
for the restricted listener, fails safely rather than combining them.

**Signature verification:** read `HttpRequest.request_body` using the SDK
`ChannelReader`; it contains the exact received bytes (UTF-8, JSON whitespace,
newlines and binary data included). Never compute HMAC from `body`, which is
only a parsed JSON convenience projection. Header names follow HTTP's
case-insensitive/lowercase representation; ASCII values such as
`x-hub-signature-256` and `x-github-delivery` pass through unchanged. The
header map is not a raw HTTP-header archive: duplicate headers and non-text
values are not represented byte-for-byte. This listener does not validate
signatures itself; the opted-in handler must authenticate deliveries.

### Local tests

`tests/webhook_listener.rs` and `tests/on_request.rs` use the shared harness's
loopback fake engine, which plays the observer lifecycle of
`http::on-request`, and real SDK binary channels, without any external service
or skipped tests:

```bash
CARGO_TARGET_DIR=target-http-local cargo test --locked --test webhook_listener --test on_request
cargo fmt --all -- --check
CARGO_TARGET_DIR=target-http-local cargo clippy --all-targets --all-features --locked -- -D warnings
```

The `e2e_*` suites need a running engine. They connect to
`III_ENGINE_WS_URL` (default `ws://127.0.0.1:49134`): point it at a disposable
engine, or at a closed port to skip them.
