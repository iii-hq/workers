# http

Exposes registered functions as HTTP endpoints. Any function bound to an
`http` trigger becomes a route — external clients call it with a plain HTTP
request instead of going through the iii SDK, which is what you want for
webhooks, browser clients, or any caller outside the engine's WebSocket
protocol. Routing, path params, per-route/global middleware, conditional
execution, CORS, and chunked streaming responses are all handled by this
worker; the function just receives an `HttpRequest` and returns a value.

## Install

```bash
iii worker add http
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker the next time it boots.

## Quickstart

Register a function and bind it to this worker's trigger type (`http` — see
[Trigger type](#trigger-type)) with `api_path` and `http_method`:

```rust
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{InitOptions, RegisterFunction, errors::Error, register_worker};
use iii_http::types::HttpRequest;
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

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "orders::get".into(),
        config: json!({ "api_path": "/orders/:id", "http_method": "GET" }),
        metadata: None,
    })?;

    tokio::signal::ctrl_c().await?;
    Ok(())
}
```

```bash
curl http://localhost:3111/orders/42
# {"order_id":"42"}
```

The function's return value becomes the response: `status_code` (default
200), `headers`, and `body`. Path segments prefixed with `:` (e.g. `:id`)
land in `req.path_params`; query string and headers arrive as
`req.query_params` / `req.headers`.

## Configuration

| Field | Default | Description |
|---|---|---|
| `port` | `3111` | TCP port the HTTP server binds to. `0` binds an OS-assigned ephemeral port. |
| `host` | `0.0.0.0` | Host/interface to bind. |
| `default_timeout` | `30000` (ms) | Per-request timeout; on expiry the server returns `504`. |
| `cors.allowed_origins` | `[]` (permissive) | Allowed CORS origins. An empty list allows any origin. |
| `cors.allowed_methods` | `[]` (permissive) | Allowed CORS methods. An empty list allows any method. |
| `concurrency_request_limit` | `1024` | Maximum in-flight requests; requests over the limit wait for a slot. |
| `middleware[]` | `[]` | Global middleware, each `{ function_id, phase, priority }`, run on every route in ascending `priority` order before the handler. |

Configuration is owned by the `configuration` worker — edit it from the
console (**Configuration → Workers → http**) or seed it once via
`--config <file>.yaml` on first boot. `middleware` and `default_timeout`
hot-reload without a restart; `port`/`host`/`cors`/`concurrency_request_limit`
take effect on the next restart.

## Trigger type

This worker always registers the `http` trigger type. Bind a function to it
with:

| Field | Required | Default | Description |
|---|---|---|---|
| `api_path` | yes | — | Route path, e.g. `/orders/:id`. Segments prefixed with `:` are extracted into `path_params`. |
| `http_method` | no | `GET` | HTTP method to match. |
| `condition_function_id` | no | — | Function invoked first; if it returns a falsy value the request is rejected with `422`. |
| `middleware_function_ids` | no | `[]` | Per-route middleware, invoked before the handler in list order, in addition to any global middleware. |

Functions can stream their response: write to `req.response` (a
`StreamChannelRef`) with a `ChannelWriter` to send `set_status` /
`set_headers` control frames and body chunks for a chunked HTTP response.
Returning a non-null value instead yields a regular buffered response built
from `{ status_code, headers, body }`.

### Requires removing the legacy built-in HTTP service

The legacy built-in HTTP service also owns the `http` trigger type. Two owners
of the same trigger type on one engine collide — whichever registers last
wins — so this worker requires it to be absent: omit it from the
engine's `config.yaml` (a config that doesn't list a worker won't run it).

On boot, this worker queries the engine for connected workers and refuses to
start with a clear error if the legacy built-in is still active, so a stale config
fails loudly instead of silently racing the built-in worker for ownership of
`http`.
