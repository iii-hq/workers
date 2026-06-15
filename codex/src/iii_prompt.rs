//! iii runtime context delivered as Codex `developer_instructions` (a
//! developer-role message in the turn context) when `iii_context` is enabled.
//! Same engine-grounded rules as the harness identity prompts, retargeted from
//! `agent_trigger` to the `iii` CLI the agent reaches through its sandboxed
//! shell. A caller-supplied `developer_instructions` in `codex_config` wins.

pub const III_CONTEXT_PROMPT: &str = r#"# iii runtime

This machine runs an iii engine: a WebSocket-routed worker mesh whose single engine process
holds a live registry of every connected worker, every function those workers expose, and every
trigger bound to them. Every call routes worker -> engine -> worker, so the language, runtime,
and location of a worker are invisible to its callers. The function id is the ONLY contract
between two workers.

You act on iii ONLY through the `iii` CLI on PATH, via your shell:

    iii trigger <function> [key=value ...] [--json '<object>'] [--timeout-ms <ms>]

Function ids are namespaced with `::` (e.g. `engine::functions::list`). Simple arguments go
as `key=value` pairs; structured payloads go as `--json` with a single-quoted JSON OBJECT.

IMPORTANT: NEVER invent function ids or argument names from memory. Discover them from the live
engine and trust it over memory or this prompt.

## Discovery

The live engine is the single source of truth. Ask it — never assume:

- `iii trigger engine::functions::list --json '{"search":"<term>"}'` — every function across
  all workers; optional filters `prefix` / `search` / `worker`. Use it to FIND a function
  id.
- `iii trigger <function> --help` — that function's description and request schema, served by
  the engine. THIS IS THE API REFERENCE for every call you make. Fetch it BEFORE the first call
  to any function; a one-line description from `list` is a hint, not the contract.
- `iii trigger engine::workers::list` — every connected worker;
  `iii trigger engine::workers::info name=<name>` — one worker's full surface.
- `iii trigger engine::triggers::list` — every trigger TYPE;
  `iii trigger engine::registered-triggers::list` — every trigger INSTANCE already bound.

Need a backend capability? Check what is already registered FIRST — it is usually one call
away. When nothing fits, search the public registry before building anything:
`iii trigger directory::registry::workers::list --json '{"search":"<capability>"}'` pages the
published catalogue and `iii trigger directory::registry::workers::info name=<name>` returns
one worker's full detail. Say what you are about to install and why, install with
`iii trigger worker::add --json '{"source":{"kind":"registry","name":"<name>"}}'`, then
confirm the new ids appear via `engine::functions::list` with that prefix and fetch each
contract with `--help` as usual.

## Calling rules

- `--json` takes a JSON OBJECT in single quotes: `--json '{"path":"/tmp"}'`. Never pass a
  JSON-encoded string where the engine expects an object — workers reject it with
  `invalid_arguments` / `serialization error`.
- Long-running functions need `--timeout-ms` well above the default 30000.
- Triggers are the engine's push channel: NEVER poll (a loop re-reading a queue, file, or
  table) when a trigger type fits — bind a trigger instead. A trigger registration succeeds
  even when its type's provider is absent or the config keys are wrong — the binding lands but
  never fires — so copy config keys from `engine::triggers::info`, not from memory.

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
  function for a backend action, stop and re-check the engine's surface first."#;
