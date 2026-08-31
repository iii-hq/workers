# code-runner — end-to-end harness

Self-asserting suite for the `code-runner` worker. Brings up a private engine,
starts the worker against it, and exercises the whole public surface —
`run`, `register_function`, `teardown` — over a real bus, **in both
languages**.

No docker, no database, no `/dev/kvm`, and **no `python3` or `node` on the
host**: V8 is linked into the worker binary and the interpreter is CPython
compiled to WebAssembly and embedded in it, not shelled out to. The only host
dependency is the engine itself.

## Run

```sh
./run-tests.sh                        # everything
./run-tests.sh --filter register      # only cases whose group/name matches
./run-tests.sh --filter 'non-ascii'   # matches group or case name
./run-tests.sh --keep                 # leave engine + worker up for poking
```

Exits 0 on PASS, 1 on any failure.

### Prerequisites

- Rust toolchain (`cargo` on `$PATH`)
- Node.js 20+ (`npm` on `$PATH`) — for the harness worker, not for the sandbox
- The iii engine, either on `$PATH` as `iii` or via `III_BIN=/path/to/iii`

## Groups

| group | what it covers |
|---|---|
| `run` | one-shot runs in both languages, the shared response shape, non-ASCII round-trips, and guest `iii.trigger` reaching the bus |
| `keep` | `keep: true` → `runtime_id` → `teardown`, and that an id routes to its own engine without being told the language |
| `register` | handlers published on the real bus and invoked through it, in both languages, plus namespace teardown |
| `errors` | the taxonomy: which failures are responses and which are errors |

### Why this suite and not more unit tests

Three things only a live engine can show:

- **A registered function actually reaches the catalog** and answers a
  trigger. A unit test can prove the router called `Engine::register`; it
  cannot prove a caller can then invoke the id.
- **The response shape is identical across engines** once it has been through
  serde, the wire, and the SDK's decoder — the whole promise of this worker is
  one API over two of them.
- **A duplicate function id would abort the process.** Node and python claim
  ids in one shared registry precisely to prevent that; `register :: a function
  id cannot be claimed by both engines` asserts the refusal *and* that the
  worker is still serving afterwards, which is the half a panic would have
  taken out.

### Leaks fail later cases, not their own

`max_runtimes` is shared across both engines, so a case that creates a runtime
or a namespace and does not tear it down fails a **later** case with a capacity
error that says nothing about the real cause. Every case here tears down what
it creates, in a `finally`.

### Ports

The engine listens on a random high port, never 49134 — a developer's own
stack usually holds that one, and a harness that hijacked it would take their
workers down. The port is verified free before use, because the engine logs
"address already in use" and then keeps running, which would otherwise produce
a harness talking to somebody else's engine and reporting nonsense about it.

## Artifacts

`reports/` holds `report.json` (machine-readable results) plus the engine,
worker and harness logs from the last run. All git-ignored.
