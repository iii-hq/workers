---
name: iii-directory-for-mcp-agents
description: >-
  Rules for agents that reach the iii engine through the mcp-server worker
  (Claude Code, Codex and other MCP clients), where their usual habits differ
  from how iii works: settings, waiting for changes, credentials, addresses,
  starting processes and function ids. Preloaded by the default-mcp profile.
type: how-to
---

# iii, for MCP agents

You reach the engine through mcp-server, with the `iii_*` tools. Some habits
that work elsewhere do not work in iii. Follow these rules unless the user asks
for something else. ADE-specific rules (pages, browser APIs, page URLs) are in
the `ade/for-mcp-agents` skill.

## Settings live in the engine, not in files

Worker settings are configuration entries. Read them with `configuration::get`
and change them with `configuration::set`. Change Compose containers with
`compose::add`, or `ade::compose::edit` for one container's path, run script,
`start_after`, environment or `config_override`. Do not edit
`worker-compose.yaml` or `config/*.yaml` by hand.

Why: the Compose daemon keeps its declarations in memory, so a hand edit does
not reach a running project until the whole project restarts.

## Observe, do not poll

Do not wait for a change with sleep or retry loops, or with `setInterval`.
Observe the function that carries the change with `iii_observe`, and read
its observations with `iii_observations_poll`, or hand each one to a function
with `target_function_id`. To stream observations instead, create the observer
with `iii_observations_sse` instead of `iii_observe`: it is a separate observer,
not a stream of an existing one. To wait for a time, observe `cron::on-tick` with
args `{ expression }`. Remove each observer with `iii_unobserve` when you are
done.

Every observer these tools create is removed when the MCP session ends, even one
with `target_function_id`. A schedule or reaction that must keep running after
the session belongs in a worker you control, which registers it with the
iii-sdk.

Why: the engine pushes each event the moment it happens. A loop misses events
between checks, wastes calls, and keeps running after you stop.

## Credentials never go in a payload or a committed file

Store a credential with `iii_secret_store`. Use `secrets::import` for a value
already in `.env` or the environment. Then pass the `secret://NAME` reference
it returns. Never put a credential in an `iii_call` payload or a committed
`.env`. Register a function that receives or returns a credential with
`observable: false`.

Why: an observable function's payloads go to its observers and are recorded
in stored traces (the span events `iii.invocation.input` and
`iii.invocation.output`), and nothing redacts them.

## Never assume ports or addresses

Read them. `ade::status` gives the ADE's `url`, `http::status` the public HTTP
server's, and the project's compose file the engine's URL.

Why: several projects can run on one machine, each on its own ports. A default
port can point at another project.

## Do not start iii processes from a worker's shell

A process started from a worker's shell inherits that worker's `III_*`
environment: `III_URL`, `III_CONFIG_NAME` and `III_COMPOSE_*`. Run workers
through the Compose functions (`compose::add`, `compose::restart`). If you
must start a process yourself, clear those variables first.

Why: with the inherited variables, the new process joins the wrong engine or
reads another worker's configuration entry.

## Function ids are discovered, never recalled

Find an id with `directory::search_functions`, read its contract with
`engine::functions::info`, then call it with `iii_call`.

Why: a plausible id from memory is usually wrong, or has a different payload
than you remember. The live engine is the only contract.
