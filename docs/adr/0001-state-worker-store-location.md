# ADR 0001: State worker — store location

## Status
Accepted (2026-07-06).

## Context
The legacy in-engine state service is not just a trigger handler: it owns a key-value
store that lives in the engine process (`engine/src/builtins/kv.rs`,
`BuiltinKvStore`, in_memory or file_based with a background flush loop).
Migrating that state service to the standalone `state` worker forces a decision about where that
store lives. Coupling analysis on the engine found ZERO inbound engine-internal
consumers of `state::*` (everything reaches state over the function bus), so the
functions can move; the question is only the store.

## Options

### (a) Store lives in the worker process — RECOMMENDED
Port `BuiltinKvStore` (file_based/in_memory, identical on-disk format) into the
worker; `redis` adapter ported as-is for multi-instance/production.

- Durability: store dies with the WORKER process instead of the engine process.
  `file_based` keeps the same crash window (unflushed writes within one
  `save_interval_ms` cadence); `redis` is unchanged. An engine restart no longer
  wipes in_memory state (the worker survives it); a worker restart now does.
- Latency: `state::get/set` change from in-process calls (µs) to engine⇄worker
  WebSocket round-trips (ms). This is the price of every builtin migration but
  state is the hottest path — MUST be benchmarked (see Acceptance criteria).
- No engine changes required; ships entirely from the workers repo.

### (b) Engine exposes the store as a service; worker is a thin façade
Engine keeps `BuiltinKvStore` behind new internal functions; the worker proxies.
- Keeps in-engine durability but ADDS a hop to every call, and requires an
  engine-repo change — out of scope for the workers repo. Rejected unless (a)'s
  benchmark fails the parity gate.

### (c) State remains a builtin
No migration. Rejected as the default: contradicts the registry-migration
program, but it is the fallback if the latency benchmark shows unacceptable
regression for real workloads (e.g. Motia per-step state reads).

## Decision
Option (a). Tasks 2–13 of the state-worker plan are written against it.

## Acceptance criteria
1. Durability profile documented in the worker README (parity table): what is
   lost on worker crash per adapter (`kv in_memory`: everything; `kv
   file_based`: ≤ one save_interval window; `redis`: nothing).
2. Latency delta acknowledged: `state::get/set` move from in-process calls
   (µs) to engine⇄worker WS round-trips (low ms). The formal p50/p95
   benchmark gate was waived by project decision on 2026-07-06; the README
   parity table records the order-of-magnitude delta instead.
