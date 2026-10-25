# eval — implementation

## Modules

| Module | Role |
|---|---|
| `contract.rs` | The one public contract: `LoopConfigV1`, `CycleRecordV1`, evidence, decisions. |
| `functions.rs`, `surface.rs` | Registration and schemas of `eval::configure`, `config`, `start`, `list`, `status`, `result`, `cancel`, plus the internal `step`, `sweep` and `execution-changed`. |
| `runtime.rs` | The cycle machine: start, list, cancel, the durable `step`, coverage, baseline and generic E2E dispatch, import and analysis, admission of executions, the sweep, and recovery after a cancel or a budget limit. |
| `stages.rs` | Triage (deterministic, Sentinel reuse, one grouped `judge::evaluate`), the read-only proposal, the candidate actor and its publication, and the source-validation dispatch. |
| `release.rs` | The promotion driver: one journaled `promotion.rs` effect per step, the artifact run, the canary on the target Engine, canary judgment, rollback and recovery. |
| `candidate.rs` | Isolated proposal and candidate actors, owned by the candidate module. |
| `promotion.rs` | GitHub fast-forward, Release Control, Registry and target Compose effects, owned by the promotion module. |
| `coverage.rs`, `evidence.rs`, `decision.rs` | Catalog selection, native report reading with the first pass, and the decision rules, owned by the evidence module. |
| `cost.rs`, `state.rs`, `queue.rs`, `ids.rs`, `diagnostics.rs` | The ledger, compare-and-set state, the `eval-run` queue, ids and labels, and transcript detectors. |

## Step discipline

- Every step takes the cycle lock.
- It writes the record with compare-and-set (`state::update_cycle`) behind
  `decision::check_write`.
- It advances `step`, so a stale queued step is skipped.
- Running cycles enqueue their next step.
- Waiting cycles in `baseline`, `proposal`, `candidate`, `validation` and
  `promotion` are polled by the 15-second sweep.
- `awaiting_execution` waits for `e2e::dashboard::changed` or for the sweep's
  re-read.

## Effects and their identities

| Effect | Identity, persisted before the effect | Reconciliation |
|---|---|---|
| E2E execution | Display label `eval:<cycle_id>:<baseline\|candidate\|artifact\|canary>`, immutable `work_id` and route. | `work_id` is sent as `idempotency_key` and is the only reconciliation key. Duplicate or partial listings are refused. Declared lost after 10 minutes, never re-dispatched. |
| Proposal / candidate actor | `proposal_identity` / `identity`: session and idempotency key. | `start` reads `harness::status` before any `harness::send`. |
| Judge | Request id stored in `cycle.triage`. | The question has no side effect: after a restart, or any error but `invalid_request` and `payload_too_large`, the sweep asks again under the same id until the cycle's time budget ends it. |
| Candidate publication | Deterministic branch, non-forced push, 30 s bound. | A retry reconciles. |
| GitHub merge | `fast_forward_main`: candidate parent equals the approved base, non-forced. | Proved by `read_merge`. |
| Release Control | `rc_planned` / `finalize_planned` journaled before `confirm_plan`. | A repeated confirm returns the same operation. |
| Compose | `compose_operation_id(cycle, action, version, attempt)`; the activation id is persisted in `cycle.activation_intent` before the pin. | First dispatch: `request_*` reconciles `compose::operation` first. Recovery: read-only `read_compose_operation`, never a re-send. `confirm_running` hashes the Harness process under the supervised pid. |

## Budget

- Reservation kinds: Investigation (proposal), Implementation (the actor,
  `{cycle}:implementation`, `candidate: true`), Jev, E2e (`{cycle}:<role>`,
  the candidate's validation run `{cycle}:candidate`), Build.
- The canary is reserved before activation.
- Recovery after an activation reserves nothing.
- Stage futures are boxed: their state machines exceed a worker thread's
  stack in unoptimized builds.

## Release service origins

- Release Control comes from `EVAL_RELEASE_CONTROL_URL` (https only).
  GitHub and the Registry are pinned.
- Credentials come from `EVAL_GITHUB_TOKEN` and
  `EVAL_RELEASE_CONTROL_SESSION`, read per request.
- None of these appear in a payload, a record or an error. Missing or
  expired access stops the cycle as `unavailable`, then `inconclusive`.
- Tests pin local origins through `Deps::with_release_origins`.

## Tests

| Suite | Covers |
|---|---|
| `tests/flow.rs` | Stage 1: coverage, baseline, analysis, admission, checkpoint, cancel, Sentinel forwarding. |
| `tests/lifecycle.rs` | Full trajectories, driven by a finished execution whose native `needles_recovered` criterion lost points (never by a self-reported token count), through controller entrypoints with the real candidate and promotion modules against local mocks: promoted, rolled back (violated criterion or primary regression), unknown canary (other digest or failures of unknown expectedness), rejected, wrong revision, missing credentials, no promotion configured, cancel during the actor, cancel after activation, budget limit during the canary, canary budget unavailable, unisolated Harness, source execution covered then judged once, uncovered execution, a token objective planned and left to the meter rule, a tampered token counter not approved. |
| `tests/budget.rs`, `tests/decision.rs`, `tests/schemas.rs`, `tests/manifest.rs`, `tests/candidate.rs`, `tests/promotion.rs` | The units and leaf modules. |

None of these is live qualification.
