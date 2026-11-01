---
name: http
tags: http, routing, observers, webhooks, endpoints
description: >-
  Expose registered functions as HTTP endpoints with the standalone http
  worker. A function serves a route by observing `http::on-request` with
  `{ method, path }` as args.
---

# http

The `http` worker exposes registered functions as HTTP endpoints, turning a
function into a route that browsers, webhooks, and external clients can call
without using the iii SDK. A route is an observer of `http::on-request`: the
args name the route, and the target is the function that answers it:
`iii.observe({ function_id: 'http::on-request', args: { method, path } }, target)`.

Install it with `iii call compose::add worker=http`.

Every HTTP request is one call of `http::on-request`. The worker matches the
request's method and path to a route and calls that route's target with the
request: method, path template, headers, query params, extracted `:path`
params, parsed body, and request/response stream channels. A path without a
route gets `404`, and a path whose routes are on other methods gets `405`.
Returning a JSON value yields a buffered response; writing to the response
channel yields a chunked streaming response.

## When to Use

- A webhook, browser, mobile app, or third-party service needs to call an iii
  function over plain HTTP.
- You need route matching with path params such as `/orders/:id`.
- A route should run middleware before the handler, or accept only requests
  whose headers or body match a filter.
- You need CORS, HTTP status codes, headers, JSON bodies, or chunked streaming
  responses at the edge of the engine.
- Several external routes should dispatch into normal iii functions while
  keeping their handler code in workers you control.

## Boundaries

- Never call `http::on-request`: it is private to the http worker. Observe it
  with one of your own functions as the target.
- This is inbound HTTP. For outbound network requests from an agent or worker,
  use `web::fetch` instead.
- This is not a scheduler or event queue. For clock-based execution, observe
  `cron::on-tick`; for data-change reactions, use the relevant state, stream,
  queue, or database worker.
- Route shapes with the same method and equivalent path structure conflict:
  `/orders/:id` and `/orders/:order_id` are the same shape. The later observer
  is rejected with `HTTP_ROUTE_CONFLICT`.
- A route lasts while its observer's worker is connected. A worker that
  restarts and observes again under the same observer `id` within the
  engine's `observers.observer_grace_ms` gets its route back. Pass a stable
  `id` in the observe options, and give the worker a name that stays the same
  across restarts: `III_WORKER_NAME` (`iii compose` and `iii worker` set it)
  or the SDK's worker name option. Under the default name, `hostname:pid`, a
  restarted process is another worker: its observe is rejected with
  `OBSERVER_ID_CONFLICT` until the grace period ends.
- `http::on-request` has one host per namespace: run one http worker per
  namespace.

## Serving a route

Observe `http::on-request` when a handler should run for each matching HTTP
request. The worker turns the handler's return value or stream writes into
the HTTP response.

Reach for it when:

- A public or internal API endpoint should call a registered function.
- A webhook provider should invoke an iii workflow without a custom server.
- Middleware should authenticate, enrich, or reject requests before the route
  handler runs.
- A handler should stream chunks to the client instead of returning one
  buffered body.

### How to observe

1. Register a handler: `iii.registerFunction('orders::get', handler)`.
2. Observe `http::on-request` with the route as args:

```typescript
const route = iii.observe(
  {
    function_id: 'http::on-request',
    args: {
      method: 'GET',
      path: '/orders/:id',
      // middleware_function_ids and public_webhook are optional.
    },
    // optional: filters on the request, e.g. { 'input.headers.x-api-key': { $exists: true } }
  },
  'orders::get',
)
await route.ready()
// later: await route.dispose() removes the route
```

Args:

| Arg | Required | Default | Description |
|---|---|---|---|
| `method` | yes | - | HTTP method to match, case insensitive. |
| `path` | yes | - | Route path such as `/orders/:id` (or `/orders/{id}`); `:name` segments become path params. |
| `public_webhook` | no | `false` | Also expose the route on the restricted webhook listener. |
| `middleware_function_ids` | no | `[]` | Per-route middleware functions, run before the handler in list order. |

Unknown args are refused with `INVALID_ARGS`. Path segments prefixed with `:`
are extracted into `path_params`; query string values arrive in
`query_params`; request headers arrive in `headers`. A request the observer's
`filters` reject gets `422 CONDITION_NOT_MET` without calling the handler.
Filters read the request as the server received it: `input.method`,
`input.path` (the request's path), `input.query_params`, `input.headers` and
`input.body`.
The handler receives the `HttpRequest` whatever the observer's `deliver`
option, and the observer's `metadata` arrives as its metadata sidecar.

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
`middleware`, `default_timeout`, CORS and `concurrency_request_limit` reload
live; a `port` or `host` change rebinds the listener.
