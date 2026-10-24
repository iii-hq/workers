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
   - An objective selects tests only from their spec text (criteria, summary,
     prompt, invariants), in their current executable definition.
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
   - Builds use the frozen `release_build_estimate_usd`.
5. **Cheap first pass.** Every imported execution gets a deterministic
   analysis. It calls no model. An unknown is listed and never assumed. An
   evaluation gap is the evaluator's to close; it never authorizes a Harness
   change.
6. **Bounded judgment.** `judge::evaluate` is used only for an ambiguous
   improvement opportunity that is already covered, in one grouped question.
   - A deterministic defect needs no judgment.
   - Sentinel triage of the same occurrence is reused.
   - Report-sourced Sentinel groups are excluded from Sentinel's automatic
     paid model sweep; Eval owns their budgeted semantic triage.
   - A lost or unavailable answer leaves that step unknown.
7. **Isolated candidates.**
   - The read-only proposal and the candidate actor run only when
     `harness::filesystem::info` reports `boundary: workspace`. Otherwise the
     cycle stops inconclusive before any dispatch.
   - The actor gets coder read/write inside `harness/` only. It never gets
     shell, fp, grants, control or credentials.
8. **Decision in code.** A decision is accepted only when:
   - the pair is comparable on behavior, inputs, contract, model, provider,
     E2E revision, engine, cases and policy;
   - each side ran what it had to: the base, candidate or rc revision as the
     E2E recorded it, and the frozen stack template with only the Harness
     pin differing;
   - every preserved criterion holds;
   - the planned metric moved by the minimum effect under the frozen
     sampling and stop rules (`e2e-loop-rules/5`).

   Effort counts only on completed, passed runs. A cost reduction is never
   claimed while a cost is unknown.
9. **Source and artifact are decided separately.** The source decision
   authorizes only the exact-base, non-forced fast-forward of `main`. The rc
   artifact built by Release Control from that exact commit gets its own
   decision under the same plan. It binds the running executable digest
   (`system_under_test.harness_binary_sha256`), which must equal the single
   `harness` executable inside the rc's Registry tar.gz. Without that digest
   nothing is activated. Finalization must republish the same bytes.
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
      measured, none regressed, and no recorded failure was unexpected or of
      unknown expectedness. Only Held promotes.
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
    - running or unknown waits; unknown past the reconcile grace ends
      `inconclusive` with the target not proved, never restored by
      assumption;
    - failed or cancelled settles the operation but does not prove its
      partial effects were undone; the prior must still be restored;
      succeeded counts only once the candidate's executable is proved;
    - the prior is then pinned and proved by its executable digest.

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
