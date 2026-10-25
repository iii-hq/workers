# eval

The Harness E2E improvement loop. One **cycle** record, started from either
input, moves through a fixed sequence. Harness E2E runs the scenarios. Eval
keeps the cycle, the evidence, the budget and the decision. The central
Sentinel groups the failures that runs recorded. User chats are never read.

```
objective ─▶ coverage ─▶ plan frozen ─▶ baseline reserved + dispatched ─▶ awaiting_execution ─┐
execution_id ──────────────────────────────────────────────────────────────────────────────┴▶ analysis ─▶ candidate | done
```

## Functions

| Function | Use |
|---|---|
| `eval::configure` `{config}` | Set the configuration that every new plan freezes. Its revision is the `policy_revision`. |
| `eval::config` | Read the configuration (`null` until configured). |
| `eval::start` `{objective}` or `{execution_id}` | Start one cycle from exactly one input. |
| `eval::status` / `eval::result` `{cycle_id}` | Read the cycle record. `result` also returns what the cycle drew on its budget. |
| `eval::list` `{limit?}` | List cycles, newest first, with what each drew on its budget (`usage` by `cycle_id`). |
| `eval::cancel` `{cycle_id}` | Stop a cycle and the E2E executions it dispatched. |
| `eval::executions` `{query?}` | Recent finished E2E executions a cycle can start from: each scenario's mean score and what a cycle would restore, or the cycle that already imported it. Executions a cycle dispatched are left out. |

The console page (`ui/`) shows the cycles as four phases, Measure,
Improve, Prove and Ship, with one sentence for what runs now or why a cycle
stopped and the setting that would let it go on. The New cycle dialog
starts from an execution picked with `eval::executions`, or from an
objective. Loop settings are laid out as the same ladder: each phase needs
the one before it.

These functions are internal: `eval::step` runs the queued step,
`eval::sweep` runs on cron every 15 s, and `eval::execution-changed` is bound
to `e2e::dashboard::changed`. The contract is in `src/contract.rs` and its
schemas are in `tests/golden/schemas`.

## Configuration

No field has a default. An unconfigured loop selects coverage, but it
dispatches nothing.

```json
{"config": {
  "subject": {"provider": "anthropic", "model": "claude-...", "agent": null},
  "where": "harness",
  "runs_per_side": 5, "technical_retries": 1, "max_scenarios": 1,
  "estimated_usd_per_run": 0.5,
  "min_effect": {"tokens": 5000, "pass_rate_pp": 10},
  "budget": {"max_spend_usd": 10, "max_duration_ms": 7200000, "max_calls": 20, "max_candidates": 2}
}}
```

`where: harness` runs on the E2E's own Harness. It observes the current stack
and cannot pin a candidate. `docker` and `github` need a
`stack: {name, yaml}` in the E2E's own shape. One example is
`package://harness` pinned to `commit:<40 hex>` of `iii-hq/workers` at path
`harness`. Credentials stay in the providers.

## Objective input

1. The objective names a need. Today the needs are `context_consumption`,
   `provider_fallback` and `skills`.
2. Every page of `e2e::dashboard::tests-list` is read, together with
   `e2e::scenarios-list`.
3. A test covers the need only when its spec matches the need's word groups.
   The spec is its criteria and summaries, plus the catalog invariants.
   The prompt, a matching title or an id is not enough.
4. A test is selected only in its current definition: the row's
   `current_version` must equal the scenario's `behavior_sha256`. History of
   another `selected_version` is flagged and never used as evidence.
5. If nothing covers the need, the cycle completes with `no_coverage` and the
   exact gap. Nothing is spent. `tool_contract_recovery` does not cover
   provider/model fallback.
6. If a test covers it, the plan freezes before any result:
   - the metric: tokens per run for `context_consumption`;
   - the preserved correctness: the pass rate and every spec criterion
     (`criterion_score` = awarded/possible), where any loss rejects;
   - the sampling (the canonical `case_id`s);
   - the exact `ExecutionParameters`, the reservation estimate and the
     `policy_revision`.
   `max_scenarios` includes the configured controls. If controls leave no
   room for a primary scenario, coverage is unavailable and nothing runs.
7. The baseline is reserved in the cycle's ledger. Its stable `work_id` and
   display label are recorded in the cycle. Only then is
   `e2e::dashboard::execution-start {label, idempotency_key: work_id,
   parameters}` called. An ambiguous answer is reconciled only by that
   immutable key through `executions-list` and is never dispatched again.
   A busy `where: harness` runner started nothing: the same work id is
   dispatched again on the next poll.

## Execution input and analysis

- `e2e::dashboard::changed` is only a hint. The loop re-reads
  `execution-get`.
- An execution is admitted once per execution/run/attempt (compare-and-set),
  and only when its status is terminal and every report slot has reported.
- An execution whose immutable idempotency key names recorded cycle work
  returns to that cycle. An
  execution started through `eval::start` converges on that cycle. Any other
  execution starts one cycle.
- The sweep recovers missed events from a checkpoint. The checkpoint never
  passes an execution that is open or incomplete, or whose admission failed.
  The first sweep only sets it, so history is analyzed through `eval::start`.
- The native bundle is copied into the loop before analysis. Analysis then
  survives teardown and E2E retention.
- The first pass calls no model and starts no campaign. It runs the
  detectors on run transcripts, reads technical, status, completion and
  per-criterion awards, and classifies findings into these classes: defect,
  improvement opportunity, evaluation gap, infrastructure/evaluator failure,
  expected behavior, insufficient evidence.
- Anything not reported is listed as unknown. A run that is `passed` but
  `task_incomplete`, or that lost points, is never expected behavior.
  Child-transcript coverage is never claimed.
- Recorded `runs[].failures[]`, and those of every attempt the E2E retried
  (`runs[].retry_attempts[]`), are forwarded to `sentinel::reports::ingest`
  in batches of 100. A score loss is never turned into an error. Sentinel's
  answer (group, occurrence, classification, triage) is stored in the
  analysis. A run the E2E had to retry did not complete.
- What comes next:
  - pertinent findings or a usable baseline: the cycle waits at stage
    `candidate` with `next.action = candidate_work`;
  - nothing pertinent: outcome `no_action`;
  - an unusable baseline: outcome `inconclusive`.

## Candidate, release and canary

Each part below needs its own optional configuration. When a part is
missing, the cycle stops `inconclusive` with the exact reason, never with a
false result.

- `candidate`: repository, `base_sha`, `worktrees_dir`, the actor's model and
  provider, and its bounds. It is checked by `eval::configure`, and it needs
  `runs_per_side` of at least 5: the sign test accepts nothing on fewer pairs.
- `validation`: `where: docker|github`, `stack_name` and two stack templates
  (`{commit}` and `{version}`) that pin only the Harness.
- `promotion`: `artifact_target` and an explicit activation `target`
  (`engine_url`, `namespace`, `compose_file`), with
  `release_build_estimate_usd`.
- `judge`: `provider` and `estimated_usd`. Only ambiguous improvement
  opportunities are asked, with what each scenario expects and what each
  lost criterion asks. The cycle acts on the answer only when the choice
  leads the next option by 0.2 or more; otherwise it ends `inconclusive`.
  A judge that does not answer is asked again on the next sweep until the
  cycle's time budget runs out.
- `controls`: control scenarios, e.g. `minimal_path`.

The release service origins and credentials come only from the
deployment's environment: `EVAL_RELEASE_CONTROL_URL`, `EVAL_GITHUB_TOKEN` and
`EVAL_RELEASE_CONTROL_SESSION`.

After a measured baseline, or a covered and triaged source execution, the
cycle runs these steps:

1. The read-only proposal produces an evidence-backed hypothesis and change.
2. The isolated candidate actor commits on the base. Its changed paths are
   recorded and it is published to its own branch.
3. A source validation runs under the frozen plan.
4. The source decision is made.
5. `main` is fast-forwarded to exactly that commit.
6. Release Control builds an rc from that commit.
7. The rc artifact is validated and gets its own decision, which binds the
   running executable digest.
8. Finalization republishes the same bytes.
9. The prior executable is validated as the rollback destination, and the
   canary is reserved.
10. The stable version is activated on the target and proved by its digest.
11. A canary runs on the target's own E2E.

The canary ends one of three ways:

- **Held:** the cycle is `promoted`. Any loss of a measured condition is a
  violation; the target's own stack, E2E revision and engine are not
  compared with the baseline's, the activated digest is.
- **Violated:** a proved rollback, then `rolled_back`.
- **Unknown:** the observation stays pending on the activated target until
  a cycle limit, which restores the prior. It is never claimed stable and no
  new canary is started.

A cancel or a budget limit after an activation request restores and proves
the prior executable before the cycle ends. A rollback pin that ends or
runs another executable is pinned again, up to three attempts. See `SPECIFICATION.md` and
`IMPLEMENTATION.md`.

## Decision and budget

Decisions are made once, in code, by the frozen plan (`e2e-loop-rules/5`):

- An incompatible pair, a side that did not run what it had to, a technical
  failure or a missing measure is inconclusive.
- A lost preserved criterion is rejected.
- Only technical failures leave a run unmeasured; an unfinished run is a
  fail.
- Effort counts only on completed, passed runs.
- A cost reduction is never accepted while any cost is unknown.

Every dispatch is reserved first: proposal, actor, judge, E2E runs (counting
retries) and official builds. A reservation that is found again is never
dispatched twice. An unreported cost stays charged at its estimate.
The sweep ends a waiting cycle only once a limit is passed (time at it):
work reserved within the budget runs. The cycle's actors and executions
are then asked to stop. An E2E report cannot release that reservation: its budget charge is at
least the estimate, even when the candidate reports a lower cost. The
reported value remains available separately; an invalid run cost makes
the total unknown.

Lost dispatches reconcile by the E2E's immutable idempotency key. Mutable
labels are display metadata: renames do not lose work, and a matching label
with a different key cannot claim a reservation. Known execution ids cannot
be replaced; duplicate or partial key listings are refused.

## State scopes

| Scope | Contents |
|---|---|
| `eval_cycle_config` | The configuration. |
| `eval_cycle` | The cycle records. |
| `eval_cycle_open` | The ids of non-terminal cycles: what the sweep reads. |
| `eval_cycle_admission` | The admission claims and the checkpoint. |
| `eval_cycle_evidence` | The imported bundles. |
| `eval_cycle_budget` | The ledgers. |

Records of earlier eval versions live in other scopes. They are never read,
migrated or deleted.

## Checks

```
cd eval && SKIP_UI_BUILD=1 cargo test --lib --test flow --test lifecycle --test decision --test budget --test schemas --test manifest --test candidate --test promotion
```

To regenerate the schema goldens, run the same command with
`UPDATE_GOLDENS=1`.

## Limitation in this release: self-reported metrics

`efficiency.total_tokens`, cost and function-call counts come from
`harness::metrics`, which is code a candidate may change. The tokens figure
also counts only input plus output, without cache. These counts therefore
never qualify a source change in this release:

- With candidate work configured, an objective whose planned metric is
  tokens, cost or function calls stops before any baseline or paid actor,
  with outcome `no_coverage` and the exact reason. Context-consumption and
  skills objectives measured in tokens are in this case.
- The metric is never replaced by another one.
- Fixes measured by native, independently assessed E2E criteria (for
  example a lost `criterion_score` from a finished execution) still go
  through the full source, artifact and canary path.
