# code-runner

Run untrusted Node.js and Python **in-process**: iterate with
`code-runner::run`, publish working functions to the bus with
`code-runner::register_function`, clean up with `code-runner::teardown`.

Same wire contract as
[sandbox-code-runner](../sandbox-code-runner/), so a caller written against
that worker needs no changes — but no microVM, no daemon, and **no
`/dev/kvm`**. JavaScript runs in a V8 isolate; Python runs as CPython compiled
to WebAssembly inside wasmtime.

```bash
iii worker add code-runner
```

## Quickstart

```bash
iii trigger code-runner::run lang=node code='console.log("hi"); return 2 + 2'
```

```json
{ "stdout": "hi\n", "stderr": "", "exit_code": 0, "success": true,
  "duration_ms": 12, "result": 4 }
```

`run` is **one-shot by default**: it creates a runtime, runs your code,
returns, and destroys it. Nothing persists, and the response carries no
`runtime_id` because there is nothing left to address.

Pass `keep: true` to leave it running. The response's `runtime_id` then
addresses it on later calls — same globals, same scratch directory — and is
the capability `teardown` needs.

**A failing script is a response, not an error.** `stdout`, `stderr` and
`exit_code` come back verbatim; a thrown exception is `exit_code: 1` with the
message on `stderr`, not a failed bus call. Errors are reserved for
infrastructure: timeouts, resource kills, unknown runtimes.

`result` is this worker's one addition to sandbox-code-runner's response: the
value your code returned, so a caller does not have to print JSON and parse
`stdout`.

## The `iii` global

Code you run gets a global `iii`:

- `await iii.trigger({ function_id, payload })` — invoke any bus function.
- `iii.registerFunction(id, handler, opts?)` — publish one for the life of
  this runtime. For one that outlives it, use `code-runner::register_function`.
  `opts` takes `description`, and `request_format` / `response_format` — JSON
  Schema objects `engine::functions::info` shows callers in place of "any".
- `iii.files` — a **private scratch directory** that lives exactly as long as
  the runtime: `write(name, contents)`, `read(name)`, `readText(name)`,
  `list()`, `remove(name)`. Names are one file each — no paths, no
  subdirectories — and the directory is bounded by `scratch_mb`/`scratch_files`.
- `iii.namespace` — the prefix this runtime may register under.

Python's `iii` is narrower: `iii.trigger({...})` and `iii.namespace`, and it
is **synchronous** — the call blocks until the answer comes back. Registration
from inside guest Python is not available; use `code-runner::register_function`
(node) or trigger a worker that registers for you. A guest timeout is clamped
to what is left of the run's own budget, so `iii.trigger` inside a 5s run
cannot ask for 30s and take the whole run down with it.

```python
answer = iii.trigger({"function_id": "state::get", "payload": {"key": "k"}})
result = answer["value"]
```

## What this cannot do

Read this before choosing between the two workers.

| | `code-runner` | `sandbox-code-runner` |
|---|---|---|
| Isolation | V8 isolate / wasm sandbox, in-process | microVM per runtime |
| Requires `/dev/kvm` or Apple Silicon | **no** | yes |
| Outbound network | **no** | yes |
| `npm install` / `pip install` | **no** | yes |
| Real filesystem | no — only `iii.files` | yes, a whole OS |
| Guest `iii` | host-implemented surface | the real iii-sdk client |
| Per-call cost | milliseconds | VM boot |

**There is no network, so package installation cannot work at all** — not
slowly, not with a flag. If your code needs a third-party package, use
`sandbox-code-runner`. What you can do here is write your own modules into
`iii.files` and load them from there.

## Configuration

`config.yaml`:

| key | default | meaning |
|---|---|---|
| `max_runtimes` | 32 | live runtimes across both engines |
| `default_timeout_ms` | 5000 | per run when unspecified |
| `max_timeout_ms` | 30000 | ceiling a request's `timeout_ms` is clamped to |
| `idle_ttl_secs` | 900 | reap runtimes idle this long |
| `heap_mb` | 128 | V8 object-heap cap per node runtime |
| `external_mb` | 64 | off-heap cap per node runtime; `heap_mb` does not cover it |
| `scratch_mb` | 8 | `iii.files` quota per runtime; **0 disables it entirely** |
| `scratch_files` | 64 | max files per runtime |
| `scratch_root` | unset | where scratch directories live |

**Worst-case host footprint is `max_runtimes * scratch_mb`** — 256 MiB at the
defaults — and the system temp directory is tmpfs, i.e. host RAM, on most
Linux hosts. Set `scratch_root` to real disk if that matters.

## Errors

| code | meaning |
|---|---|
| `code-runner::invalid_request` | malformed request, or a `lang` that contradicts the runtime |
| `code-runner::runtime_not_found` | unknown `runtime_id`, or a namespace with nothing behind it |
| `code-runner::expired` | the runtime was reaped or killed; get a fresh one |
| `code-runner::capacity` | all runtime slots in use — retry later |
| `code-runner::timeout` | the run blew its deadline |
| `code-runner::resource_exhausted` | a cap was hit mid-run (memory, scratch quota). Unlike `capacity`, retrying will not help — shrink the workload |
| `code-runner::handler_error` | a registered handler threw |
| `code-runner::engine` | infrastructure failure |

## Console UI

The worker ships an injected console UI (the injectable-UI protocol,
`iii/tech-specs/2026-07-17-injectable-ui`) — two assets, built from `ui/` by
esbuild and embedded in the binary, so there is nothing to install and nothing
to serve separately:

| Asset | Slot | What it does |
|---|---|---|
| `code-runner/page.js` | `console:script` | how `run` / `register_function` / `teardown` render in chat and the traces span view |
| `code-runner/styles.css` | `console:style` | the stylesheet, every rule scoped under `[data-iii-ui="code-runner"]` |

One renderer per op, under `ui/src/function-trigger-message/`, sharing the card
frame in `ui/src/lib/shared.tsx`. They replace the console's raw-JSON card,
which turns `code` into one escaped line and buries the verdict.

What the cards are for:

- **`run`** leads with the verdict (`exit`, duration), then the **completion
  value**, then the source highlighted as the language it runs as, then
  stdout/stderr as terminal output. A **non-zero exit renders as a warning, not
  an alert** — errors are for infrastructure, and a script that throws is an
  ordinary response.
- A **null `result`** is shown explicitly, with the engine's return convention
  beside it: node code is a function body (`return 2 + 2`), python code is a
  module (assign `result`). That mismatch is the usual reason a call "worked"
  and returned nothing.
- **`register_function`** puts the namespace the id claims on its own line, and
  advises when the source defines no `handler` — on python that fails the
  registration outright, not the first call.
- **`teardown`** answers the one question the payload does not: which function
  ids stopped resolving.

**`runtime_id` is never rendered in full.** It is a capability — whoever holds
one can run into or tear down that runtime — so it appears only as a truncated
chip whose full value is handed over on an explicit click-to-copy. Every other
string on a card is filtered first: stdout, stderr, error messages (which quote
the id by design — see `src/error.rs`), the submitted source, the completion
value, and the `raw json` tab the console mounts regardless of what a card does.

### Developing it

```sh
cd ui && pnpm install && pnpm build   # cargo build does this for you
pnpm test                             # vitest, renders the cards server-side
```

`build.rs` runs `pnpm install && pnpm build` when `ui/dist/` is missing or
stale, so a plain `cargo build` is enough. `SKIP_UI_BUILD=1` uses the existing
`ui/dist/` as-is. For a hot-reload loop, run `pnpm watch` and start the worker
with `III_CODE_RUNNER_UI_WATCH=1` — every open console tab swaps the changed
asset.

## Status

| | node | python |
|---|---|---|
| one-shot `run` | yes | yes |
| `keep` / `runtime_id` | yes | **yes** |
| what a kept runtime persists | globals **and** files | globals **and** files |
| `register_function` | yes | **yes** |
| `iii.trigger` from guest code | yes | **yes** |
| `iii.registerFunction` from guest code | yes | no |
| `iii.files` | yes | use `/work` |

**A kept python runtime persists its working directory *and* its
interpreter.** `python.wasm` is a WASI *command* module — the artifact exports
`memory` and `_start` and nothing else — so the interpreter cannot be re-entered
once `_start` returns. Instead it never returns: the wrapper parks on stdin
between calls and the host hands it one turn at a time. Globals stay bound,
modules stay imported, and files under `/work` stay put.

That is a **superset** of what `sandbox-code-runner` promises for its own
`keep`, which is *"same filesystem, fresh interpreter process each time"*. Code
written against that worker still behaves correctly here; code written against
this one may not port back.

Two consequences worth knowing before you rely on it:

- **A call that overruns its `timeout_ms` takes the interpreter with it.** The
  only kill that reaches a guest parked in a host call unwinds `_start`, so
  there is nothing left to resume. The working directory survives, and the next
  call on that `runtime_id` boots a fresh interpreter on the same files.
- **`memory_mb` is fixed when the runtime is created.** Wasm linear memory only
  ever grows, so a per-call ceiling would be a promise the sandbox cannot keep —
  a later caller would inherit whatever an earlier one grew to. Sending
  `memory_mb` alongside a `runtime_id` is refused rather than ignored.

Only `result` is cleared between turns. It is the output slot, not state: a
turn that assigns no `result` returns `null`, never the previous turn's value.

### `register_function` with `lang: "python"`

Your `source` must define `handler(payload)`; anything else it defines stays
available to every later invocation, because the namespace runs on one pinned
interpreter.

```python
# code-runner::register_function, lang=python, function_id="my-app::greet"
import json

_greetings = {"en": "hello", "pt": "olá"}

def handler(payload):
    return _greetings.get(payload.get("lang", "en"), "hello")
```

Four things worth knowing:

- **Source that leaves no callable `handler` fails the registration**, not the
  first call. The id never reaches the catalog, so a corrected redeploy of the
  same id works.
- **Re-registering an id replaces it.** The previous registration is retired
  before the new one is published.
- **The interpreter is exempt from the idle sweep.** A registration's lifetime
  is the registration's, not its traffic's; `teardown` with `namespace` is the
  only way to remove one.
- **The namespace is shared with node.** Function ids are claimed in one
  registry, so `my-app::greet` cannot exist in both languages, and the same
  charset rule applies to the namespace in both.
- **Schemas are optional but shown.** `request_format` / `response_format`
  (JSON Schema objects, max 16 KiB, must actually constrain something) ride
  the registration in either language, and `engine::functions::info` shows
  them in place of "any".

Unlike node, guest Python cannot register a function *from inside* a run —
`python.wasm` exports `_start` and nothing else, so there is no
`iii.registerFunction` to call. The host publishes on the guest's behalf and
dispatches each invocation as one turn on the namespace interpreter.

```python
# lang=python, keep=true — then reuse the runtime_id
open("/work/state.json", "w").write('{"n": 1}')
result = "saved"
```
