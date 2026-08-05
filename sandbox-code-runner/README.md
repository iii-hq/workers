# sandbox-code-runner

Run Node.js and Python in isolated microVMs: iterate on code with
`sandbox-code-runner::run`, publish working functions to the bus with
`sandbox-code-runner::register_function`, clean up with
`sandbox-code-runner::teardown`.
Code running inside the VM gets a global `iii` — the real
[iii-sdk](https://iii.dev/docs/reference/sdk-node) client, lazily
connected to the engine ([details below](#the-iii-global)).

sandbox-code-runner delegates every execution to the
[iii-sandbox daemon](https://workers.iii.dev/workers/iii-sandbox)
(`sandbox::*` triggers) rather than running an in-process interpreter — you
get Python, npm/pip, and a real OS per call, at the cost of heavier runtimes.
sandbox-code-runner itself executes nothing and touches no host filesystem.

## Install

```bash
iii worker add sandbox-code-runner
```

sandbox-code-runner delegates every execution to the iii-sandbox daemon, so
install that too:

```bash
iii worker add iii-sandbox
```

Missing iii-sandbox is not fatal — sandbox-code-runner warns loudly at boot
and keeps serving; every call fails with a clear message until you add it.

## Prerequisites

- Hardware virtualization on the engine host (`/dev/kvm` on Linux, Apple
  Silicon on macOS) — the iii-sandbox daemon's requirement, inherited.

## Running code

`run` is **one-shot by default**: it boots a VM, runs your code, returns
the result, and destroys the VM before the response is even sent. Nothing
persists — no files, no installed packages — and the response carries no
`runtime_id`, because there is nothing left to address.

```bash
iii trigger sandbox-code-runner::run lang=python code='print(2+2)'
# → { "stdout": "4\n", "stderr": "", "exit_code": 0, "success": true, "duration_ms": … }
```

Pass `keep=true` to leave the VM running instead. The response's
`runtime_id` then addresses it — treat it as a secret — and is the
capability `sandbox-code-runner::teardown` needs to stop it:

```bash
iii trigger sandbox-code-runner::run lang=python code='print(2+2)' keep=true
# → { "runtime_id": "rt-…", "stdout": "4\n", … }

# run more code in the SAME runtime (same filesystem, fresh process)
iii trigger sandbox-code-runner::run runtime_id=rt-… code='print(open("/tmp/x").read())'
```

Passing `runtime_id` back in a later run reuses that VM: **variables do
not survive between runs; files and installed packages do.** A runtime
you hold via `runtime_id` is never auto-stopped — you own it until you
tear it down (`sandbox-code-runner::teardown`) or its idle TTL reaps it. A
failing script is a response (`success: false`, `stderr`), not an error —
errors are reserved for infrastructure (unknown runtime, expired VM,
timeouts, capacity).

**Every VM has outbound network.** The `iii` global's engine link rides
the sandbox's network gateway, so networking is always enabled —
`npm install` / `pip install` work in every runtime, one-shot runs
included. (Earlier versions had a `network` request field with refusal
semantics; it is gone, and a request still carrying it is ignored
harmlessly.)

## Registering a function

`register_function` needs **no `runtime_id`**. Give it `function_id`,
`source`, `lang`, and an optional `description`; sandbox-code-runner keeps
one persistent runtime per **namespace** (the first segment of the id —
`app::greet` claims `app::`) and language, creating it on the namespace's
first registration and reusing it for every later one in the same
namespace and `lang`:

```bash
iii trigger sandbox-code-runner::register_function \
  function_id=my-app::double \
  lang=python \
  description='Double a number. Payload: { n: number }.' \
  source='def handler(payload):
    return {"doubled": payload["n"] * 2}'

iii trigger my-app::double n=21
# → { "doubled": 42 }
```

`source` must define `handler(payload)` in `lang`. The first registered id
in a namespace claims it; later ids there must share it — AND share its
`lang`, since a runtime is single-language. Each call runs in a fresh
interpreter process inside the namespace's runtime at the configured
`default_timeout_ms`; anything the handler prints goes to
sandbox-code-runner's debug log, and the caller receives exactly what
`handler` returned, JSON-serialized.

The runtime backing a namespace is entirely an implementation detail — you
never see or manage its `runtime_id`. It carries no network access (there
is no `network` field on this request either).

## The `iii` global

The code you run and registered handlers both see a global `iii`: the real
iii-sdk client
([Node reference](https://iii.dev/docs/reference/sdk-node),
[Python reference](https://iii.dev/docs/reference/sdk-python)), created
lazily — nothing dials the engine until the first use, so code that never
touches `iii` pays nothing.

```js
// node — the SDK's IIIClient; trigger returns a Promise
const rows = await iii.trigger({
  function_id: "database::query",
  payload: { sql: "SELECT 1" },
});
```

```python
# python — synchronous (the SDK's trigger_async exists too)
rows = iii.trigger({'function_id': 'database::query',
                    'payload': {'sql': 'SELECT 1'}})
```

- **The full SDK surface is available** — `trigger`, `registerFunction`,
  `registerTrigger`, connection-state listeners, `shutdown` (the wrapper
  calls it for you after the run) — exactly as the reference documents it.
  The global explains itself: `console.log(iii)` / `repr(iii)` print a
  usage hint before anything has connected, and after first use
  `Object.keys(iii)` / `dir(iii)` list the client's callable surface —
  none of which opens a connection by itself.
- **SDK registrations are ephemeral.** `iii.registerFunction` registers
  THIS guest process, and a run's process exits moments later — its
  registrations (and trigger bindings) go with it. They are genuinely live
  while it runs (a run can trigger its own registration through the
  engine); for a function that outlives the process, call
  `sandbox-code-runner::register_function` through `iii.trigger` —
  everything in the section above applies.
- **Delivery.** Node runtimes get the SDK planted at
  `/node_modules/iii-sdk` from a bundle embedded in sandbox-code-runner —
  no registry, no `npm install`, works offline. Python runtimes
  `pip install iii-sdk` once at runtime creation (its pydantic-core
  dependency is compiled per-platform, so planting is not an option); if
  that install fails — no PyPI route, say — the runtime still works and
  `iii` raises a clear "not installed" error on first use.
- **Identity and reach.** The guest connects to the engine as an ordinary
  worker (`III_URL` is set at runtime creation and rides the sandbox
  gateway), named `sandbox-code-runner:run` or
  `sandbox-code-runner:<function_id>`. What guest code may call is whatever
  the engine lets a connected worker call — the same trust model as a
  worker process you run yourself. The VM's network also reaches the
  internet and, via the gateway, services on the engine host's loopback —
  do not run code you would not run as a worker.
- **Self-calls stall.** A registered handler that triggers a function
  living on ITS OWN runtime waits on that runtime's one-exec-at-a-time
  slot — the very slot its own call is holding — so it can only time out.
  Calls to functions on other runtimes, to other workers, and from runs
  (whose runtimes host no registered functions) all work, including
  nested.

## Teardown and expiry

Pass **exactly one** of `runtime_id` (a kept run's runtime) or
`namespace` (every runtime — one per language — backing a
`register_function` namespace):

```bash
iii trigger sandbox-code-runner::teardown runtime_id=rt-…
# → { "runtime_id": "rt-…", "torn_down": true, "unregistered": [] }

iii trigger sandbox-code-runner::teardown namespace=my-app
# → { "namespace": "my-app::", "torn_down": true, "unregistered": ["my-app::double"] }
```

Passing both, or neither, is refused (`invalid_request`) with a message
naming which one to use. Tearing down a namespace destroys every runtime
backing it (one per language it was used in) and unregisters every
function any of them had published — exactly as tearing down a single
kept-run runtime unregisters that runtime's functions.

Idle runtimes are reaped by the iii-sandbox daemon after `idle_ttl_secs`
(default 900; any run or call resets the clock). A reaped runtime
surfaces as `sandbox-code-runner::expired` on its next use, its bus
functions are unregistered, and its id is forgotten — for a kept-run
runtime, run again with `keep: true` (or one-shot, if persistence is no
longer needed) to boot a fresh one; for a namespace runtime, the next
`register_function` in that namespace boots a fresh one automatically.
There is no auto-respawn: a revived VM would have lost its filesystem
(installed packages included), and a half-working function is worse than
an honest error.

**Unregistration is lazy.** A reaped runtime's bus functions are not
removed the moment the TTL passes — they are unregistered only when
something next calls into the dead runtime and gets the `expired` outcome
above. Until then, a stale runtime's functions still show up in the
catalog (e.g. `engine::functions::info`) even though they can no longer be
invoked.

**Restarting sandbox-code-runner also invalidates every outstanding
`runtime_id`** and every namespace binding — all state is in-process —
but with a **different** error: `sandbox-code-runner::runtime_not_found`,
not `expired`, since the new process has no record of the old id at all
(and a `namespace` teardown against a namespace with no live runtime gets
the same code). The orphaned microVM itself is unaffected by the restart
and keeps running in the iii-sandbox daemon until its own idle TTL reaps
it; there is no drain-on-shutdown.

## Configuration

```yaml
default_timeout_ms: 5000  # per run and per handler invocation when unspecified
max_timeout_ms: 30000     # ceiling a request's timeout_ms is clamped to
idle_ttl_secs: 900        # passed to sandbox::create; the daemon reaps idle VMs
```

Images (`node`, `python`), CPU/memory caps, sandbox concurrency, and the
image allowlist are the iii-sandbox daemon's configuration —
sandbox-code-runner deliberately duplicates none of them; a daemon-side
refusal (e.g. capacity) maps to `sandbox-code-runner::capacity`.

## Errors

| code | meaning |
|---|---|
| `sandbox-code-runner::invalid_request` | malformed field, wrong lang, namespace violation, id already taken, `network: true` with no runtime to honor it, `teardown` given both or neither of `runtime_id`/`namespace` |
| `sandbox-code-runner::runtime_not_found` | unknown `runtime_id`, or a `namespace` teardown naming one with no live runtime |
| `sandbox-code-runner::expired` | the runtime's idle VM was reaped; retry the call that discovered it (a fresh `run`, or `register_function` again) |
| `sandbox-code-runner::capacity` | the daemon refused a new sandbox (its concurrency/image caps) |
| `sandbox-code-runner::timeout` | the run or call blew its deadline |
| `sandbox-code-runner::handler_error` | the handler threw, or returned non-JSON-serializable data |
| `sandbox-code-runner::engine` | anything else from the bus or daemon, diagnostic passed through |
