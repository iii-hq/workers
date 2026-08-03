# Orchestrating a fan-out run (opt-in guidance)

This is **not** part of any system prompt. The built-in and provider identity
prompts teach the tools and nothing else; how to structure a multi-agent run
is the operator's choice. To use this guidance, paste it (or the parts that
fit) into the task prompt you give a top-level agent, or pass it via
`harness::send` / `harness::spawn` `options.system_prompt` (the default
`enrich` strategy appends it to the built-in identity). A worker could also
serve it mechanically through the `harness::hook::pre-generate` hook, the way
`web::inject-guidance` serves its own guidance — the harness itself ships no
such hook.

The invariant this pattern lives by:

> The parent chooses and initializes a shared-state medium, registers
> notifications, and directly spawns workers. Workers only perform their
> assignments and update that medium. Changes to the medium notify the parent
> through generic triggers. Triggers never decide to spawn workers.

## 1. Own the run

You — the top-level agent — own the whole control plane. Pick ONE shared
medium for this run and initialize it before anything starts:

- a **state scope** named after the run (keys written with `state::set`),
- a **database table** (rows via `database::execute`),
- or any other trigger-capable medium the deployment offers.

Pin the medium's exact name once and use that identical string everywhere: in
every worker task and in every binding config. A gate watching `results_v2`
while workers write `results` never fires.

## 2. Arm notifications first

Register the wakes before any producer starts — events do not replay, so a
write that lands before its binding is armed is lost to that binding.

- **Completion**: one wake on the medium's change stream. Define completion
  YOUR way, per medium: a `state::barrier` condition over the N expected keys
  (skips until every arrival is in, then wakes you once with all of them), a
  wake per `database::row-changed` event that you count yourself, or a
  completion key the last writer sets.
- **Deadline**: a `timer` wake (`{ "in_ms": <ms> }`) so silence becomes a
  notification instead of an eternal park. A `lifecycle.expires_at` on the
  completion wake does the same from the other side — an expired unfired wake
  wakes you with a notice.

## 3. Spawn workers directly

`harness::spawn` each worker yourself — no binding ever starts an agent. Each
task must be fully self-contained: the exact inputs inline, the exact medium
destination to write, and the status vocabulary this run uses (e.g. write
`{ "status": "pending" | "running" | "done" | "error", ... }` — the format is
YOURS to define; the platform does not prescribe one). Workers are LEAF
agents by capability: they cannot spawn, register triggers, or message
sessions. Pass `options: { orchestrator: true }` only for a sub-coordinator,
and hand it this same discipline in its task.

## 4. Wake, decide, act

Your bindings wake you with the medium's events. On each wake you hold the
full control plane again: read the medium, judge progress, respawn a failed
item, extend the deadline, or finish. Worker failures reach you through the
medium (an `error` status the worker managed to record) or through your
deadline catching the silence — nothing is injected into your session on a
worker's behalf. `harness::status { session_id: <child> }` answers a direct
health question about a specific worker.

## 5. Tear down

Standing bindings outlive the run until you unregister them. At the end,
`engine::unregister_trigger { id }` everything you registered, and clear the
run's medium if the deployment expects it. Give any deliberately-standing
binding a `lifecycle` bound up front so a forgotten teardown cannot fire
forever.
