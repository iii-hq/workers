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
Improve, Prove and Review, with one sentence for what runs now or why a cycle
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

## Candidate and pull request

Each part below needs its own optional configuration. When a part is
missing, the cycle stops `inconclusive` with the exact reason, never with a
false result.

- `candidate`: repository, `base_sha`, `worktrees_dir`, the actor's model and
  provider, and its bounds. It is checked by `eval::configure`, and it needs
  `runs_per_side` of at least 5: the sign test accepts nothing on fewer pairs.
- `validation`: `where: docker|github`, `stack_name` and a stack template
  with `{commit}`. It declares each worker a candidate may change as a
  `package://<worker>` container with `commit: {commit}`, so both sides
  build it from their own commit. A change to a unit it does not build that
  way is never accepted: no run measured it. Neither is a change no
  commit-built worker runs (only documentation, or shared code only unbuilt
  workers read). A dependent the stack does not build is named in the
  decision: not measured.
- `judge`: `provider` and `estimated_usd`. Only ambiguous improvement
  opportunities are asked, with what each scenario expects and what each
  lost criterion asks. The cycle acts on the answer only when the choice
  leads the next option by 0.2 or more; otherwise it ends `inconclusive`.
  A judge that does not answer is asked again on the next sweep until the
  cycle's time budget runs out.
- `controls`: control scenarios, e.g. `minimal_path`.

After a measured baseline, or a covered and triaged source execution, the
cycle runs these steps:

1. The read-only proposal produces an evidence-backed hypothesis and change.
2. The isolated candidate actor commits on the base. It may change any file
   of the repository but the loop's own `eval/` and git metadata. The
   commit records its units (each top-level directory or kit it touches,
   `crates/<c>` and `packages/<p>` for shared code) and its dependents:
   every other unit whose non-test files read changed code, transitively,
   by a relative path (Cargo `path`, `#[path]`, `include_str!`, an import,
   a build script, a symlink) or a `workspace:` package. Its changed paths
   are recorded and it is published to its own branch.
3. A source validation runs under the frozen plan.
4. The source decision is made.
5. An accepted change becomes a pull request from its branch to `main`,
   ready for review, and the cycle ends `proposed`. The body carries the
   evidence: objective, hypothesis and change, each criterion on both sides
   with its runs and result, the E2E executions, the transcript findings,
   where the change runs (measured and not measured) and what the cycle
   spent. A pull request already open or merged for the same head is
   reused. A lost answer is asked again on the next sweep; a refusal (no
   `EVAL_GITHUB_TOKEN`, a branch that moved) ends `decided` with the reason,
   the change left on its branch.

Nothing is merged, released or activated by the loop: a person reviews and
merges. The GitHub token comes only from the deployment's environment
(`EVAL_GITHUB_TOKEN`). See `SPECIFICATION.md` and `IMPLEMENTATION.md`.

## Decision and budget

Decisions are made once, in code, by the frozen plan (`e2e-loop-rules/5`):

- An incompatible pair, a side that did not run what it had to, a technical
  failure or a missing measure is inconclusive.
- A lost preserved criterion is rejected.
- Only technical failures leave a run unmeasured; an unfinished run is a
  fail.
- Effort counts only on completed, passed runs.
- A cost reduction is never accepted while any cost is unknown.

Every dispatch is reserved first: proposal, actor, judge and E2E runs
(counting retries). A reservation that is found again is never
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
cd eval && SKIP_UI_BUILD=1 cargo test --lib --test flow --test lifecycle --test decision --test budget --test schemas --test manifest --test candidate --test pull_request
```

To regenerate the schema goldens, run the same command with
`UPDATE_GOLDENS=1`.

## Self-reported metrics

`efficiency.total_tokens`, cost and function-call counts come from
`harness::metrics`, which counts the usage `llm-router` and the
`provider-*` workers report. The tokens figure counts only input plus
output, without cache.

- A candidate that changes `harness`, `llm-router` or a `provider-*`
  worker changes its own meter: these totals do not qualify it, and the
  decision is inconclusive with the exact reason.
- A change anywhere else (for example `context-manager` or a kit) is
  qualified by these totals like by any other metric.
- The metric is never replaced by another one.
