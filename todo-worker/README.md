# todo-worker

Quickstart CRUD todo worker built on the Node.js [iii SDK](https://github.com/iii-hq/iii). It exposes a tiny in-memory todos service over five HTTP routes and is the canonical "Node container" template referenced in [AGENTS-NEW-WORKER.md](../AGENTS-NEW-WORKER.md).

If you're scaffolding a new Node worker, start by copying this directory and trimming what you don't need.

## Functions

Each route is registered through the local [`useApi`](src/hooks.ts) helper, which derives a function id of the form `api::<method>::<path>`:

| Function ID | Description |
|---|---|
| `api::post::/todos` | Create a todo |
| `api::get::/todos` | List all todos |
| `api::get::/todos/:id` | Get a single todo by id |
| `api::put::/todos/:id` | Update a todo's `title` and/or `completed` |
| `api::delete::/todos/:id` | Delete a todo |

## HTTP triggers

| Method | Path | Body |
|---|---|---|
| POST | `/todos` | `{ "title": "buy milk" }` |
| GET | `/todos` | — |
| GET | `/todos/:id` | — |
| PUT | `/todos/:id` | `{ "title"?: string, "completed"?: boolean }` |
| DELETE | `/todos/:id` | — |

The worker stores todos in memory (`TodoStore` in [src/store.ts](src/store.ts)) — restarting the process clears all data. Swap `TodoStore` for an iii `state::*` call to make it persistent.

## Run locally

```bash
npm install
npm run dev      # tsx watch src/index.ts
```

By default the SDK connects to the engine at `ws://localhost:49134` (override with `III_URL`).

Once it's up:

```bash
curl -X POST http://localhost:3111/todos -H 'Content-Type: application/json' \
  -d '{"title":"buy milk"}'

curl http://localhost:3111/todos

curl -X PUT http://localhost:3111/todos/<id> -H 'Content-Type: application/json' \
  -d '{"completed":true}'

curl -X DELETE http://localhost:3111/todos/<id>
```

## Tests

```bash
npm test         # vitest run
```

See [tests/handlers.test.ts](tests/handlers.test.ts) for an example of testing the handler factory directly without booting the SDK.

## Build & container

```bash
npm run build                       # tsc → dist/
docker build -t todo-worker .       # multi-stage Node 22 image
```

The image inherits `III_URL=ws://localhost:49134`; override at runtime with `-e III_URL=...`.

## See also

- [todo-worker-python/README.md](../todo-worker-python/README.md) — same API, Python SDK.
- [AGENTS-NEW-WORKER.md](../AGENTS-NEW-WORKER.md) — full checklist for adding a new worker.
