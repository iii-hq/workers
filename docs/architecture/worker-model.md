# Worker model

How a worker relates to the iii engine and the workers registry.

## Lifecycle

A typical long-running worker:

1. **Connect** — `iii_sdk::register_worker` (Rust) or equivalent SDK call over
   WebSocket (`ws://127.0.0.1:49134` by default, overridable via `--url`).
2. **Register** — declare custom trigger types, then register functions (and
   optionally subscribe to engine trigger types).
3. **Serve** — handle invocations until SIGINT/SIGTERM.
4. **Shutdown** — `iii.shutdown_async().await` (Rust) for clean disconnect.

Binary workers support `--manifest` to print registry metadata (name, version,
`default_config`, `supported_targets`) and exit — used by the publish pipeline.

## Engine as bus

The iii engine is the message bus. Workers do not call each other directly;
they invoke functions via `iii.trigger('worker::namespace::function', payload)`
and subscribe to trigger types for reactive updates.

Function ids are `::`-separated paths, conventionally prefixed by the worker's
domain: `shell::exec`, `session::append` (session-manager), `shell::fs::read`.

## Discovery

| Context | How workers are found |
|---|---|
| In-repo development | Folder at repo root with `iii.worker.yaml` |
| Production install | Workers registry API — `iii worker add <name>` |
| Runtime catalogue | `engine::workers::list`, `engine::functions::list` |

Published workers ship a collected **interface** (functions + trigger types)
attached to the registry manifest at release time.

## Deploy shapes

Workers ship as one of three deploy kinds (see [`deploy-modes.md`](deploy-modes.md)):

- **binary** — single cross-compiled CLI per target triple
- **image** — OCI container (Node/Python daemons)
- **bundle** — single-file archive (esbuild bundle for Node monorepos like harness)

The kind is declared in `iii.worker.yaml` `deploy` and routes CI smoke + release
build jobs.

## Related

- Scaffold: [`../sops/binary-worker.md`](../sops/binary-worker.md)
- Manifest fields: [`iii-worker-yaml.md`](iii-worker-yaml.md)
- Release: [`../sops/release.md`](../sops/release.md)
