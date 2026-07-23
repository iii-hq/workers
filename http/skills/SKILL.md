---
type: index
name: http
tags: http, routing, triggers, webhooks, endpoints
description: >-
  Expose registered functions as HTTP endpoints with the standalone http
  worker. Its whole surface is the `http` trigger type.
---

# http

The `http` worker exposes registered functions as HTTP endpoints, turning a
function into a route that browsers, webhooks, and external clients can call
without using the iii SDK. It exposes no callable `http::*` functions; its
entire surface is the `http` trigger type, bound through a worker SDK trigger
registration such as `iii.registerTrigger({ type: 'http', function_id, config })`.

Install it with `iii worker add http`. The engine builtin `iii-http` must not
run on the same engine because it also owns the `http` trigger type. Remove
`iii-http` from the engine config before starting this worker; the standalone
worker refuses to boot when the builtin is active.

Incoming requests are matched by HTTP method and path. The invoked handler
receives method, path, headers, query params, extracted `:path` params, parsed
body, trigger metadata, and request/response stream channels. Returning a JSON
value yields a buffered response; writing to the response channel yields a
chunked streaming response.

## When to Use

- A webhook, browser, mobile app, or third-party service needs to call an iii
  function over plain HTTP.
- You need route matching with path params such as `/orders/:id`.
- A route should have per-route middleware or a condition function before the
  handler runs.
- You need CORS, HTTP status codes, headers, JSON bodies, or chunked streaming
  responses at the edge of the engine.
- Several external routes should dispatch into normal iii functions while
  keeping their handler code in workers you control.

## Boundaries

- No callable functions are exposed. Never invoke `http::*`; bind the `http`
  trigger type to one of your own functions.
- This is inbound HTTP. For outbound network requests from an agent or worker,
  use `web::fetch` instead.
- This is not a scheduler or event queue. For clock-based execution, use
  `cron`; for data-change reactions, use the relevant state, stream, queue, or
  database worker.
- The built-in `iii-http` worker cannot run beside this standalone worker on
  the same engine because both own the `http` trigger type.
- Route shapes with the same method and equivalent path structure conflict:
  `/orders/:id` and `/orders/:order_id` are the same shape.

## Reactive triggers

Bind an `http` trigger when a handler should run automatically for each
matching HTTP request. The handler runs through the engine as a normal function
invocation, and the worker turns the handler's return value or stream writes
back into an HTTP response.

Reach for it when:

- A public or internal API endpoint should call a registered function.
- A webhook provider should invoke an iii workflow without a custom server.
- Middleware should authenticate, enrich, or reject requests before the route
  handler runs.
- A handler should stream chunks to the client instead of returning one
  buffered body.

### How to bind

1. Register a handler: `iii.registerFunction('orders::get', handler)`.
2. Register the trigger:

```typescript
iii.registerTrigger({
  type: 'http',
  function_id: 'orders::get',
  config: {
    api_path: '/orders/:id',
    http_method: 'GET',
    // condition_function_id and middleware_function_ids are optional.
  },
})
```

`api_path` is required. `http_method` defaults to `GET`. Path segments prefixed
with `:` are extracted into `path_params`; query string values arrive in
`query_params`; request headers arrive in `headers`.

Trigger config:

| Field | Required | Default | Description |
|---|---|---|---|
| `api_path` | yes | - | Route path such as `/orders/:id`; `:name` segments become path params. |
| `http_method` | no | `GET` | HTTP method to match. |
| `condition_function_id` | no | - | Function invoked before middleware and handler; a falsy result rejects the request with `422`. |
| `middleware_function_ids` | no | `[]` | Per-route middleware functions, run before the handler in list order. |

Handler return values become buffered responses with `status_code`, `headers`,
and `body`; `status_code` defaults to `200` and `body` defaults to an empty
object. For streaming, write `set_status` and `set_headers` control messages
plus body chunks to the request's `response` channel; return `null` or no final
buffered body from the handler.

Configure the server through the central configuration entry for worker
`http`:

```yaml
port: 3111
host: 0.0.0.0
default_timeout: 30000
concurrency_request_limit: 1024
cors:
  allowed_origins: []
  allowed_methods: []
middleware: []
```

Global `middleware` runs on every route in ascending `priority` order. Per-route
`middleware_function_ids` run after global middleware and before the handler.
`middleware` and `default_timeout` hot-reload; `port`, `host`, CORS, and
`concurrency_request_limit` take effect on the next restart.
