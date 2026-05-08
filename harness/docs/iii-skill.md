# iii functions

How to discover and call functions on the iii engine.

## If you're an agent calling through `agent_call`

You don't call `iii.trigger` directly — you go through the `agent_call` tool.
Three differences from the SDK examples below:

1. The argument is named **`function`**, not `function_id`. Same string,
   different field name.
   - Wrong: `agent_call({function_id: "...", payload: {...}})` → returns
     `{error: "missing_function"}`.
   - Right: `agent_call({function: "...", payload: {...}})`.
2. Errors arrive as **JSON envelopes inside the result**, not as thrown
   `IIIError`. You will see `{error: "function_not_found", function}`,
   `{error: "timeout", function}`, `{error: "trigger_failed", function,
   message}`, or `{blocked: true}` (policy refusal).
3. `action` and `timeout_ms` are **not exposed** through `agent_call`.
   Every call is synchronous with the bus default timeout. Putting these
   fields in `payload` does nothing.

`skill::fetch` is a real, callable function for loading skill bodies by
`iii://` URI — the blacklist below is about *function-listing* calls
only.

Everything else in this document — discovery, schemas, listings —
applies as written.

## TL;DR — there is exactly ONE way to list functions

```json
{ "function_id": "engine::functions::list",
  "payload": { "include_internal": false } }
```

That is the only function-listing call. It exists. It always works.
The response is `{ "functions": [ { function_id, description, request_format, response_format, metadata }, ... ] }`.

**Do NOT guess any of these — none of them exist:**

- ~~`skill::list`~~ → use `engine::functions::list`
- ~~`skills::list`~~ → that is a skills-registry CRUD call (lists skill bodies, not functions)
- ~~`iii::list`~~ → not a thing
- ~~`bus::list`~~ → not a thing
- ~~`function::list`~~ → wrong scope; the scope is `engine`, the noun is plural `functions`
- ~~`functions::list`~~ → missing the `engine::` prefix
- ~~`engine::list`~~ → missing the `functions` segment

If a call returns `{"error":"function_not_found", ...}`, **do not retry
with another guess**. Call `engine::functions::list` first; pick a real
id from the response.

## The mental model

- **Worker** — a process connected to the engine over WebSocket.
- **Function** — a unit of work with a stable id `<scope>::<name>`
  (e.g. `state::set`, `harness::status`). JSON in, JSON out.
- **Trigger** — what causes a function to run (direct call, HTTP,
  cron, queue, stream, custom).

To use a function you need three things: its **id**, its **input
schema**, and its **output schema**. All three live in the
`engine::functions::list` response. Read that listing first.

## Step 1 — Discover what exists

```json
{ "function_id": "engine::functions::list",
  "payload": { "include_internal": false } }
```

Returns:

```json
{
  "functions": [
    {
      "function_id": "harness::status",
      "description": "Returns the harness bundle name, version, ...",
      "request_format":  { "type": "object", "properties": {}, ... },
      "response_format": { "type": "object", "properties": {...} },
      "metadata": null
    },
    ...
  ]
}
```

| Field | Meaning |
|---|---|
| `function_id` | The id you pass as `function_id` when calling. |
| `description` | One-line summary the worker registered. |
| `request_format` | JSON Schema of the accepted payload. **This is the contract.** |
| `response_format` | JSON Schema of the return value. |
| `metadata` | Optional free-form annotations the worker attached. |

`include_internal: true` adds engine-private functions (the `engine::*`
namespace itself). Default `false` is what you want unless you're
debugging the engine.

## Step 2 — Read the schema for the function you want to call

Filter the listing to the one entry you care about, then read its
`request_format`:

- `request_format.required` — array of property names that MUST be present.
- `request_format.properties` — keys = allowed fields, values = per-field schema.
- Each property's schema gives you the type, enum, `oneOf`/`anyOf`, etc.
- If `request_format` is `null` or absent, the function accepts `{}`.

Read `response_format` too so you know the shape of what comes back.

## Step 3 — Call it

```rust
let result = iii.trigger(TriggerRequest {
    function_id: "myworker::do_thing".into(),
    payload: json!({ /* matches request_format */ }),
    action: None,                  // sync. Use "fire-and-forget" or "enqueue" only when you mean it.
    timeout_ms: Some(5_000),
}).await?;
```

```ts
const result = await iii.trigger({
  function_id: 'myworker::do_thing',
  payload: { /* matches request_format */ },
})
```

```python
result = await iii.trigger(
    function_id='myworker::do_thing',
    payload={ ... },
)
```

The result is the function's raw JSON output (matching
`response_format`). At the raw SDK layer, errors arrive as a thrown
`IIIError`. Through `agent_call` (the agent path), the dispatcher
converts those into JSON envelopes inside the result — see the
preamble above for the exact shapes.

## Step 4 — Adjacent listings (same scope, plural noun, ::list)

The same `engine::<plural>::list` shape works for three other things:

| Function | Returns |
|---|---|
| `engine::workers::list` | Connected workers + their function_ids, runtime, status, metrics. |
| `engine::triggers::list` | Active triggers (HTTP routes, cron, queue subs) and the function each invokes. |
| `engine::trigger-types::list` | Built-in and custom trigger types with their config schemas. |

All four (`functions`, `workers`, `triggers`, `trigger-types`) accept
`{ include_internal?: bool }` and return `{ <plural>: [ ... ] }`.

## Step 5 — Attach metadata to a worker: `engine::workers::register`

The engine knows a worker exists the moment it dials the WebSocket,
but it doesn't yet know its language, version, hostname, or framework.
`engine::workers::register` is the **write** call that fills those
fields in. It's what makes a row in `engine::workers::list` go from
"unknown runtime" to "node 20.x, project foo, framework express".

```json
{ "function_id": "engine::workers::register",
  "payload": {
    "runtime":  "rust",
    "version":  "0.3.1",
    "name":     "harness@host-12",
    "os":       "darwin 25.0",
    "pid":      9876,
    "isolation": "libkrun",
    "telemetry": {
      "language":     "en-US",
      "project_name": "my-project",
      "framework":    "express"
    }
  }
}
```

| Field | Meaning |
|---|---|
| `runtime` | `"node"`, `"python"`, `"rust"`, etc. Drives console grouping. |
| `version` | Worker's package/binary version. |
| `name` | Display name for the console (`<worker>@<host>` is conventional). |
| `os` | OS string (e.g. `"darwin 25.0"`, `"linux 6.5"`). |
| `pid` | Process id. Optional. |
| `isolation` | `"libkrun"`, `"docker"`, `"none"`, … Optional. |
| `telemetry` | Free-form `{ language, project_name, framework }` block. |

Returns `{ "success": true }`. Fires the custom trigger type
`engine::workers-available` so dashboards refresh.

`_caller_worker_id` is **injected automatically** by the engine on
every call — never pass it yourself; the engine attributes the
metadata to whatever worker made the call.

This is something a worker calls **about itself** at boot, normally
once. Most SDK `register_worker(...)` helpers already invoke it for
you; you only call it explicitly when you need to update metadata
mid-session or are speaking the bus protocol directly.

## Built-in namespaces (real ids, copy-pasteable)

These are always present because the engine itself registers them.

| Prefix | Examples |
|---|---|
| `engine::*` | `engine::functions::list`, `engine::workers::list`, `engine::workers::register`, `engine::triggers::list`, `engine::trigger-types::list` |
| `state::*` | `state::get`, `state::set`, `state::list`, `state::delete` |
| `stream::*` | `stream::set`, `stream::get`, `stream::list`, `stream::update`, `stream::delete` |

`stream::*` has no tail/range API — to consume a stream live, register
a `stream` trigger bound to your function.

This bundle's harness adds:

- `harness::status` — bundle name, version, expected workers.
- `bridge::trigger` — HTTP POST `/bridge/trigger` forwards `{function_id, payload}` onto the bus.

## Discovery checklist (use before EVERY new call)

1. `engine::functions::list { include_internal: false }` — confirm
   the id exists. **Do not skip this.**
2. Read `request_format` for that entry — know exactly what fields
   the payload needs.
3. Read `response_format` — know the shape of the return value before
   you write code that consumes it.
4. (Optional) `engine::workers::list` filtered by the function's
   worker — confirm the worker is connected and `status` is healthy.
5. Call `iii.trigger({ function_id, payload })` with a payload that
   satisfies the schema.

If a call fails with `function_not_found`, the function does NOT
exist under that id. Re-run step 1 and pick a real id from the
response — never invent another guess.
