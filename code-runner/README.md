# code-runner

Run Node.js and Python in isolated microVMs: iterate on code with
`code-runner::eval`, publish working functions to the bus with
`code-runner::register_function`, clean up with `code-runner::teardown`.

code-runner delegates every execution to the
[iii-sandbox daemon](https://workers.iii.dev/workers/iii-sandbox)
(`sandbox::*` triggers) rather than running an in-process interpreter — you
get Python, npm/pip, and a real OS per call, at the cost of heavier runtimes.
code-runner itself executes nothing and touches no host filesystem.

## Install

```bash
iii worker add code-runner
```

code-runner delegates every execution to the iii-sandbox daemon, so install
that too:

```bash
iii worker add iii-sandbox
```

Missing iii-sandbox is not fatal — code-runner warns loudly at boot and
keeps serving; every call fails with a clear message until you add it.

## Prerequisites

- Hardware virtualization on the engine host (`/dev/kvm` on Linux, Apple
  Silicon on macOS) — the iii-sandbox daemon's requirement, inherited.

## Evaluating code

`eval` is **one-shot by default**: it boots a VM, runs your code, returns
the result, and destroys the VM before the response is even sent. Nothing
persists — no files, no installed packages — and the response carries no
`runtime_id`, because there is nothing left to address.

```bash
iii trigger code-runner::eval lang=python code='print(2+2)'
# → { "stdout": "4\n", "stderr": "", "exit_code": 0, "success": true, "duration_ms": … }
```

Pass `keep=true` to leave the VM running instead. The response's
`runtime_id` then addresses it — treat it as a secret — and is the
capability `code-runner::teardown` needs to stop it:

```bash
iii trigger code-runner::eval lang=python code='print(2+2)' keep=true
# → { "runtime_id": "rt-…", "stdout": "4\n", … }

# run more code in the SAME runtime (same filesystem, fresh process)
iii trigger code-runner::eval runtime_id=rt-… code='print(open("/tmp/x").read())'
```

Passing `runtime_id` back in a later eval reuses that VM: **variables do
not survive between evals; files and installed packages do.** A runtime
you hold via `runtime_id` is never auto-stopped — you own it until you
tear it down (`code-runner::teardown`) or its idle TTL reaps it. A failing
script is a response (`success: false`, `stderr`), not an error — errors
are reserved for infrastructure (unknown runtime, expired VM, timeouts,
capacity).

**`network` needs an existing runtime.** Outbound network
(`npm install` / `pip install`) can only ever be enabled on a runtime's
*own* creation — and neither the one-shot path nor `keep: true` can create
one with network, because both run through the daemon's `sandbox::run`,
which has no network flag at all. Passing `network: true` without an
explicit `runtime_id` is therefore refused outright (`invalid_request`),
not silently ignored. `network: true` is still accepted, and still
ignored, when reusing an existing runtime by `runtime_id` — that runtime's
network was fixed when it was created.

## Registering a function

`register_function` needs **no `runtime_id`**. Give it `function_id`,
`source`, `lang`, and an optional `description`; code-runner keeps one
persistent runtime per **namespace** (the first segment of the id —
`app::greet` claims `app::`) and language, creating it on the namespace's
first registration and reusing it for every later one in the same
namespace and `lang`:

```bash
iii trigger code-runner::register_function \
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
`default_timeout_ms`; anything the handler prints goes to code-runner's
debug log, and the caller receives exactly what `handler` returned,
JSON-serialized.

The runtime backing a namespace is entirely an implementation detail — you
never see or manage its `runtime_id`. It carries no network access (there
is no `network` field on this request either).

## Teardown and expiry

Pass **exactly one** of `runtime_id` (a kept eval's runtime) or
`namespace` (every runtime — one per language — backing a
`register_function` namespace):

```bash
iii trigger code-runner::teardown runtime_id=rt-…
# → { "runtime_id": "rt-…", "torn_down": true, "unregistered": [] }

iii trigger code-runner::teardown namespace=my-app
# → { "namespace": "my-app::", "torn_down": true, "unregistered": ["my-app::double"] }
```

Passing both, or neither, is refused (`invalid_request`) with a message
naming which one to use. Tearing down a namespace destroys every runtime
backing it (one per language it was used in) and unregisters every
function any of them had published — exactly as tearing down a single
kept-eval runtime unregisters that runtime's functions.

Idle runtimes are reaped by the iii-sandbox daemon after `idle_ttl_secs`
(default 900; any eval or call resets the clock). A reaped runtime
surfaces as `code-runner::expired` on its next use, its bus functions are
unregistered, and its id is forgotten — for a kept-eval runtime, eval again
with `keep: true` (or one-shot, if persistence is no longer needed) to
boot a fresh one; for a namespace runtime, the next `register_function` in
that namespace boots a fresh one automatically. There is no auto-respawn: a
revived VM would have lost its filesystem (installed packages included),
and a half-working function is worse than an honest error.

**Unregistration is lazy.** A reaped runtime's bus functions are not
removed the moment the TTL passes — they are unregistered only when
something next calls into the dead runtime and gets the `expired` outcome
above. Until then, a stale runtime's functions still show up in the
catalog (e.g. `engine::functions::info`) even though they can no longer be
invoked.

**Restarting code-runner also invalidates every outstanding `runtime_id`**
and every namespace binding — all state is in-process — but with a
**different** error: `code-runner::runtime_not_found`, not `expired`,
since the new process has no record of the old id at all (and a
`namespace` teardown against a namespace with no live runtime gets the
same code). The orphaned microVM itself is unaffected by the restart and
keeps running in the iii-sandbox daemon until its own idle TTL reaps it;
there is no drain-on-shutdown.

## Configuration

```yaml
default_timeout_ms: 5000  # per eval and per handler invocation when unspecified
max_timeout_ms: 30000     # ceiling a request's timeout_ms is clamped to
idle_ttl_secs: 900        # passed to sandbox::create; the daemon reaps idle VMs
```

Images (`node`, `python`), CPU/memory caps, sandbox concurrency, and the
image allowlist are the iii-sandbox daemon's configuration — code-runner
deliberately duplicates none of them; a daemon-side refusal (e.g. capacity)
maps to `code-runner::capacity`.

## Errors

| code | meaning |
|---|---|
| `code-runner::invalid_request` | malformed field, wrong lang, namespace violation, id already taken, `network: true` with no runtime to honor it, `teardown` given both or neither of `runtime_id`/`namespace` |
| `code-runner::runtime_not_found` | unknown `runtime_id`, or a `namespace` teardown naming one with no live runtime |
| `code-runner::expired` | the runtime's idle VM was reaped; retry the call that discovered it (a fresh `eval`, or `register_function` again) |
| `code-runner::capacity` | the daemon refused a new sandbox (its concurrency/image caps) |
| `code-runner::timeout` | the eval or call blew its deadline |
| `code-runner::handler_error` | the handler threw, or returned non-JSON-serializable data |
| `code-runner::engine` | anything else from the bus or daemon, diagnostic passed through |
