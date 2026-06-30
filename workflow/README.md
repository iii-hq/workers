# workflow worker

A deterministic DAG orchestrator running over the iii harness. Accepts
`workflow::start` / `workflow::tick` / `workflow::stop` triggers; persists run
state in iii-state; fans out nodes as child harness sessions with deterministic
ids so duplicate deliveries are idempotent.

---

## Getting the result back (don't poll)

`workflow::start` returns a `run_id` immediately. There are a few ways to learn
the outcome; prefer push over polling `workflow::status` in a loop.

0. **Reply into your session (for an AGENT caller).** Set `reply_to: {}` on
   `workflow::start`. When the run reaches a terminal state the worker posts the
   outcome into your session as a new message (via `harness::send`), so your next
   turn just sees it — no polling, no callback wiring:

   ```jsonc
   workflow::start {
     "definition": { /* ... */ },
     "reply_to": { "template": "Naming run done — pick the winner:" }  // template optional
   }
   ```

   You pass NO session id: a `pre_trigger` hook auto-stamps your real
   `session_id` / `model` / `provider` (an agent cannot target another session).
   Delivery is at-least-once; a deterministic `idempotency_key` (`wfreply_<run_id>`)
   makes a re-fire a no-op.

1. **Completion callback (push), for a WORKER caller.** Pass `notify` to
   `workflow::start`. When the run reaches a terminal state the worker triggers
   your function once with `{run_id, status, result, result_error}`:

   ```jsonc
   workflow::start {
     "definition": { /* ... */ },
     "input": { /* ... */ },
     "notify": { "function_id": "myworker::wf-done" }  // optional "queue", defaults to "default"
   }
   ```

   Delivery is durable (enqueued) and **at-least-once** — make the handler
   idempotent and dedup on `run_id`. This is the trigger-driven path; no polling.

2. **Global broadcast.** Every terminal run also fires `workflow::run-completed`
   (untargeted). Bind a worker to it to observe *all* runs, e.g. for audit.

`workflow::status` remains for one-off inspection, but a live tracker should use
`reply_to` (agents) or `notify` (workers) rather than a status loop.

---

## Scaling & concurrency

### Single-writer-per-run (the correctness model)

Every path that reads, mutates, and writes a run record — `workflow::tick`,
`reconcile`, the pending-timeout sweep, and `workflow::stop` — holds the
per-run `WorkflowLocks` guard before touching state.  This serialises writes
within one process and closes the read-modify-write race.

Idempotency for duplicate event deliveries is provided by deterministic ids:
the child session id (`wf_<run_id>_<node_uid>@r<attempt>`) and the
`workflow_node_result/<run_id>/<node_uid>` key are stable, so a re-delivered
tick that re-fires an already-running node is a no-op at the harness layer.

### Horizontal scaling (run sharding)

To run more than one workflow-worker instance safely, shard `workflow::tick`
events by `run_id` so that all writes for a given run land on one owning
instance.  Concretely: consistent-hash the queue partition on `run_id`, or run
a single orchestrator instance.  No new code is needed — this is a deployment
topology choice.

### No record-level CAS available

iii-state (sdk 0.19.2) exposes `state::get`, `state::set`, `state::delete`,
`state::list`, and `state::update` (atomic per-field ops).  None carry a
precondition or version check — `state::set` is an unconditional overwrite.
There is no `state::cas` or `if_match` parameter.

`WorkflowRunRecord` therefore carries no `version` field; pretending one
enforces optimistic concurrency would be misleading.

### Blocked prerequisite for true multi-writer-per-run

Whole-record optimistic concurrency (allowing two instances to safely race on
the same run) requires an upstream iii engine operation: a `state::set` with an
`expected`/`if_match` argument, or a dedicated `state::cas` call.  Until that
engine op exists, the correct stance is run sharding (above).

**Action:** file an engine feature request for `state::set` with `if_match` /
`state::cas`.  Do not build a fake CAS on top of the current unconditional set.
