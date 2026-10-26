# eval — specification

The Harness E2E improvement loop. Harness E2E runs the scenarios. Eval keeps
the cycle, the evidence, the budget and every decision. The central Sentinel
groups the failures that runs recorded. User chats and sessions are never read.

## Inputs

There are exactly two inputs, and both converge on one `CycleRecordV1`:

- `eval::start {objective}`: a free objective.
- `eval::start {execution_id}` or `e2e::dashboard::changed`: a finished E2E
  execution, re-read through `execution-get`.

## Guarantees

1. **Frozen configuration.** The configuration is frozen into the cycle
   (`cycle.config`) when the cycle is created. Its revision is the
   `policy_revision`. Later edits never change the cycle's actor, budget,
   target or criteria.
2. **No spend without coverage.**
   - An objective selects tests only from their spec text (criteria,
     summaries, invariants; never the prompt, whose code and paths match
     anything), in their current executable definition.
   - An execution maps to what its own runs lost, on its executed scenarios.
     They must still be current.
   - Otherwise the outcome is `no_coverage` with the exact gap. Skill and
     provider/model fallback objectives have no coverage today.
     `tool_contract_recovery` is not provider fallback.
3. **Plan before results.** Before any dispatch, the plan freezes:
   - the metric and the controls, e.g. `minimal_path`;
   - every spec criterion, and the pass rate as preserved correctness;
   - the cases;
   - the exact `ExecutionParameters` and the selected test identities;
   - the policy revision.
4. **Budget before work.** Every paid or external dispatch reserves its work
   first: proposal, candidate actor, judge and each E2E execution. Reservations are keyed by a stable work id, so a
   redelivery or a restart never dispatches twice.
   - An unreported cost stays charged at its estimate.
   - E2E estimates count technical retries.
   - The sweep ends a waiting cycle only past a limit (time at it), never
     with work reserved within it, and stops what the cycle dispatched.
5. **Cheap first pass.** Every imported execution gets a deterministic
   analysis. It calls no model. An unknown is listed and never assumed. An
   evaluation gap is the evaluator's to close; it never authorizes a Harness
   change.
6. **Bounded judgment.** `judge::evaluate` is used only for an ambiguous
   improvement opportunity that is already covered, in one grouped question.
   - A deterministic defect needs no judgment.
   - Report-sourced Sentinel groups are excluded from Sentinel's automatic
     paid model sweep; Eval owns their budgeted semantic triage.
   - A lost or unavailable answer leaves that step unknown.
7. **Isolated candidates.**
   - The read-only proposal and the candidate actor run only when
     `harness::filesystem::info` reports `boundary: workspace`. Otherwise the
     cycle stops inconclusive before any dispatch.
   - The actor gets coder read/write/move/delete anywhere in the
     repository worktree except the loop's own `eval/` and git metadata
     (`.git*`, `.github/`). It never gets shell, fp, grants, control or
     credentials.
8. **Decision in code.** A decision is accepted only when:
   - the pair is comparable on behavior, inputs, contract, model, provider,
     E2E revision, engine, cases and policy;
   - each side ran what it had to: the base or candidate revision as the
     E2E recorded it, and the frozen stack template with only the commit
     differing;
   - every unit the candidate changes is built from the candidate commit
     by the source template;
   - a total the changed code reports itself (tokens, cost, function
     calls, counted by `harness`, `llm-router` and `provider-*`) does not
     qualify a change to that code;
   - every preserved criterion holds;
   - the planned metric moved by the minimum effect under the frozen
     sampling and stop rules (`e2e-loop-rules/5`).

   Effort counts only on completed, passed runs. A cost reduction is never
   claimed while a cost is unknown.
9. **The loop ends at a pull request.** An accepted source decision opens a
   pull request from the candidate's branch to `main`, ready for review,
   with the evidence in its body (`pull_request.rs`), and the cycle ends
   `proposed`. Nothing is merged, released or activated by the loop.
   - A pull request already open or merged for the exact candidate head is
     reused, so a lost answer asked again never opens a second one.
   - A lost answer or a GitHub 5xx is asked again on the next sweep, within
     the cycle's limits; a refusal (no token, a branch that moved, a 4xx)
     ends `decided` with the reason and the change left on its branch.
   - The token comes only from `EVAL_GITHUB_TOKEN`, read at each request;
     no reason carries remote text.
10. **One state, current shape only.** State lives in the `eval_cycle*`
    scopes. Legacy scopes are never read, migrated or deleted.

## Outcomes

| Outcome | Meaning |
|---|---|
| `no_coverage` | The objective or execution has no E2E coverage. |
| `no_action` | The analysis found nothing worth candidate work. |
| `budget` | A limit of the cycle was reached. |
| `inconclusive` | Missing capability or configuration, unknown evidence or isolation not available. |
| `decided` | A source decision and no pull request: rejected, inconclusive, or accepted with the reason none was opened. |
| `proposed` | The accepted change is in a pull request ready for review (`pull_request`). |

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
