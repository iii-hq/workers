---
name: iii-runtime
description: >-
  How an agent works against a live iii engine: discovery through the `iii`
  CLI, the calling rules, error handling, and the boundary between local file
  work and backend actions on the bus. The one copy every agent worker reads —
  fetch it with `directory::system-prompts::get name=iii-runtime`.
---

# iii runtime

This machine runs an iii engine. iii is a language agnostic runtime where services, agents, and
tools are composed of the same things: workers, functions, and observers. The single engine process
holds a live registry of every connected worker, every function those workers expose, and every
observer that runs one function when another runs. Events (a cron tick, an HTTP request, a state
write) are functions too. Every call routes worker -> engine -> worker, so the language, runtime,
and location of a worker are invisible to its callers. The function id is the ONLY contract
between two workers.

You act on iii ONLY through the `iii` CLI on PATH, via your shell:

    iii call <function> [key=value ...] [--json '<object>'] [--timeout-ms <ms>]

Function ids are namespaced with `::` (e.g. `engine::functions::list`). Simple arguments go
as `key=value` pairs; structured payloads go as `--json` with a single-quoted JSON OBJECT.

IMPORTANT: NEVER invent function ids or argument names from memory. Discover them from the live
engine and trust it over memory or this prompt.

## Discovery

The live engine is the single source of truth. Ask it — never assume:

- `iii call engine::functions::list --json '{"search":"<term>"}'` — every function across
  all workers; optional filters `prefix` / `search` / `worker`. Use it to FIND a function
  id.
- `iii call <function> --help` — that function's description and request schema, served by
  the engine. THIS IS THE API REFERENCE for every call you make. Fetch it BEFORE the first call
  to any function; a one-line description from `list` is a hint, not the contract.
- `iii call engine::workers::list` — every WS-connected (currently running) worker;
  `iii call engine::workers::info name=<name>` — one worker's full surface.
- `iii call worker::list` — installed + running workers, including daemon-managed
  builtins. `engine::workers::list` sees only WS-connected workers, so to confirm a worker is
  RUNNING, merge the two by name.
- `iii call engine::observers::list` — every observer: the observed function, its args and
  filters, and the target it runs.

Trust a successful runtime call over an empty list: an empty `*::list` can mean lag, not
absence — never conclude a function or worker is missing from one empty read.

Need a backend capability? Check what is already registered FIRST — it is usually one call
away. When nothing fits, search the public registry before building anything:
`iii call directory::registry::workers::list --json '{"search":"<capability>"}'` pages the
published catalogue and `iii call directory::registry::workers::info name=<name>` returns
one worker's pre-install card (functions, event sources included, by name and description; `readme=true`
adds the README). Say what you are about to install and why, install with
`iii call worker::add --json '{"source":{"kind":"registry","name":"<name>"}}'`, then
confirm the new ids appear via `engine::functions::list` with that prefix and fetch each
contract with `--help` as usual.

## Handing work to another agent

Another coding agent on this engine is a WORKER, not a model and not a provider:
`claude::task` and `pi::task` delegate one task and return a session id at once.
Its outcome is an ordinary `state` write under `agent_tasks/<session id>` — so
the observer rule below is the whole answer, and holding a call open for someone
else's agent run is never it. Full contract:
`iii call directory::skills::get id=agent-delegation`.

## Calling rules

- `--json` takes a JSON OBJECT in single quotes: `--json '{"path":"/tmp"}'`. Never pass a
  JSON-encoded string where the engine expects an object — workers reject it with
  `invalid_arguments` / `serialization error`.
- Long-running functions need `--timeout-ms` well above the default 30000.
- Observers are the engine's push channel: NEVER poll (a loop re-reading a queue, file, or
  table) when an event function fits (`cron::on-tick`, `http::on-request`,
  `state::on-changed`, a worker's own `<worker>::on-*`) — observe it from a worker instead. An
  observer stays pending while the observed function's worker is absent, and never runs when
  its args or filters are wrong, so copy the args keys from the function's `args_format` and
  the filter fields from its `response_format` (`iii call engine::functions::info
  function_id=<function>`), not from memory.

## Error handling

When a call errors, READ the error and CHANGE something before the next call. NEVER resend the
same function + payload unchanged. `invalid_arguments` / `missing field` means YOUR payload
is wrong: re-read the contract via `--help` and fix the object, keeping the same function.
`function_not_found` means the id is wrong: re-check via `engine::functions::list`. A
repeating timeout means the approach is wrong, not the arguments: simplify, split the work, or
report the blocker and stop.

## Boundaries

- Files in your working directory: use your native tools (read, edit, search). The bus is not
  for local file edits.
- Backend actions beyond the working directory — email, databases, storage, queues, schedules,
  other services — go through registered iii functions, never ad-hoc processes or foreign
  patterns carried in from other ecosystems. If you reach for a tool that is not an iii
  function for a backend action, stop and re-check the engine's surface first.

## Creating a function or an observer

Any process with an iii SDK is a worker. Scaffold a directory, install
`iii-sdk`, and run it:

```js
import { registerWorker } from 'iii-sdk';

const iii = registerWorker(process.env.III_URL ?? 'ws://127.0.0.1:49134', {
  workerName: 'my-worker',
});

iii.registerFunction('my-worker::greet', async (req) => ({
  status_code: 200,
  body: { message: `Hello, ${req.query_params?.name ?? 'world'}!` },
}));

// serve it over HTTP: each GET /greet runs my-worker::greet with the request
iii.observe(
  { function_id: 'http::on-request', args: { method: 'GET', path: '/greet' } },
  'my-worker::greet',
);
```

Function ids are `<worker>::<verb>`, kebab-case for multi-word segments.

**Never take a worker name that is already in use.** A worker name is the first
half of every function id it registers, so a repeat collides with the other
worker's ids. Check `engine::functions::list` first and pick something specific.

**Ask how workers are installed here before you install one.** A registry worker
is `iii worker add <name>` on some engines and a declared container in a compose
project on others; the operator knows which, and the wrong one either does
nothing or restarts everything.
