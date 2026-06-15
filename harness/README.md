# The iii harness

**The harness is not a layer on top of your backend. On iii, it is the backend.**

Many setups keep the agent loop in one process and everything else (queues, HTTP, state, traces) in another. Tool calls cross that boundary; retries and traces rarely line up.

On iii, agents are workers. Tools are functions. Handoffs use the same triggers and queues as the rest of the system.

This package is the production harness for that model: turn orchestration, approvals, sessions, providers, context compaction, and budgets, all as iii workers next to shell, storage, database, and whatever you add.

Read [The Harness Is the Backend](https://www.linkedin.com/pulse/harness-backend-mike-piccolo-2aocf/) by Mike Piccolo for the full argument.

---

## What you get

**One trace.** Each hop is a `trigger()` on the bus. Trace IDs propagate across workers, languages, and queue steps. You debug one runtime, not separate logs aligned by timestamp.

**Live discovery.** Workers register functions on connect; the engine keeps a catalog. Agents and the console see what the system can do today, including workers added without redeploying the orchestrator. Providers self-register; the model catalog fills from discovery, not a hardcoded seed.

**Composition, not frameworks.** Thin vs thick harnesses map to how many functions you register and how you wire triggers. Fewer functions for a lean loop; approval rules and extra workers for more structure.

**New capability, new worker.** When the harness needs something else (shell, database, coder, another provider), you add a worker, not a fork of the orchestrator. Published workers install from the [iii worker registry](https://workers.iii.dev) with `iii worker add <name>`; they register on the iii engine and show up in the live catalog.

**Turns, approvals, budgets.** Seven-state durable turn FSM with queue-backed steps. A `harness::hook::pre-dispatch` trigger type gates every agent function call — the standalone [approval-gate worker](../approval-gate/) binds its policy there (permission modes, YAML rules, pending inbox) and settles holds via `harness::function::resolve`; the chain fails closed when a bound hook is unreachable. Parallel tool batches, pending state across reload. Workspace and agent budget caps. Five provider workers behind one registry.

**Context compaction.** Long sessions exceed model windows. The `context-compaction` worker compacts history as turns accumulate and backs the console `/compact` command.

---

## What ships here

One folder per worker, one feature per file:

| Concern | Workers |
| --- | --- |
| Orchestration | `turn-orchestrator` (durable turn FSM, pre_dispatch hook point, `harness::function::resolve`), `hook-fanout` |
| Governance | `harness` (yaml policy rules, UI fanout) |
| Context | `context-compaction` (keeps long sessions inside the model window) |
| Models | `models-catalog`, `provider-anthropic`, `provider-openai`, `provider-kimi`, `provider-lmstudio`, `provider-llamacpp` |
| Cost | `llm-budget` |

Rust workers (`shell`, `iii-directory`, `session-manager` — the durable, reactive conversation store the harness drives through `session::*`, and `approval-gate` — the standalone approval policy worker at the repo root) and engine builtins (`state::*`, `stream::*`, `iii::durable::*`) stay on the same bus; this package does not reimplement them.

---

## Quickstart

1. Install iii: `curl -fsSL https://install.iii.dev/iii/main/install.sh | sh`
2. Verify the install: `iii --version`
3. Add the harness, session-manager, and console workers: `iii worker add harness session-manager console coder`
4. Start the engine: `iii --config config.yaml`
5. Open the [console](https://workers.iii.dev/workers/console) at `http://127.0.0.1:3113`

Chat, approve/deny, model picker, and trace explorer ship in one binary ([console](https://workers.iii.dev/workers/console)).

Tools, orchestration, governance, and observability use the same worker, trigger, and function model as the rest of iii.

---

## Further reading

- [The Harness Is the Backend](https://www.linkedin.com/pulse/harness-backend-mike-piccolo-2aocf/)
- [console worker](https://workers.iii.dev/workers/console)
- [iii worker registry](https://workers.iii.dev)
- [iii engine](https://github.com/iii-hq/iii)
