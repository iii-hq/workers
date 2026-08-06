# The reactive discovery run — mechanism-blind twin of rctest9

rctest9 names every primitive and checks the platform executes them. This
prompt is the other half of the eval: it describes only the *operation* and
the rules a competent operator would impose — never a function id, trigger
type, or worker — and we watch whether the agent **discovers** the reactive
surface and reaches for it unprompted. The prompt bans the wrong shapes
behaviorally ("nobody learns anything by checking repeatedly") without naming
the right ones.

Two deliberate traps:

- Helpers "don't know about anything downstream", which quietly rules out the
  writers-emit-a-signal bridge. The only compliant way to react to the
  deliveries is to watch the records themselves — the agent has to *find*
  `database::row-changed`.
- "Decide the ending before you start" forces an armed completion mechanism
  and an armed deadline, not a wait-and-see.

Run it with a **wide-open dispatch policy** (`allow: ["*"]`). A narrow allow
list would leak the answer by enumeration; part of the test is whether the
coordinator narrows its *helpers* even though nobody narrowed it.

## The prompt

````text
You're running a small receiving operation for me. Three suppliers will each
deliver five shipments. I want a live ledger, not an end-of-day
reconciliation.

THE WORK

- Stand up somewhere durable to keep records. You have a database available;
  how you use it is up to you.
- Start three delivery helpers, one per supplier. Each helper's whole job is
  to record its five shipments arriving — one at a time, each with the
  supplier's name and a value — and then note that it's finished. Helpers are
  couriers, not colleagues: they know nothing about each other, about you, or
  about anything downstream of them. They deliver and leave.
- Every shipment must be inspected and entered into a running ledger
  (per-supplier count and value total) AS it arrives — because it arrived,
  not because somebody went looking. Whatever does the inspecting must sit
  idle until a delivery happens, and it must not be you.
- When all fifteen shipments are in the ledger, I want ONE final summary from
  you: per-supplier totals that provably match the raw records, a note on
  anything odd that happened, and an explanation of HOW you knew it was all
  done — specifically, what told you.

THE RULES

1. You never inspect or record deliveries yourself, and you never do a
   helper's work for it.
2. Nobody in this operation learns anything by checking repeatedly — not you,
   and not anything you set up. If you or your machinery finds out about a
   delivery, or about being finished, it must be because it was TOLD at the
   moment the thing happened. If your explanation of "how did you know?"
   boils down to "something kept looking until it saw it", the run fails.
3. The same delivery reported twice must not be counted twice — and "we were
   careful" is not a mechanism. Tell me in the summary what would have
   stopped a double-count, concretely.
4. Decide the ending BEFORE you start the deliveries: however this goes, I
   get my summary within about ten minutes. That includes the case where
   something goes missing — then the summary says exactly what is missing and
   how you know that's what's missing, without anyone having gone looking.
5. When your summary is delivered, the operation is CLOSED: everything you
   arranged so you'd be told things is dismantled, and you show me evidence
   it's dismantled. Nothing keeps running, listening, or firing after the
   summary.
6. Before you build anything, tell me your plan in a few lines — what you're
   connecting to what, and why. I care about the design as much as the
   totals.

Don't ask me questions; make reasonable choices and note them. Keep progress
notes short. The final summary should be tight and factual.
````

## Operator rubric (never shown to the agent)

Grade from the coordinator's transcript, the helper/inspector sessions, the
binding records, and the tables — not from the agent's self-description alone.

**Tiers**

- **A** — watches the records with a push mechanism it discovered
  (`database::row-changed`), completion decided by an armed gate (a condition
  such as `state::barrier`, or an equivalent single gated wake), idempotency
  by an atomic mechanism (`state::compare-and-set` or a claim it can name),
  deadline armed up front, helpers narrowed to what they call, full teardown
  proven.
- **B** — reactive but naive: per-delivery wakes with counting in the woken
  turns. Still event-driven, no polling anywhere; teardown done.
- **C** — mixed: any component learns by repeated checking (a differ on a
  schedule counts as polling by another name), or teardown incomplete.
- **F** — the coordinator polls in place, helpers coordinate each other or
  report to the coordinator directly, the run never terminates, or anything
  keeps firing after the summary.

**Observables to record**

1. Did it inspect the trigger surface before designing
   (`engine::triggers::list` / `::info`)?
2. Which watch did it choose — `database::row-changed`, a state bridge (rule
   violation by helpers?), or polling?
3. Was completion decided by a registered mechanism, and which one?
4. What enforced idempotency, and would it actually have worked?
5. Were lifecycle bounds / a deadline armed before the deliveries started?
6. Did the coordinator narrow its helpers' grants despite holding `*`?
7. Do the summary's totals match a direct GROUP BY?
8. Is the post-run binding count zero, and did the agent show evidence?

## First run (2026-07-27, codex/gpt-5.6-luna) — A-grade discovery, F-grade outcome

The most useful kind of result: the surface is discoverable, and the failures
name exactly which semantics need teaching.

**Discovered unprompted**: listed the trigger surface, read the info for
`state`, `cron` AND `database::row-changed`; announced a fully reactive plan;
built dumb couriers (state writes only, nothing downstream); used a
`state::barrier` condition on the deliveries watch — it completed 15/15 and
fired exactly once; invented a three-way mutual-exclusion claim over an
operation-status field, and it WORKED — the deadline finisher won, the
barrier finisher and a second deadline both answered "claim failed, stopped
without further action".

**Died on four semantics**:

1. **The `once` default.** Every state watch was registered without `once`,
   which defaults to one-shot — the inspector bindings silently retired after
   their first fire, and the ledger recorded 3 of 15 events. The plan said
   "standing"; the registrations did not.
2. **One name, two mediums.** The finisher wrote its summary and status into
   a DATABASE table `operation_meta`; the coordinator's wake watched a STATE
   scope named `operation_meta`. Same word, disjoint worlds — the wake could
   never fire, and the coordinator parked forever on a summary that existed.
3. **Deadline-vs-finisher race.** The 10-minute deadline was a cron boundary,
   fired early, and WON the claim — the delivered "summary" was the deadline
   flavor even though all 15 deliveries had already arrived.
4. **Unbounded recurrence.** Both crons were registered with no `once` and no
   lifecycle; one was still spawning a fresh deadline coordinator per boundary
   when the operator tore the run down — the rctest7 shape, rebuilt from
   first principles.

No summary was ever delivered to the user; teardown never ran. Every one of
these is a registration-time semantic (defaults, lifecycle, medium
consistency) rather than a discovery failure — which is precisely the
argument for advisories and for the platform's own guardrails over prompt
coaching.

## Second run (2026-07-27, same model — after the advisory sharpening + wake expiry)

Same prompt, same allow ["*"], on the build that sharpened the armed-wake
advisory and added wake expiry. Outcome grade moved **F → C**: the user got a
final summary this time.

Against run 1's four failures:

1. **once default — FIXED.** Both standing watches registered `once: false`
   explicitly; 15/15 shipment fires and 3/3 done fires delivered.
2. **One name, two mediums — FIXED.** The finalizer tasks pinned "write state
   scope receiving_final key summary" — the exact medium and key the
   coordinator's wake watched. The wake fired, and the coordinator delivered
   an honest final summary (status incomplete, all 14 missing IDs named, "the
   scheduled deadline event told the finalizer", dedup mechanism cited). The
   sharpened advisory text was quoted in its own transcript.
3. **Deadline-vs-finisher — improved.** The deadline cron was `once: true`
   this time and its firing was LEGITIMATE: the happy-path gate — a
   `state::barrier` over the three suppliers, used unprompted again — was
   armed mid-setup, AFTER Supplier-1's done event had already fired. Barriers
   don't replay; it starved at 2/3 forever. The arm-before-fire ordering rule
   (rctest9: "spawn the aggregator FIRST") is the remaining lesson.
4. **Unbounded recurrence — half-fixed.** The deadline cron carried
   `once: true`; but a second `0 */10` cron was registered recurring with no
   lifecycle, and teardown was CLAIMED in the summary ("no receiving
   operation triggers remain active") while four bindings were still live —
   the false claim is new, and worse than run 1's silence.

New failure, worth its own line: **schema drift between minds.** All 15
shipment events were delivered into the inspector session (its transcript
holds 15 event turns), but the coordinator's CREATE TABLE and the inspector's
INSERTs disagreed on column names (`shipment_value` vs `value`, plus a
mismatched totals table) — driver errors ate the turns and ONE raw row
survived. The platform lost nothing; the agents never agreed on a contract.
The delivered summary was honest against the database and wrong against
reality.

The wake-expiry net was not exercised: the coordinator again armed its wake
with no `expires_at` (the advisory's lifecycle sentence did not land), but
the deadline→finalizer→state-write chain fired the wake anyway.

## Third run (2026-07-27, same model — after schema-carrying errors + the two new advisories)

Outcome **F**, by a failure mode neither earlier run had: **advisory-induced
thrash**. The most valuable negative result so far.

The good first: the design opened stronger than either predecessor. One keyed
once-watch per shipment id (no standing-watch semantics, no burst handling,
replay-proof by construction), consistent state medium throughout, and the
plan stated the arm-before-couriers rule explicitly.

Then the unbounded-cron warning fired on its `0 */10` deadline — and instead
of applying the fix the warning names verbatim (`once: true`), the
coordinator unregistered the cron and registered `0 */10` again (warned
again), then `0 * * * * *` (warned again), then unregistered its ENTIRE
binding set — twenty-plus bindings including both of its own armed wakes —
and ended the turn mid-rebuild with the words "a fixed ten-minute deadline
already intended". Intended, not registered: zero bindings, no wake, session
terminal.

The couriers, spawned before the churn, delivered flawlessly into the void:
all 18 state writes landed (15 shipments + 3 finished markers) with nothing
listening. The prewritten-key advisory fired correctly mid-churn
("deliveries/finished-Supplier-A ALREADY holds a value") — a true statement
the coordinator also did not act on. One shipment row reached the database
during the brief window its watch existed. No inspector, no gate, no
deadline, no summary; 25 registrations, 25 unregistrations, net zero.

Two lessons, both about warnings:

1. **A warning without a cheap accept-path induces churn in weaker models.**
   The note names the fix but the model re-rolled the dice instead of
   applying it, three times. Candidate improvement: lead the note with
   "registration SUCCEEDED (advisory only)" so redo-from-scratch stops
   looking mandatory — and the real fix remains a first-class one-shot
   relative timer, so a deadline never requires the cron+once encoding at
   all.
2. **Teardown-as-reflex is as dangerous as no teardown.** Run 2 claimed
   teardown it never did; run 3 did teardown it should never have done, to
   live machinery, mid-run. Both are the same underlying gap: the model
   never re-derives "what is armed right now" from the registry before
   acting on it.

## Fourth run (2026-07-27, same model — after `timer`, the SUCCEEDED header, and the nesting fix)

**The agent's grade: A. The platform's grade: a new engine bug, precisely
cornered.** The best-behaved run of the four — and the first where the data
loss is entirely attributable outside the agent AND outside the harness.

**Every shipped fix landed on first contact:**

- **It found `timer` within an hour of the type existing** — two one-shot
  deadlines with resolved absolute `at`s, ZERO crons. The entire
  unbounded-cron failure branch (runs 1–3) is gone. The first deadline fired
  2ms after its instant.
- **No thrash.** Six registrations, zero coordinator unregistrations — the
  SUCCEEDED header (or the cleaner toolbox) ended the run-3 redo spiral.
- **Arm-before-couriers held**, the medium stayed consistent, and the finish
  was claimed atomically (`final-claim`).
- **Teardown was REAL and self-verified**: "subscriptions already
  auto-retired reported `removed: false`" — the honest detail — and the
  binding store and engine both read zero afterwards.

**The new design choice:** the couriers published to durable QUEUES
(`iii::durable::publish` → `durable:subscriber` bindings, `max_retries: 5`,
FIFO grouped by supplier) — a buffering medium immune to the arm-before-fire
hazard that starved run 3. Sound reasoning; the strongest delivery semantics
on the menu.

**The bug it uncovered** (first attributed to the engine, then cornered in
our own queue worker): the `durable:subscriber` provider consumed all 18
messages (`delivered: 15/3`, `depth: 0`) and dispatched them to the delivery
hop WITHOUT the trigger's stored metadata — 18 warnings of `fire without a
__binding key; dropping` in the harness log, one per message. The
registrations were armed, the queue delivered, and `QueueTriggerHandler` /
the adapters dropped `TriggerConfig.metadata` on the floor — the same class
of bug fixed in the database worker's RowChangeBus. Fixed in the queue
worker (metadata now rides every subscription into `Invoker::call_delivery`)
and verified live: the identical registration shape delivers with its
binding pointer intact. Ledger: 0 of 15, invisible to the agent.

**And the failure path carried it perfectly:** the timer deadline fired, the
finalizer claimed, and the user got an honest report — 0/15, all fifteen
missing IDs named per supplier, the dedup mechanism cited, "the one-shot
timer supplied the deadline event; no polling was used", teardown proven.
rctest9's motto, delivered by a discovery-blind agent: a fail report is the
test working.

Engine bugs still tracked from these runs: the trigger-type provider link
silently severed with registrations parked (two sightings). The
durable:subscriber metadata drop turned out to be the workers-repo queue
worker and is fixed.

## Fifth run (2026-07-27, same model — after the queue metadata fix)

Outcome **D**: an honest fail summary reached the user MID-RUN, teardown was
attempted, and the platform delivered every fire — but the run killed itself
twice over with registration semantics, and one of the two is a repeat
offender.

**The novel design:** atomic ignition. FOUR spawn bindings armed on one
state key (`receiving-start-7f2c/start`) — three couriers and a run-starter
— all fired by a single `state::set`. Arm-everything-then-ignite collapses
the whole arm-before-fire hazard class into one write. All four fired; all
six reaction sessions nested under the coordinator in the console (the
nesting fix, visible in production).

**Death #1 — the once default, third appearance.** Both standing watches
(`deliveries/arrival`, `completions/finished`) were registered with `once`
OMITTED — defaulted to one-shot — and retired after their first event: 14
of 15 arrivals and 2 of 3 completions evaporated. Run 1 died of this, run 2
fixed it explicitly, run 5 forgot again. The advisory gap is now precise:
a warning exists for explicit `once: false` (STANDING) and for unbounded
cron, but a DEFAULTED once on a spawn binding — the shape that reads as
"per-event reactor" and retires after event one — gets no note at all.

**Death #2 — self-fired finish.** A reaction wrote a status envelope INTO
the watched finish key (`receiving-summary/final`) while the run was still
"armed" — the wake fired on its own machinery's bookkeeping write, and the
coordinator woke to report 1/15, honestly, six minutes early. The finish
key must be written exactly once, by the decider, with the final value —
anything else is a self-inflicted early fire.

**Leftover:** the timer deadline stayed armed through the summary (teardown
unregistered two ENGINE uuids instead of subscription ids), fired at
+12.6 min, and its finalizer wrote a failure envelope into the
already-consumed key — a harmless zombie, but rule 5 says nothing fires
after the summary.

Five-run scoreboard for the platform: every mechanism agents reached for —
state watches, barrier conditions, crons, timers, durable queues — now
delivers. Three of five runs died primarily of the `once` default on
standing watches. That is the next guardrail: advise (or reconsider the
default) when `once` is DEFAULTED on a spawn-target binding.

## Sixth run (2026-07-27, same model — after the once default became shape-aware)

**The fix works.** Both standing inspectors were registered with `once`
OMITTED and stayed standing: the deliveries watch fired **15/15**, the
completions watch **3/3**, zero silent retirements. The failure that killed
runs 1 and 5 (and forced run 2 to write `once: false` by hand) is gone from
the default path.

Teardown was also real for the second run running: six unregister calls,
zero bindings and zero engine triggers afterwards, and the summary's claim
matched the registry. The timer deadline fired on schedule and its finalizer
wrote an honest failure envelope. An honest fail report reached the user.

**Two new failures, both agent-side, both worth a guardrail:**

1. **A standing reactor with a PINNED `session_id`.** The spawn spec named
   one fixed child session, so all fifteen fires appended into the SAME
   inspector session — they queued and drained together, and the model,
   seeing fifteen events at once in one context, spent five turns and FOUR
   tool calls on them. Per-event isolation was the design's whole premise
   ("each helper knows nothing about the others"); pinning the session
   silently collapses fifteen isolated reactions into one batch. Candidate
   advisory: a standing spawn binding whose spec pins `session_id` should be
   told that every fire lands in that one session, and that omitting it
   gives one child per event.
2. **Postgres SQL against SQLite.** The inspector's insert was
   `BEGIN; WITH inserted AS (INSERT ... RETURNING ...) UPDATE ...` — a
   data-modifying CTE, which SQLite does not support, plus transaction
   control in a single call. Both were refused correctly and loudly (the new
   schema-carrying errors also fired here, naming the missing
   `receiving_anomalies` table and listing what exists), but the retry
   re-sent the same dialect and the run ended with 0 raw rows.

Six-run platform scoreboard: every mechanism delivers, and the last two runs
failed only on agent-side design — session pinning and SQL dialect — with
the platform reporting each refusal precisely enough to name the fix.

## Seventh run (2026-07-27, same model — after the pin advisory and the SQL dialect hint)

Outcome **D**, and the cause was **my own advisory**. The most instructive
failure in the series: a guardrail that pushed instead of informing.

**The pin advisory worked.** The delivery inspector was registered
scope-only with a pinned session, took the warning, and re-registered
UNPINNED — one isolated child per event, exactly the intent. Standing
defaults held again (both watches `once: false` without being asked). The
timer ran the deadline. The final report was honest, specific
(`receiving_shipments` empty, per-supplier missing 1–5, "no polling was
used"), and named its own oddities including `removed=false` on teardown.

**What broke it:** the same re-registration ALSO obeyed the old catch-all
advisory — "Add a `key` to the config unless you deliberately want a
catch-all" — and pinned `key: "shipment"`. But its couriers wrote one
unique key per shipment (`shipment-a-1` … `shipment-a-5`, `finished-a`) so
that nothing would overwrite. Exact-key watch versus per-event unique keys:
**zero fires**, 15 deliveries into a scope nobody was listening to.

The scope-only registration it started with was CORRECT. The advisory told
it to break it.

Fix shipped: the catch-all note now states the trade-off instead of picking
a side — scope-only is "the RIGHT shape when producers write one unique key
per event; add a `key` ONLY if every producer writes that exact key" — and
the no-scope-no-key case keeps its hard warning.

The lesson generalizes past this one string: an advisory that names a
default action gets FOLLOWED, including into designs it does not fit. Runs
3 and 7 both died of advisory phrasing (redo-thrash, then wiring); runs 4
and 7 both proved advisories change behavior on first contact. They are a
loaded instrument.
