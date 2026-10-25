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
   first: proposal, candidate actor, judge, each E2E execution and each
   official build. Reservations are keyed by a stable work id, so a
   redelivery or a restart never dispatches twice.
   - An unreported cost stays charged at its estimate.
   - E2E estimates count technical retries.
   - The sweep ends a waiting cycle only past a limit (time at it), never
     with work reserved within it, and stops what the cycle dispatched.
   - Builds use the frozen `release_build_estimate_usd`.
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
   - each side ran what it had to: the base, candidate or rc revision as the
     E2E recorded it, and the frozen stack template with only the commit
     or rc pin differing;
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
9. **Source and artifact are decided separately.** The source decision
   authorizes only the exact-base, non-forced fast-forward of `main`. The rc
   artifact built by Release Control from that exact commit gets its own
   decision under the same plan, from one run that pins every released
   worker's rc. It binds each worker's running executable digest
   (`system_under_test.worker_binaries`, and `harness_binary_sha256` for the
   Harness), which must equal the single executable named after the worker
   inside its rc's Registry tar.gz. Without every digest nothing is
   activated. Finalization must republish the same bytes.
10. **Explicit activation.** Activation only targets the configured local
    Compose target. It is never the operator's own Harness by default.
    - Requested and confirmed are recorded apart.
    - Confirmed means the target runs the approved executable digest.
    - The prior executable must match its Registry artifact before becoming
      the rollback destination.
    - The canary is reserved before anything is activated.
11. **Canary.** The canary runs on the activated target through that
    Engine's own E2E (`where: harness`). Its ownership and route are recorded
    before the dispatch.
    - **Held:** every preserved condition and the planned metric were
      measured, none lost anything (the planned metric's minimum effect
      never excuses a loss), and no recorded failure, retried attempts
      included, was unexpected or of unknown expectedness. Only Held
      promotes. The canary runs on the target's own stack and E2E, so its
      stack digest, E2E revision and engine are not compared with the
      baseline's; the activated executable digest proves what ran.
    - **Violated:** a regression or an unexpected failure. It requests a
      rollback, and the rollback must be proved by the prior digest.
    - **Unknown:** anything else, including an executable digest other than
      the activated one, or no coverage or traffic. The observation stays
      pending on the activated target (never claimed stable, no new E2E is
      started) until a cycle limit turns it into a restoration of the prior.
12. **Never abandon an activation.** The activation's operation id is
    persisted (`cycle.activation_intent`) before the pin is sent. A cancel,
    a budget limit or an activation error after that turns into a recovery (`cycle.recovery`), which
    needs no model or E2E budget:
    - the original operation is reconciled read-only
      (`read_compose_operation`) and never sent again;
    - running or unknown waits; unknown past the reconcile grace is restored
      like a failed operation;
    - failed or cancelled settles the operation but does not prove its
      partial effects were undone; the prior must still be restored;
      succeeded counts only once the candidate's executable is proved, and
      a succeeded pin whose process has exited is restored like a failed
      one;
    - the prior is then pinned and proved by its executable digest; a pin
      that ends or runs another executable is pinned again, up to three
      attempts, before the cycle ends with the target not proved;
    - a Compose call that errors says nothing about the target and is
      asked again on the next sweep; only an observation (the target runs
      neither the prior nor the candidate) refuses a restoration;
    - with several workers, a refused restoration never stops the others:
      every other worker is restored first, then the cycle ends naming
      what was restored and what was refused.

    Until that is proved the cycle is not terminal.
13. **One state, current shape only.** State lives in the `eval_cycle*`
    scopes. Legacy scopes are never read, migrated or deleted.

## Outcomes

| Outcome | Meaning |
|---|---|
| `no_coverage` | The objective or execution has no E2E coverage. |
| `no_action` | The analysis found nothing worth candidate work. |
| `budget` | A limit of the cycle was reached. |
| `inconclusive` | Missing capability or configuration, unknown evidence, isolation not available, credentials unavailable, or an activation whose outcome could not be reconciled. |
| `decided` | A source decision with nothing released. |
| `promoted` | Activated, proved, and the canary held. |
| `rolled_back` | The prior executable is proved running again. |

An accepted source decision alone is never `promoted`.

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
