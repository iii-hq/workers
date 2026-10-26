# eval — implementation

## Modules

| Module | Role |
|---|---|
| `contract.rs` | The one public contract: `LoopConfigV1`, `CycleRecordV1`, evidence, decisions. |
| `functions.rs`, `surface.rs` | Registration and schemas of `eval::configure`, `config`, `start`, `list`, `status`, `result`, `cancel`, plus the internal `step`, `sweep` and `execution-changed`. |
| `runtime.rs` | The cycle machine: start, list, cancel, the durable `step`, coverage, baseline and generic E2E dispatch, import and analysis, admission of executions and the sweep. |
| `stages.rs` | Triage (deterministic, Sentinel reuse, one grouped `judge::evaluate`), the read-only proposal, the candidate actor and its publication, and the source-validation dispatch. |
| `candidate.rs` | Isolated proposal and candidate actors, owned by the candidate module. |
| `pull_request.rs` | The one outward effect: the pull request of an accepted candidate, ready for review, with its evidence body; and the step that opens it. |
| `coverage.rs`, `evidence.rs`, `decision.rs` | Catalog selection, native report reading with the first pass, and the decision rules, owned by the evidence module. |
| `cost.rs`, `state.rs`, `queue.rs`, `ids.rs`, `diagnostics.rs` | The ledger, compare-and-set state, the `eval-run` queue, ids and labels, and transcript detectors. |

## Step discipline

- Every step takes the cycle lock.
- It writes the record with compare-and-set (`state::update_cycle`) behind
  `decision::check_write`.
- It advances `step`, so a stale queued step is skipped.
- Running cycles enqueue their next step.
- Waiting cycles in `baseline`, `proposal`, `candidate`, `validation` and
  `pull_request` are polled by the 15-second sweep.
- `awaiting_execution` waits for `e2e::dashboard::changed` or for the sweep's
  re-read.

## Effects and their identities

| Effect | Identity, persisted before the effect | Reconciliation |
|---|---|---|
| E2E execution | Display label `eval:<cycle_id>:<baseline\|candidate>`, immutable `work_id`. | `work_id` is sent as `idempotency_key` and is the only reconciliation key. Duplicate or partial listings are refused. Declared lost after 10 minutes, never re-dispatched. |
| Proposal / candidate actor | `proposal_identity` / `identity`: session and idempotency key. | `start` reads `harness::status` before any `harness::send`. |
| Judge | Request id stored in `cycle.triage`. | The question has no side effect: after a restart, or any error but `invalid_request` and `payload_too_large`, the sweep asks again under the same id until the cycle's time budget ends it. |
| Candidate publication | Deterministic branch, non-forced push, 30 s bound. | A retry reconciles. |
| Pull request | The candidate branch and its exact head sha. | `ensure` searches the branch's pull requests first: one open or merged for that head is reused, so a lost answer asked again never opens a second. |

## Budget

- Reservation kinds: Investigation (proposal), Implementation (the actor,
  `{cycle}:implementation`, `candidate: true`), Jev, E2e (`{cycle}:<role>`;
  the validation's rounds `{cycle}:candidate-<n>` and `{cycle}:control-<n>`,
  each a share of the plan's estimate). Opening the pull request reserves
  nothing.
- Stage futures are boxed: their state machines exceed a worker thread's
  stack in unoptimized builds.

## GitHub

- The API origin is pinned (`pull_request::GITHUB_API_URL`); tests pin a
  local one through `Deps::with_github_api`.
- The token comes from `EVAL_GITHUB_TOKEN`, read per request. It never
  appears in a payload, a record or an error; missing or expired access
  ends the cycle `decided` with the reason.

## Tests

| Suite | Covers |
|---|---|
| `tests/flow.rs` | Stage 1: coverage, baseline, analysis, admission, checkpoint, cancel, Sentinel forwarding. |
| `tests/lifecycle.rs` | Full trajectories, driven by a finished execution whose native `needles_recovered` criterion lost points (never by a self-reported token count), through controller entrypoints with the real candidate module against local mocks: an accepted candidate proposed in a pull request ready for review with its evidence, a change measured in two workers named in it, a lost answer found on the next sweep and never opened twice, no GitHub access leaving the change on its branch, rejected, wrong revision, cancel during the actor, unisolated Harness, source execution covered then judged once, a judge asked again, uncovered execution, forged or changed candidate roots, a token objective planned and left to the meter rule, a tampered token counter not approved. |
| `tests/budget.rs`, `tests/decision.rs`, `tests/schemas.rs`, `tests/manifest.rs`, `tests/candidate.rs`, `tests/pull_request.rs` | The units and leaf modules. |

None of these is live qualification.
