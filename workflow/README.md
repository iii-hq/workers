# workflow worker

A deterministic DAG orchestrator running over the iii harness. Accepts
`workflow::start` / `workflow::tick` / `workflow::stop` calls; persists run
state in state; fans out nodes as child harness sessions with deterministic
ids so duplicate deliveries are idempotent.

The always-on harness observers (`harness::on-turn-completed` → wake,
`harness::hook::on-pre-call` → stamp-reply, `harness::hook::on-post-turn` →
validate-result) are created once at startup. An
observer created before the harness is up stays `pending` and becomes active
when the harness registers its function, and again after a harness restart.
`workflow::start` is registered only after the stamp-reply observer is active,
or after 10 s while the harness is absent, so an agent's call cannot reach it
unstamped while the worker boots.
The last — `harness::hook::on-pre-generate` → inject-guidance — follows the
`inject_guidance` knob in the `workflow` configuration entry (ON by default):
turning it off shrinks prompts (the harness's `# Granted functions` catalog
still advertises the `workflow::*` ids) and the worker observes or disposes it
hot, no restart.

---

## Getting the result back (don't poll)

`workflow::start` returns a `run_id` immediately. There are a few ways to learn
the outcome; prefer push over polling `workflow::status` in a loop.

0. **Observe the run (for an AGENT caller).** For a call from an agent turn,
   `workflow::start` returns `{run_id, next}`. `next` is the
   `harness::observe` call that wakes your session when the run ends. Make it
   exactly as given, then end your turn:

   ```jsonc
   // workflow::start returns
   {
     "run_id": "r_…",
     "next": {
       "call": "harness::observe",
       "arguments": {
         "function_id": "workflow::on-run-completed",
         "args": { "run_id": "r_…" },
         "label": "workflow r_…",
         "lifecycle": { "expires_in_ms": 82800000 }
       },
       "then": "END YOUR TURN after this call; …"
     }
   }
   ```

   The outcome arrives as `[notification] workflow r_…: {…}`, at once if the
   run already ended: the worker replays a finished run to an observer of it.
   The wake expires an hour before the run is collected (24 h after it ended),
   so an expired wake can observe again and still get the replay; a collected
   run is refused with `UNKNOWN_RUN`. A notification cut at the harness's
   8,000-character limit is incomplete: call `workflow::status` once for the
   run. `next` is present only when the pre-call hook stamped your session; you
   pass no session id. A session whose policy narrows `options.functions` must
   allow `harness::observe` and `workflow::on-run-completed`.

   `reply_to` was removed: a start that passes it is refused with this recipe.

1. **Completion callback (push), for a WORKER caller.** Pass `notify` to
   `workflow::start`. When the run reaches a terminal state the worker calls
   your function once with `{run_id, status, result, result_error, failed_nodes?}`
   (`failed_nodes` is omitted when no node failed; see [Events](#events)):

   ```jsonc
   workflow::start {
     "nodes": { /* ... */ },
     "input": { /* ... */ },
     "notify": { "function_id": "myworker::wf-done" }  // optional "queue", defaults to "workflow-notify"
   }
   ```

   Delivery is durable (enqueued) and **at-least-once** — make the handler
   idempotent and dedup on `run_id`. This is the push path; no polling. The
   worker defines its `workflow-notify` queue at boot, apart from the `workflow`
   tick queue, so a slow callback cannot delay ticks; a custom `queue` must
   already be defined with `queue::define`.

2. **Observe every run.** Every terminal run also calls
   `workflow::on-run-completed`. Observe it without args to see *all* runs,
   e.g. for audit (see [Events](#events)); to wait for one run, pass args
   `{ run_id }` instead.

   ```ts
   iii.observe(
     { function_id: 'workflow::on-run-completed', filters: { 'input.status': { $eq: 'failed' } }, capture: ['output'] },
     'audit::workflow-failed',
   )
   ```

`workflow::status` remains for one-off inspection, but a live tracker should
observe the run (agents: the `next` call) or pass `notify` (workers) rather than
run a status loop.

---

## The request

`workflow::start` takes one level of nodes:

```jsonc
{"input": {"topic": "durable workflows"},
 "nodes": {
   "research": {"task": "List the key findings on the topic.",
                "schema": {"type": "object", "properties": {"findings": {"type": "array", "items": {"type": "string"}}},
                           "required": ["findings"]}},
   "report":   {"reads": "research", "task": "Write a one-page report from these findings."}}}
```

The worker refuses an unknown key at either level, and lists every shape
problem of a request in one refusal. A key whose value is `null` counts as
omitted. The removed shape (`definition`, `agent`, `input.from`, `depends_on`,
`fanout.over`, `output.from`, the `node:` prefix) is refused with the recipe
for each moved key.

| Top-level key | Default | Meaning |
|---|---|---|
| `nodes` | required | One sub-agent session per node, keyed by node id. An id has no `#`, `.` or `/`. |
| `input` | `null` | Any JSON. Every node without `reads` or `fanout` receives it. |
| `output` | every unread node | A node id returns that result as is; an array of ids returns an object keyed by id. An unread node is one no other node names in `reads` or `fanout`: one returns its result as is, several an object keyed by id. A loop node or a loop body node returns its last pass; see [Loops](#loops). |
| `max_concurrency` | no cap | Live node sessions at once (running, or reserved for a send). At least 1. |
| `default_functions` | everything except `workflow::*`, `configuration::*`, `approval::*`, `harness::hook::*` and `harness::send` | Dispatch policy of every node without `functions`. |
| `idempotency_key`, `notify` | none | See [Getting the result back](#getting-the-result-back-dont-poll). |

| Node key | Default | Meaning |
|---|---|---|
| `task` | required | The node's opening message. Its input follows as JSON, fenced as untrusted data. |
| `reads` | the run `input` | A node id delivers that result as is (a text node as a JSON string, a fanout node as the array of its copies' results); an array of ids delivers an object keyed by id. The node runs after them. |
| `fanout` | none | One copy per element of an array; see [Fanout](#fanout). |
| `schema` | text node | JSON Schema, with a top-level `type`, of the JSON value the node delivers. Required on a node a `fanout` path points at and on a loop node. |
| `model` | the caller's model | A model id from `router::models::list`; an unregistered id is refused at start. |
| `provider` | the routed provider | Only with `model` or `profile`. |
| `profile` | none | Agent profile id (lowercase letters, digits, `-` and `_`, at most 64 characters) sent as the harness `options.agent`. It excludes `system_prompt`, and a model it pins wins over `model`. |
| `system_prompt` | none | Appended to the child's built-in identity prompt. |
| `functions` | `default_functions` | The node session's dispatch policy: an allow-list array or `{ "allow", "deny" }`. List a denied id to give it back; a node that starts a sub-run lists both `workflow::start` and `workflow::on-run-completed`. |
| `filesystem_root` | the caller's folder | Absolute folder the node's session runs in. It must lie inside the caller's folder with symlinks resolved, checked once at start on the worker's host; a run without a caller folder cannot set it. Fanout copies and loop passes share it. |
| `timeout_ms` | `default_pending_timeout_ms` (30 min) | Wall clock an attempt may run, from its send. At least 1000. |
| `retries` | `max_node_retries` (1) | Refires after a timeout, each in a new `@r<n>` session. At most 10. A failed turn is never retried. |
| `on_error` | `"fail"` | `"continue"`: a failed or cancelled node counts as finished with a `null` result, so its readers run and the run can complete. The error stays in `node_errors`. |
| `label` | none | Session title (fanout copies get ` #<i>`, loop passes ` #it<k>`) and `node_details.label`. 1 to 80 characters. |
| `phase` | none | Group counted in `workflow::status` `phases`. 1 to 80 characters. |
| `loop` | none | Repeat this node and its `body` while a field of its result holds; see [Loops](#loops). |

**The model.** A node that omits `model` and `provider` runs on the caller's
model, read once at start from the caller session's `harness::status`
context and written into the stored definition, so retries, loop passes and
fanout copies stay on it. It also takes the caller's provider, unless it has a
`profile`. With no caller session, or an unreadable model, a node without a
`profile` is refused; a `profile` node then runs on the profile's pinned
model. With `output` omitted, every unread node is required: a failed side
node fails the run unless it sets `on_error: "continue"`.

### Fanout

A fanout node runs one copy per element of an array.

| `fanout` | The array | Copies start |
|---|---|---|
| `"plan.items"` | The `items` array of `plan`'s JSON result, read once. | When `plan` and every node in `reads` are settled. |
| `["security","cost"]` | The array as written. | When every node in `reads` is settled; without `reads`, at once. |

| The node has | A copy receives |
|---|---|
| `fanout` only | its element |
| `fanout` and `reads: "draft"` | `{"input": <draft's result>, "fanout_item": <its element>}` |
| `fanout` and `reads: ["draft","spec"]` | `{"input": {"draft": …, "spec": …}, "fanout_item": <its element>}` |

`input` is what a plain node with the same `reads` receives. A copy never
receives the run `input`. The node's result, for its readers and `output`, is
the array of its copies' results in element order (the order written, for a
written array); a failed copy of an `on_error: "continue"` node reads as
`null`. A fanout cannot iterate another fanout's result: read it with a join
node that returns one array.

Several critics, each with an angle, over one draft:

```jsonc
{"input": {"feature": "..."},
 "nodes": {
   "draft":  {"task": "Write a one-page design note for the feature."},
   "critic": {"fanout": ["security", "cost", "operability"], "reads": "draft",
              "task": "Criticize the draft in `input` from the angle in `fanout_item`. List concrete problems."},
   "final":  {"reads": ["draft", "critic"], "task": "Revise the draft. Fix every valid problem in `critic`."}}}
```

Three `critic` copies start when `draft` is done; `final` receives
`{"draft": "<the draft>", "critic": ["…", "…", "…"]}` and is the run's result.

### JSON nodes

A JSON node (one with `schema`) runs under the harness's text contract, so its
session has no `submit_result` tool. Its opening message ends with the schema
and asks the node to deliver its result by calling `workflow-result::submit`
with `{"result": <value>}`; the value is checked against the schema, and an
invalid one is refused with the cause so the node calls again.
`workflow::validate-result` observes the harness's post-turn hook for node
sessions: a turn that submitted passes, and so does a final message that is
itself a valid JSON value (one fenced code block is tolerated); anything else
is denied with the cause and the schema, so the harness re-prompts the turn,
at most `max_validation_retries` times. A turn that ends with a wake armed
(`harness::status` reads `expects_wake`) passes whatever it says, so a node can
park on a status line and answer on the woken turn. When the node settles,
reconcile checks its result again and stores the parsed value. A result that
does not parse or match (a turn the hook did not check, for example while this
worker restarted) gets the same correction sent into the node's session, at
most twice per attempt; then the node fails. The failed node's error
(`node_errors`, the run's `result_error`) names the cause without the schema.
`workflow::start` rejects a schema that does not compile.

Provider-native structured output is not used for a node, even on a model that
supports it: the schema is checked after the answer, not enforced while the
model writes it. The node session's harness result (`harness::status` `result`,
`harness::on-turn-completed`, the `result` any other post-turn validator on
that session receives) is the answer text; the parsed value is only in
`workflow::status` and `workflow::node-result`.

There is no separate finish signal: with no wake armed, a final message that
parses and matches the schema settles the node. A node that may park needs a
schema with `required` fields a status line cannot satisfy. Under
`{"type": "object"}` alone, or any schema with no `required`, a status object
such as `{"status": "waiting on build"}` sent after a refused
`harness::observe` becomes the node's result.

## Loops

A `loop` on a node (the loop node) runs it and its `body` nodes again while a
field of its JSON result equals a string, as a do-while:

```jsonc
"work":   {"task": "Implement the ticket; fix every finding in loop_previous.", "profile": "deepwork-impl",
           "on_error": "continue"},
"verify": {"task": "Review the work.", "profile": "deepwork-adversary", "reads": "work", "on_error": "continue",
           "schema": {"type": "object",
             "properties": {"verdict": {"type": "string", "enum": ["pass", "reject"]},
                            "findings": {"type": "array", "items": {"type": "string"}}},
             "required": ["verdict", "findings"]},
           "loop": {"body": ["work"], "while": {"field": "verdict", "equals": "reject"}, "max_repeats": 2}},
"report": {"task": "Report the outcome.", "reads": "verify"}
```

- `while.field` is a dotted path into the loop node's result; a missing field,
  a non-string value or a null result ends the loop. `max_repeats` (1 to 9)
  counts passes after the first, so `2` means at most 3 passes. The loop node
  must have a `schema`, and when its schema declares the field (and an `enum`
  for it), the start checks `field` and `equals` against it. The start refuses
  a field whose declared `type` is not `string`, and a field the schema does
  not list in `required`: each segment of the path must be in the `required`
  of its enclosing object, even with `additionalProperties`, unless that level
  uses `anyOf`, `oneOf`, `allOf` or `$ref`. A result without the field ends the
  loop after one pass.
- A new pass starts only when every node of the loop is `done` in the last
  pass. A failed node ends the loop, even with `on_error: "continue"`; to
  retry a pass that timed out, set the body node's `retries`.
- Pass `k` of a node runs as uid `<id>#it<k>` (the first pass keeps `<id>`), in
  its own session, with the node's `timeout_ms` and `retries`.
  `workflow::node-result` with `<id>#it<k>` returns pass `k` (`<id>#it0` the
  first pass); the bare id of a loop node or body node returns its last pass,
  like `reads` and `output`. It returns null for a node that is not `done`.
- Every node of the loop receives `{"input": <its input>, "loop_previous":
  <the loop node's result of the previous pass>}`, and a body node also
  `"own_previous": <its own result of the previous pass>`. Both are null on
  the first pass. Inside the loop, `reads` reads the same pass.
- Outside the loop a node can read only the loop node: readers wait for its
  last pass and read that result. `output` may name the loop node and any
  body node: each returns its last pass, all from the same pass, and the run
  waits for the loop to end even when `output` names only a body node. A body
  node whose last pass failed under `on_error: "continue"` returns `null` (its
  error is in `failed_nodes`), never an earlier pass; read one with
  `workflow::node-result` and `<id>#it<k>`. Without `on_error: "continue"`, a
  failed pass of any node of a loop the output needs fails the run.
- Put the loop node in `output`, or read it from a later node. After the run,
  a result whose `while.field` still equals `equals` means the loop gave up:
  the repeats ran out, or a node of the last pass failed (see `failed_nodes`).
  Any other value means the condition stopped holding. The pass count is the
  number of `<id>#it<k>` uids in `nodes`, plus one.
- Say in the `task` of each node of the loop how to use `loop_previous` (for
  example: fix every finding in it). A body node whose result is the work
  itself (a text or a JSON document, nothing left on disk) starts every pass
  from scratch unless its `task` says to build on `own_previous` (for example:
  keep own_previous and change only what the findings ask for).
- Every body node must feed the loop node, a body node must not read its loop
  node, a node belongs to at most one loop, and no node of a loop may declare
  `fanout` or be iterated by one.
- Under `max_concurrency`, free slots go to ready uids in node-id order, so an
  earlier node's pass can take a slot before a later node's first pass.

## `workflow::status`

`workflow::status { run_id, details? }` returns `run_id`, `status`, `nodes`
(uid → state), `node_errors`, `node_results`, `result`, `result_error`,
`created_at`, `updated_at` and `phases`.

- `nodes` also lists the nodes that have not started as `pending`. An
  unexpanded fanout appears once under its node id; a fanout over an empty
  array lists nothing. Each loop pass is listed under its own `<id>#it<k>` uid,
  in `nodes`, `node_errors`, `node_results` and `node_details`.
- `phases` counts node uids per `phase`: `{"code": {"total": 2, "done": 0,
  "failed": 0, "running": 1}}`. `failed` includes cancelled. Nodes without a
  phase are left out, and the map is omitted when empty. Each loop pass counts
  once, so `total` grows with every repeat.

With `details: true` it adds `node_details` for at most 100 uids (running
first, then pending, failed, cancelled, done) and `details_truncated: true`
when it cut the list:

```jsonc
"node_details": {"kan-12": {
  "state": "running", "label": "KAN-12 · x", "phase": "code", "model": "m",
  "profile": "deepwork-lead", "session_id": "wf_r_ab_kan-12@r0", "attempt": 0,
  "started_at": 1, "finished_at": 2,
  "harness_status": "completed", "expects_wake": true, "last_generate_at": 3,
  "usage": {"complete": true, "sessions": 3, "turns": 12, "function_calls": 40,
            "function_call_errors": 0, "input_tokens": 1, "output_tokens": 1,
            "cache_read_tokens": 1, "cache_write_tokens": 1,
            "reasoning_tokens": null, "cost_usd": 0.42}}}
```

`usage` comes from `harness::metrics` over the node session's tree, and `model`
and `last_generate_at` from its context snapshots. `harness_status` and
`expects_wake` come from `harness::status`, for running nodes only. A node whose
send never reached the harness gets no live fields. These live
reads run without the run lock, 16 at a time, with a 5 s timeout each. A failed
read leaves its fields out, and nothing live is stored. `model` of a node with
a `profile` is shown only once observed, because the profile may pin another
model. Callers compute idle time from `last_generate_at`.

## Limits

- **Parked nodes and stop.** A node whose session waits on an armed wake stays
  running until a later turn ends it. When this worker stops a node session
  (timeout, refire, `workflow::stop`, a run ending with nodes in flight), it
  disposes every observer in the session's tree after the stop, so a parked
  session ends cancelled instead of waking later. This needs harness v1.9.4 or
  newer: an older harness ignores `quiet_deadlines`, so disposing a parked
  node's armed wake sends it a `[notification] wake removed` message and starts
  a new turn right away. A wake that delivers between the stop and the disposal
  starts one more turn, which a second stop cancels; wakes that turn arms before
  the second stop stay armed, so the session can still wake once more. If
  `harness::stop` fails, the observers stay and the session can still wake;
  nothing retries the stop or the disposal for a refired or timed-out node; only
  `workflow::stop` gets a second pass when the run finalizes. A node stopped
  from outside while it holds armed wakes settles only when its wakes retire or
  at its `timeout_ms`.
- **A finished node settles on a second poll.** A node whose `harness::status`
  reads finished is polled again 2 s later and settles from that reply, so a
  parked node does not settle on its parking turn. Each tick or sweep pass that
  settles a node takes about 2 s longer, holding the run lock and one of the 10
  tick-queue slots that all runs share. A wake whose turn starts more than 2 s
  after its claim, or whose dispatch fails after the claim, can still settle the
  parking turn. A message that reaches a finished node session within the 2 s
  makes the node settle on that later turn.
- **The timeout counts queue wait.** `timeout_ms` runs from the send, so it
  includes harness queue wait and parked time. The sweep checks it once a
  minute.
- **`retries` defaults to 1.** A node that times out is refired once unless it
  sets `retries: 0`, and a refire repeats the node's side effects.
- **No ordering-only dependency.** A node runs after the nodes it names in
  `reads` or `fanout`, and receives their results, so in "do X, then
  validate" over shared state the validator reads X's result, which may be
  text.
- **Loops are bounded and quiet.** Each pass is a new session; continuity is
  the work it leaves behind plus `loop_previous` and, for a body node,
  `own_previous`. A loop ends without an error when its repeats run out, when
  the condition reads a missing field, or when a pass would push the run past
  50,000 materialized nodes. Put the loop node in `output`, or read it from a
  later node: a last result whose `while.field` still equals `equals` means
  the loop gave up (the repeats ran out, or a node of the last pass failed;
  see `failed_nodes`), any other value means the condition stopped holding,
  and the pass count is the number of `<id>#it<k>` uids in `nodes`, plus one.
  Say in the `task` of each node of the loop how to use `loop_previous` (for
  example: fix every finding in it), and in a body node that answers with the
  work itself, to build on `own_previous`. No node of a loop can fan out.
  Outside its loop a node reads only the loop node; `output` may also name a
  body node, which returns its last pass.
- **The run result is capped at 1 MiB.** An `output` whose combined value is
  larger fails the run, and `result_error` says so. The node results stay
  readable with `workflow::node-result`, so name fewer nodes in `output` when
  their results are large.
- **Usage covers the current attempt.** A refired node reports the usage of
  its newest `@r<n>` session tree only.
- **Stopping an idle caller.** Stopping an idle caller cancels the running
  nodes, but consumers of `on_error: continue` nodes still fire. Use
  `workflow::stop` to end the run.
- **`finished_at` after an upgrade.** Nodes that ended before the upgrade get
  the time of the run's next write as `finished_at`; a run that had already
  ended has none.
- **`reply_to` runs after an upgrade.** A run started with `reply_to` before
  the upgrade gets no delivery: its record still loads, but nothing posts its
  outcome and nothing wakes its caller. To recover, prompt the caller to
  observe the run with args `{ run_id }` (a run that already ended is replayed
  at once; one still running reports when it ends) or to read
  `workflow::status`, within 24 h of the run's end.
- **Forgotten observe.** An agent that never makes the `next` call is not
  woken; the outcome stays in `workflow::status` for 24 h. The worker logs a
  warning and counts `workflow.runs.ended_unobserved` for a run with a stamped
  caller, no `notify` and no observer of that run (an every-run observer does
  not count). A run that ends before its
  caller's observe lands counts too: unobserved minus `workflow.runs.replayed`
  is the number of stalled callers.

---

## Events

| Function | Args | Filter on | Event |
|---|---|---|---|
| `workflow::on-run-completed` | `{ run_id? }`: one run, replayed if it already ended; omitted: every run | `input.run_id`, `input.status` (no replay) | `RunCompletedEvent { run_id, status: completed \| failed \| cancelled, result, result_error, failed_nodes? }` |

The function is private (only this worker calls it; anyone may observe it) and
returns the event it is called with. The worker calls it once per terminal run,
at least once (a crash mid-finalize re-fires it), and only while an observer
wants that run; dedup on `run_id`. `failed_nodes` lists the
`node '<uid>': <error>` of each node a completed run tolerated failing
(`on_error: continue`), and is omitted when there are none.

An observer with args `{ run_id }` gets only that run. Adding it for a run that
already ended replays the event to it at once, through the observer: a call
target receives the bare event, not the `Observed` envelope of a live delivery,
so a worker with its own target must accept both (or use `notify`). An observer
accepted during the run's final tick can get both the live event and the
replay. Adding one for an unknown or collected run is refused with
`UNKNOWN_RUN`, and unknown args with `INVALID_ARGS`. `filters` get no replay:
to wait for one run, use args, never filters.

## Observers

| Observed function | Args / filters | Target | Target receives |
|---|---|---|---|
| `cron::on-tick` | args `{ expression: <sweep_expression> }` | `workflow::sweep` | the tick (`capture: [output]`, read as `payload.output`) |
| `harness::on-turn-completed` | filters `{ "input.session_id": { "$prefix": "wf_" } }` | `workflow::wake` | `TurnCompletedEvent` of a node session (`capture: [output]`, read as `payload.output`) |
| `harness::hook::on-pre-call` | args `{ functions: ["workflow::start"], on_error: "fail_closed", timeout_ms: 30000 }` | `workflow::stamp-reply` | the pre-call hook input; returns the hook decision. A hook error or timeout refuses the agent's `workflow::start` |
| `harness::hook::on-post-turn` | args `{ sessions: ["wf_*"], on_error: "fail_open", timeout_ms: 30000, retry_prompt: <answer-format nudge> }` | `workflow::validate-result` | the post-turn hook input; returns `deny` with a reason for a JSON node's answer that is not a JSON value matching its schema, unless a wake is armed. An error or timeout passes the turn, and reconcile still checks the result |
| `harness::hook::on-pre-generate` | args `{ on_error: "fail_open", inject_prompt: <workflow guidance> }`, only while `inject_guidance` is on | `workflow::inject-guidance` | not called: the harness joins `inject_prompt` into the system prompt |
| `configuration::on-updated` | filters on `input.id` and `input.event_type` (through `iii-config-client`) | `workflow::on-config-change` | the configuration event (`capture: [output]`, read as `payload.output`) |

A `sweep_expression` change observes the new expression first, then disposes
the old observer; if the cron worker refuses the new expression
(`INVALID_CRON_EXPRESSION`), the old observer and schedule stay.

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
A tick persists each node's attempt session before sending it; a later tick
that finds the session reserved asks `harness::status` before sending again.
A `workflow::start` `idempotency_key` is checked and written under one fixed
lock (`"idem"`), and the sweep deletes the key with its run under that lock.

Node sessions carry the caller as `metadata.parent_session_id`, so they sit in
the caller's session tree: stopping the caller's session stops every running
node. When the caller's turn reads `cancelled`, the run ends `cancelled`, even
with `on_error: continue` nodes; an idle caller is not detected (see Limits).
`workflow::stop` stops one run's node sessions without touching the caller.

Node sessions run in the caller's folder: the pre-call hook stamps the caller
turn's `metadata.fs_scope.root` as `caller_fs_root` (replacing any value the
agent sent), and every node send carries it as
`options.metadata.fs_scope.root`, unless the node sets `filesystem_root`.
Without one, nodes get the harness default folder. A non-agent caller (CLI, worker) is not hooked, so its
`caller_fs_root` is used as sent; an empty one is refused. A node granted
`workflow::start` and `workflow::on-run-completed` (its `next` observe;
`workflow::*` is denied by default) passes its own folder to a nested run and
is woken when it ends. The hook runs only on
calls made from an agent turn, so the harness refuses `workflow::start` as an
observer binding target and `fp` refuses it as a pipe step; both need the
harness and `fp` releases that carry those refusals.

### One instance per namespace

A function id has one host per namespace: the engine rejects a second
registration of the same id with `FUNCTION_NAMESPACE_CONFLICT`. The worker
registers every `workflow::*` function, `workflow::on-run-completed` included,
so the first instance in a namespace hosts them all and the
per-run `WorkflowLocks` live in that one process. A second instance in the same
namespace gets every registration rejected while its observers (the sweep and
the harness hooks) still deliver to the first instance's functions, so each
sweep and hook runs twice. Run one workflow worker per namespace.

### Why not `state::compare-and-set`

`state::set` is an unconditional overwrite. `state::compare-and-set` exists,
but the worker does not adopt it for run records or idempotency keys:

- it compares the whole stored value, so every run-record write would have to
  carry the full previous record;
- an absent key compares equal to a stored `null`;
- it has no size guard;
- on Redis it retries a scope-wide `WATCH` up to 8 times, so concurrent writers
  to different runs in the same scope contend with each other.

Since the engine allows one instance per namespace (above), the in-process
locks give every run one writer, and `WorkflowRunRecord` carries no `version`
field.
