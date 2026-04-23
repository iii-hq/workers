# todo-worker-python

Quickstart CRUD todo worker built on the Python [iii SDK](https://github.com/iii-hq/iii). Mirrors the Node-based [todo-worker](../todo-worker/) one-for-one and is the canonical "Python container" template referenced in [AGENTS-NEW-WORKER.md](../AGENTS-NEW-WORKER.md).

If you're scaffolding a new Python worker, start by copying this directory and trimming what you don't need.

## Functions

Each route is registered through the local [`use_api`](src/hooks.py) helper, which derives a function id of the form `api::<method>::<path>`:

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

The worker stores todos in memory (`TodoStore` in [src/store.py](src/store.py)) — restarting the process clears all data. Swap `TodoStore` for an iii `state::*` call to make it persistent.

## Run locally

```bash
pip install -e .
python -m src.main
```

The `pyproject.toml` also exposes a `todo-worker` console script and the `iii.worker.yaml` script entry uses `watchfiles` for live reload during development:

```bash
watchfiles 'python -m src.main'
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

## Container

```bash
docker build -t todo-worker-python .   # python:3.12-slim image
```

The image inherits `III_URL=ws://localhost:49134`; override at runtime with `-e III_URL=...`.

## Tests

A `tests/` directory is not yet present. Per [AGENTS-NEW-WORKER.md](../AGENTS-NEW-WORKER.md) §5, Python workers should ship `tests/test_*.py` runnable with `pytest`. Adding one is the next step before this worker can be released through the standard `pr-checks` flow.

## See also

- [todo-worker/README.md](../todo-worker/README.md) — same API, Node SDK.
- [AGENTS-NEW-WORKER.md](../AGENTS-NEW-WORKER.md) — full checklist for adding a new worker.
