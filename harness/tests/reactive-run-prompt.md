# The reactive acceptance run — rctest9

The successor to the rctest7 prompt, written for agent observers. Its
ancestor was written against a surface where database change events were a
stub, idempotency and joins were hand-rolled, and nothing bounded a runaway —
so it carried a probe-and-fallback ladder and an `fp::*` mandate. Each thing it
worked around has a named mechanism here, and this prompt makes each one
load-bearing and citable:

| rctest7 asked for | rctest9 uses |
|---|---|
| push DB change events when present, else state-bridge, else cron differ | `database::on-row-changed`, probed once, **fail loudly** — no fallback |
| fp-gated idempotency | `state::compare-and-set` claim, then work, then a separate announce key the barrier counts |
| fp-computed finish gate | a `state::barrier` condition with an explicit 15-key expect list |
| nothing | `lifecycle` bounds on every standing observer — what rctest7 died without |

Run it against a live stack with a real model. A fail report is the test
working; the only failure is a run that parks forever or leaves something
armed after its report.

## The prompt

````text
You are the coordinator of a reactive acceptance run. Pick a fresh run id:
rctest9- plus 4 random alphanumerics. You design the process; sub-agents stay
dumb; work flows DOWN as direct spawns and comes back ONLY through what gets
written and the observers you arm. A fail report is a SUCCESS for this
test; a run that parks forever, or leaves anything armed after its report, is
the only failure.

NAMES — derive these ONCE, now, and use the exact strings everywhere. Never
paraphrase a name you already chose:
  run id       = rctest9- followed by 4 alphanumerics YOU generate now.
                 Never reuse an id you have seen written anywhere, including
                 in this prompt's own examples.
  sql prefix   = the run id with "-" replaced by "_"
  tables       = <sql prefix>_orders, _writers, _totals, _report, _probe
  state scope  = the run id
  barrier id   = the run id
  sessions     = <run id>-writer-1..3, <run id>-aggregator

PRIMITIVES UNDER TEST — each of these must end up load-bearing, and the report
must cite where:
  database::on-row-changed     the watch (push, on commit, this worker's writes)
  conditions + state::barrier  the finish gate — decided, not eyeballed
  state::compare-and-set       idempotency — enforced, not hoped
  lifecycle on observers       nothing you arm may be able to deliver forever

SETUP + PROBE — in your first turn:
1. Create the tables. orders: (id INTEGER PRIMARY KEY, writer TEXT,
   amount INTEGER, created_at TEXT). _probe: (n INTEGER).
2. Inspect the observed function first: engine::functions::info
   { function_id: "database::on-row-changed" } — its args_format is what the
   observer's args must match. Then give the mechanism a bounded probe:
   harness::observe database::on-row-changed with args { db: "primary",
   table: "<sql prefix>_probe" } and no target, so it wakes THIS session,
   once. Arm the probe deadline as a one-shot observer of cron::on-tick with
   args { expression: "0 */1 * * * *" } and once: true EXPLICITLY (cron is the
   one source whose observers default to recurring; this expression ticks at
   the next minute boundary, giving the probe up to 60 seconds — never use a
   per-second expression as a deadline, it ticks before anything can
   happen). THEN
   insert one probe row and END YOUR TURN IMMEDIATELY — the race is judged by
   whichever wake arrives, never inline in a running turn. If the deadline
   wake arrives first, check your latest messages for a probe wake queued
   right behind it — if both arrived, the probe wins. Only if the probe
   truly never fired: write a fail report stating exactly what you
   observed and what never arrived, tear down, and stop. Do NOT build a
   fallback watcher — the mechanism either works or this run's job is to say
   so. When the probe wake arrives, unobserve the probe deadline cron
   before continuing.

WATCH — after the probe wake arrives:
3. Spawn the aggregator FIRST, before any writer: a sub-agent in session
   <run id>-aggregator whose task is, verbatim in spirit:
     "Arm ONE wake (harness::observe with no target — it wakes this session)
      on database::on-row-changed, args { db: 'primary', table:
      '<sql prefix>_orders' }, once: false, lifecycle: { max_fires: 60,
      expires_in_ms: 900000 }. Report the observer_id, then end your turn. A
      wake delivers ONE OR MORE [notification] messages — bursts queue and
      drain together — each carrying one insert event with returning rows
      [{id, writer, amount}].
      Process EVERY notification in the wake, one at a time, in THREE steps
      whose order is the whole contract — claim, work, THEN announce:
      (1) CLAIM: state::compare-and-set { scope: '<run id>', key:
          'claim-<id>', value: true } with NO expected field (only-if-
          absent). swapped:false → already processed: note 'duplicate
          order-<id> skipped' and move on.
      (2) WORK: upsert the running aggregate: INSERT INTO
          <sql prefix>_totals (writer, order_count, amount_sum) VALUES
          (?, 1, ?) ON CONFLICT(writer) DO UPDATE SET order_count =
          order_count + 1, amount_sum = amount_sum + excluded.amount_sum.
      (3) ANNOUNCE: only after the upsert succeeded, state::set { scope:
          '<run id>', key: 'order-<id>', value: { writer, amount } }.
      The announce key is what the finish gate counts, and it must mean
      'my work for this event is durable' — announcing before the upsert
      wakes the finisher while the last write is still in flight, and its
      verification reads a total one short. When every notification in the
      wake is processed, stop. Never poll, never query the orders table,
      never observe anything else."
   Grant it only what that task calls. You must already hold every function
   you hand down — a coordinator cannot grant what it lacks.
4. Arm YOUR one finish wake, gated so it delivers exactly once, when
   everything has landed:
     harness::observe { function_id: "state::on-changed",
       filters: { "input.scope": { "$eq": "<run id>" } }, once: true,
       conditions: [ { function_id: "state::barrier", config: {
         id: "<run id>",
         expect: ["order-1", "order-2", …, "order-15"],
         carry: "/new_value" } } ] }
   The expect list is EXPLICIT — a count cannot name what never arrived. The
   barrier never allows on incomplete, so also arm the run deadline: a
   harness::on-timer wake with args { in_ms: 600000 }. If the deadline wake
   arrives before the barrier wake, read state_barrier/<run id> — it lists
   exactly who arrived — name the missing orders in a fail report, tear down,
   stop.

WRITE — only after the watch is armed:
5. Spawn 3 writers in ONE message, sessions <run id>-writer-1..3, each with
   only database::execute. Writer w's task, self-contained, no context about
   watchers or siblings: insert 5 orders one call at a time with explicit ids
   — writer w owns ids (w-1)*5+1 … (w-1)*5+5 — each as
   INSERT INTO <sql prefix>_orders (id, writer, amount, created_at)
   VALUES (?, 'writer-w', ?, CURRENT_TIMESTAMP) RETURNING id, writer, amount
   — and ALSO pass the option returning: ["id", "writer", "amount"]. The
   RETURNING clause lives IN THE SQL; the option alone does not add it. On
   SQLite the option is refused only when the SQL returns no rows, so pass
   both (Postgres and MySQL ignore the option). These rows are what the aggregator
   keys its idempotency on; an insert without them is invisible to the claim. Then mark itself done:
   INSERT INTO <sql prefix>_writers (writer, done_at) VALUES ('writer-w',
   CURRENT_TIMESTAMP). No delays, no reads, no retries over time — but a
   call that FAILS with a clear argument error (wrong value count, malformed
   SQL) may be corrected and retried ONCE; a skipped order starves the finish
   gate. On a genuinely failed insert, still write the done row.
6. End your turn: say what you armed and spawned, then STOP. No polling —
   your next activity is the barrier wake, the deadline wake, or nothing.

FINISH — when the barrier wake arrives (it carries all 15 results):
7. Verify against the TABLES, not the notification:
   SELECT writer, COUNT(*) AS c, SUM(amount) AS s FROM <sql prefix>_orders
   GROUP BY writer — must match <sql prefix>_totals exactly, row for row,
   and 15 orders total.
8. Write ONE row into <sql prefix>_report — as a SINGLE INSERT statement in a
   single call (the worker refuses multi-statement SQL; compose the column
   values first, then one INSERT) — containing: mechanism used and the
   probe's measured wake latency; rows written vs events processed vs
   duplicates skipped (the compare-and-set losers); elapsed time computed by
   SQL, not from transcript timestamps — e.g. (julianday(CURRENT_TIMESTAMP)
   - julianday(MIN(created_at))) * 86400 over the orders table; the barrier's decisive trail (your own
   transcript's delivery records carry its skip notes — "barrier <run id>:
   N/15 arrived, waiting on […]" — cite the progression and the final allow);
   every observer you armed with its lifecycle bound and how it ended
   (delivered-and-retired / unobserved / expiring-at); pass or fail per
   acceptance check below.
9. TEAR DOWN, then stop: unobserve your run-deadline timer if it has not
   delivered. The aggregator owns its observer, so spawn a cleanup task INTO
   session <run id>-aggregator: "call harness::unobserve { id: <its
   observer_id> }, report the result, stop." Then verify:
   harness::observers::list for this session and for <run id>-aggregator
   must show nothing whose args or filters name this run's tables or scope.

ACCEPTANCE — the report self-verifies each of these, with the query or
citation that proves it:
  a. totals == GROUP BY over orders, exactly, all 15 orders covered
  b. no event lost and none double-counted: 15 compare-and-set wins; any
     duplicate wake was refused by swapped:false, and if none occurred, say
     so — the mechanism was in place either way
  c. the aggregator ran only when woken by its observer — cite its session
     id and one delivered event; it never queried the orders table
  d. nobody slept or polled in place of an observer: the finish came from the
     barrier-gated wake, and the deadline existed but (pass case) never
     delivered
  e. the finish was DECIDED by the barrier condition — cite its final allow
     carrying the 15 results; you never counted rows to decide doneness
     yourself
  f. every standing observer carried a lifecycle bound when it was armed, and
     after the report zero observers for this run remain

Report progress as you go; keep the final summary short and factual.
````

## Why each rule is there

| Rule | The failure it prevents |
|---|---|
| Names derived once | `rctest_rctest7_*` stutter; a gate counting a table nobody writes |
| Probe then fail loudly | a run that silently builds a poller when the push path is broken — the finding this test exists to surface |
| Aggregator spawned before writers | events emitted before anything observes are lost |
| Bursts queue and drain together | an aggregator told "one event per wake" that ignores messages 2..N of a drained batch |
| `returning` on every insert | events with no identity — nothing for the claim to claim |
| Claim first, announce LAST, two keys | run 3 collapsed them into one: the 15th claim woke the finisher while the 15th upsert was in flight, and its verify read a total one short — caught by its own report |
| Explicit expect list | a barrier starved at 14/15 with no way to name the missing producer |
| Deadline timer + readable barrier record | the barrier never allows on incomplete; the deadline path must exist and can say exactly who is missing |
| Lifecycle on every standing observer | rctest7: 69 deliveries at 10s intervals until a human intervened |
| Cleanup INTO the aggregator session | `harness::unobserve` is owner-scoped; the coordinator cannot remove an observer it does not own |
