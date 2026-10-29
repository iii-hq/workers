# Worker model

How a worker relates to the iii engine and the workers registry.

## Lifecycle

A typical long-running worker:

1. **Connect** — `iii_sdk::register_worker` (Rust) or equivalent SDK call over
   WebSocket (`ws://127.0.0.1:49134` by default, overridable via `--url`).
2. **Register** — register functions, including the private functions that
   are the worker's events, set observer hooks on the ones that need setup per
   observer, and observe the functions whose calls should run one of its own.
3. **Serve** — handle invocations until SIGINT/SIGTERM.
4. **Shutdown** — `iii.shutdown_async().await` (Rust) for clean disconnect.

Binary workers support `--manifest` to print registry metadata (name, version,
`default_config`, `supported_targets`) and exit — used by the publish pipeline.

## Engine as bus

The iii engine is the message bus. Workers do not call each other directly;
they invoke functions with `iii.call({ function_id: 'worker::function', payload })`
and observe functions for reactive updates:
`iii.observe({ function_id: 'session::on-created', filters }, 'worker::on-session')`
runs `worker::on-session` after every matching call of `session::on-created`.

An event is a function. The worker that owns an event registers it as a
private function (only that worker calls it) whose handler returns the event,
and calls it when the event happens; its observers receive the event. A
function that needs setup per observer (a cron job, an HTTP route, a queue
consumer, a database trigger) takes `args` described by its `args_format` and
sets that up in its `onObserverAdded` hook.

Function ids are `::`-separated paths, conventionally prefixed by the worker's
domain: `shell::exec`, `session::append` (session-manager), `shell::fs::read`.

## Discovery

| Context | How workers are found |
|---|---|
| In-repo development | Folder with public `iii.worker.yaml`; release-only build metadata lives in `.deploy/workers.yaml` |
| Production install | Workers registry API — `iii worker add <name>` or `package://<name>` in Compose |
| Runtime catalogue | `engine::workers::list`, `engine::functions::list` |

Published workers ship a collected **interface** (their functions, each with
its schemas, `args_format`, `private` and `observable`) attached to the
registry manifest at release time.

## Deploy shapes

Workers ship as one of three deploy kinds (see [`deploy-modes.md`](deploy-modes.md)):

- **binary** — single cross-compiled CLI per target triple
- **image** — OCI container (Node/Python daemons)
- **bundle** — single-file archive (esbuild bundle for Node monorepos like harness)

The public kind is declared by `deploy` in `iii.worker.yaml`. The private
release catalog maps it to `artifact.kind`; the compiler rejects mismatches
before release build jobs can run.

## Related

- Scaffold: [`../sops/binary-worker.md`](../sops/binary-worker.md)
- Public manifest fields: [`iii-worker-yaml.md`](iii-worker-yaml.md)
- Public Compose and private release boundary: [`worker-compose.md`](worker-compose.md)
- Release: [`../sops/release.md`](../sops/release.md)
