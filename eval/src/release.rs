//! Promotion of an accepted source candidate, one journaled effect per step:
//! guarded exact-head merge, the official Release Control rc build from the
//! exact merged commit, the final comparison of that immutable rc artifact
//! under the same frozen plan, finalization reusing the rc bytes, explicit
//! local Compose activation proved by the running executable's digest, a
//! canary, and a proved rollback when the canary is violated.
//!
//! Every effect is in `crate::promotion`; this module only decides which
//! effect comes next from the journal, persists each phase before the next
//! effect, reserves builds and E2E runs, and never claims a state the
//! journal did not prove.

use crate::contract::*;
use crate::decision;
use crate::error::EvalError;
use crate::promotion::{
    check_artifact_approval, check_republished, compose_operation_id, ApprovalOutcome,
    ArtifactApproval, Confirmation, DeploymentEvidence, OperationState, Promotion, PromotionError,
    PromotionJournal, PromotionPhase, ReleaseArtifact, ReleasePlan, SourceApproval, StopKind,
    Stopped, ValidatedArtifact,
};
use std::sync::Arc;

use iii_sdk::IIIClient;
use serde_json::{json, Value};

use crate::evidence::{self, Readiness};
use crate::promotion::ActivationTarget;
use crate::runtime::{
    call_on, claim_of, dispatch, end, import, reserve, settle, transition, transition_with, Deps,
    BUS_TIMEOUT_MS, RECONCILE_GRACE_MS, START_TIMEOUT_MS,
};
use crate::{cost, ids, state};

fn journal_of(record: &CycleRecordV1) -> PromotionJournal {
    record
        .promotion
        .clone()
        .unwrap_or_else(|| PromotionJournal::new(record.cycle_id.clone()))
}

fn find<T>(journal: &PromotionJournal, pick: impl Fn(&PromotionPhase) -> Option<T>) -> Option<T> {
    journal.phases.iter().rev().find_map(pick)
}

fn approval_of(journal: &PromotionJournal) -> Option<SourceApproval> {
    find(journal, |phase| match phase {
        PromotionPhase::SourceApproved(approval) => Some(approval.clone()),
        _ => None,
    })
}

fn rc_of(journal: &PromotionJournal) -> Option<ReleaseArtifact> {
    find(journal, |phase| match phase {
        PromotionPhase::RcPublished(artifact) => Some(artifact.clone()),
        _ => None,
    })
}

fn rc_plan_of(journal: &PromotionJournal) -> Option<ReleasePlan> {
    find(journal, |phase| match phase {
        PromotionPhase::RcPlanned(plan) => Some(plan.clone()),
        _ => None,
    })
}

fn stable_of(journal: &PromotionJournal) -> Option<ReleaseArtifact> {
    find(journal, |phase| match phase {
        PromotionPhase::StablePublished(artifact) => Some(artifact.clone()),
        _ => None,
    })
}

fn prior_of(journal: &PromotionJournal) -> Option<ValidatedArtifact> {
    find(journal, |phase| match phase {
        PromotionPhase::RollbackDestination(prior) => Some(prior.clone()),
        _ => None,
    })
}

fn validated_rc(journal: &PromotionJournal) -> Result<ValidatedArtifact, PromotionError> {
    let approval = find(journal, |phase| match phase {
        PromotionPhase::ArtifactApproved(approval) => Some(approval.clone()),
        _ => None,
    })
    .ok_or_else(|| PromotionError::inconclusive("no artifact decision"))?;
    let rc = rc_of(journal).ok_or_else(|| PromotionError::inconclusive("no rc artifact"))?;
    check_artifact_approval(&approval, &rc)
}

fn validated_stable(journal: &PromotionJournal) -> Result<ValidatedArtifact, PromotionError> {
    let stable =
        stable_of(journal).ok_or_else(|| PromotionError::inconclusive("no stable artifact"))?;
    check_republished(&validated_rc(journal)?, &stable)
}

/// An activation was requested and no rollback has been proved yet: a
/// cancel or a budget limit must restore the prior, never abandon it.
pub fn activation_in_flight(record: &CycleRecordV1) -> bool {
    record.promotion.as_ref().is_some_and(|journal| {
        (record.activation_intent.is_some()
            || journal
                .phases
                .iter()
                .any(|phase| matches!(phase, PromotionPhase::ActivationRequested(_))))
            && !journal
                .phases
                .iter()
                .any(|phase| matches!(phase, PromotionPhase::RollbackConfirmed(_)))
    })
}

/// The read-only reconciliation of an activation operation during a
/// recovery: running or unknown waits (unknown ends unproved after the
/// grace, never assumed); failed or cancelled settles the operation, so its
/// partial effects can be restored by pinning and proving the prior;
/// succeeded is activation only once the candidate's executable is proved.
async fn reconcile_activation(
    promotion: &Promotion,
    promotion_id: &str,
    prior: &ValidatedArtifact,
    stable: &ValidatedArtifact,
    operation: &str,
    since_ms: i64,
    reason: &str,
) -> Result<Next, PromotionError> {
    use crate::promotion::ComposeOperationState as Op;
    let wait = |what: &str| {
        Next::Wait(format!(
            "restoration waits ({reason}): activation {operation} is {what}"
        ))
    };
    Ok(match promotion.read_compose_operation(operation).await? {
        Op::Running => wait("running"),
        Op::Unknown if since_ms < RECONCILE_GRACE_MS => wait("unknown to Compose"),
        // Past the grace Compose no longer holds the operation (a restarted
        // daemon): the prior is pinned back and proved like after a failure,
        // never left unproved.
        Op::Unknown | Op::Failed | Op::Cancelled => {
            let rollback = compose_operation_id(promotion_id, "rollback", &prior.version, 1);
            Next::Phase(PromotionPhase::RollbackRequested(
                promotion.request_rollback(prior, stable, &rollback).await?,
            ))
        }
        Op::Succeeded => match promotion.confirm_running(stable, operation).await? {
            Confirmation::Confirmed(running) => {
                Next::Phase(PromotionPhase::ActivationConfirmed(running))
            }
            Confirmation::Pending { reason: pending } => {
                wait(&format!("succeeded, not yet proved: {pending}"))
            }
        },
    })
}

/// Completes the restoration of a recovering cycle: reconcile the
/// requested activation, pin the prior back and prove its executable
/// running. Needs no model or E2E budget.
async fn recover(
    promotion: &Promotion,
    journal: &PromotionJournal,
    intent: Option<&str>,
    since_ms: i64,
    reason: &str,
) -> Result<Next, PromotionError> {
    let prior =
        prior_of(journal).ok_or_else(|| PromotionError::inconclusive("no rollback destination"))?;
    let stable = validated_stable(journal)?;
    let id = &journal.promotion_id;
    let restored = restore_step(
        promotion, journal, intent, since_ms, reason, &prior, &stable,
    )
    .await;
    let Err(mut error) = restored else {
        return restored;
    };
    // A pin that ended, or an executable other than the validated one: pin
    // the prior again under a fresh attempt before giving up. A journaled
    // attempt is never asked again, and one that ended before it was
    // journaled is only read (refused) and skipped.
    for attempt in 1..=MAX_ROLLBACK_ATTEMPTS {
        if error.kind != StopKind::Rejected {
            break;
        }
        let operation = compose_operation_id(id, "rollback", &prior.version, attempt);
        if journal.phases.iter().any(|phase| {
            matches!(phase, PromotionPhase::RollbackRequested(request) if request.operation_id == operation)
        }) {
            continue;
        }
        match promotion
            .request_rollback(&prior, &stable, &operation)
            .await
        {
            Ok(request) => return Ok(Next::Phase(PromotionPhase::RollbackRequested(request))),
            Err(refused) => error = refused,
        }
    }
    Err(error)
}

/// Rollback attempts a recovery makes before it stops unproved.
const MAX_ROLLBACK_ATTEMPTS: u32 = 3;

async fn restore_step(
    promotion: &Promotion,
    journal: &PromotionJournal,
    intent: Option<&str>,
    since_ms: i64,
    reason: &str,
    prior: &ValidatedArtifact,
    stable: &ValidatedArtifact,
) -> Result<Next, PromotionError> {
    let id = &journal.promotion_id;
    Ok(match journal.last() {
        // The pin may have been sent before its record was written: complete
        // the idempotent request under the same operation id (it reconciles
        // an operation Compose already holds and never sends a second one),
        // so its outcome is known before anything is restored.
        // A pin whose record may be lost (only the intent was persisted), or
        // a requested activation, is reconciled READ-ONLY: its operation is
        // read and never sent again. While it may still change the target
        // nothing is pinned back and nothing is claimed restored.
        Some(PromotionPhase::RollbackDestination(_)) if intent.is_some() => {
            reconcile_activation(promotion, id, prior, stable, intent.unwrap_or_default(), since_ms, reason).await?
        }
        // A request recorded as not sent (the target already ran it) needs
        // no operation guard: the observation is the proof.
        Some(PromotionPhase::ActivationRequested(request)) if !request.accepted => {
            match promotion.confirm_running(stable, &request.operation_id).await? {
                Confirmation::Confirmed(running) => Next::Phase(PromotionPhase::ActivationConfirmed(running)),
                Confirmation::Pending { reason: pending } => {
                    Next::Wait(format!("restoration waits ({reason}): {pending}"))
                }
            }
        }
        Some(PromotionPhase::ActivationRequested(request)) => {
            reconcile_activation(promotion, id, prior, stable, &request.operation_id, since_ms, reason).await?
        }
        Some(PromotionPhase::RollbackRequested(request)) => {
            match promotion.confirm_running(prior, &request.operation_id).await? {
                Confirmation::Confirmed(running) => Next::Phase(PromotionPhase::RollbackConfirmed(running)),
                Confirmation::Pending { reason: pending } => {
                    Next::Wait(format!("restoration pending ({reason}): {pending}"))
                }
            }
        }
        Some(PromotionPhase::RollbackConfirmed(running)) => Next::Finish(
            OutcomeCodeV1::RolledBack,
            format!(
                "{reason}; prior {} proved running again (executable {})",
                running.version, running.binary_sha256
            ),
        ),
        Some(PromotionPhase::ActivationConfirmed(_)) => {
            let operation = compose_operation_id(id, "rollback", &prior.version, 1);
            Next::Phase(PromotionPhase::RollbackRequested(
                promotion.request_rollback(prior, stable, &operation).await?,
            ))
        }
        _ => {
            return Err(PromotionError::inconclusive(format!(
                "{reason}: the activation state cannot be restored automatically; the target's running artifact is not proved"
            )))
        }
    })
}

/// The rc version the artifact comparison must have run.
pub fn rc_version(record: &CycleRecordV1) -> Option<String> {
    rc_of(record.promotion.as_ref()?).map(|artifact| artifact.version)
}

/// The executable digest the activation proved and the canary must have
/// run.
/// A native E2E digest (`sha256:<hex>`) as the promotion records digests
/// (bare lowercase hex).
fn bare(digest: &str) -> String {
    digest
        .strip_prefix("sha256:")
        .unwrap_or(digest)
        .to_ascii_lowercase()
}

pub fn stable_binary(record: &CycleRecordV1) -> Option<String> {
    validated_stable(record.promotion.as_ref()?)
        .ok()
        .map(|artifact| artifact.binary_sha256)
}

/// The stable version the canary must have run.
pub fn stable_version(record: &CycleRecordV1) -> Option<String> {
    stable_of(record.promotion.as_ref()?).map(|artifact| artifact.version)
}

/// What the canary cannot share with the baseline by construction: it runs on
/// the target's own stack and E2E (a Source-mode identity there), never the
/// pinned validation stack. The activated executable digest, checked first,
/// is the proof of what ran.
const TARGET_ENVIRONMENT: [&str; 3] = ["fixed_stack_digest", "e2e_revision", "engine_version"];

/// The canary judged against the baseline under the frozen plan: every
/// preserved condition must be measured and hold on the activated artifact,
/// and no run failure Sentinel classified as unexpected may appear.
/// Incomparable or unmeasured is unknown, never held.
pub fn canary(
    plan: &CyclePlanV1,
    baseline: &DecisionSideV1,
    canary: &DecisionSideV1,
    evidence: &[EvidenceV1],
    analysis: &CycleAnalysisV1,
    expected_binary: Option<&str>,
    now: i64,
) -> CanaryV1 {
    // Any loss of the planned metric is a regression on the canary: the
    // minimum effect qualified the gain, it never excuses a loss.
    let mut held = plan.clone();
    held.criterion.min_effect = 0.0;
    let judged = decision::decide(&held, baseline, canary, evidence, now);
    let comparable = judged
        .comparability
        .checks
        .iter()
        .all(|check| check.matches || TARGET_ENVIRONMENT.contains(&check.field.as_str()));
    let ran_bare = canary.identity.harness_binary_sha256.as_deref().map(bare);
    let ran = ran_bare.as_deref();
    let unexpected = analysis
        .sentinel
        .iter()
        .flat_map(|sentinel| sentinel.results.iter())
        .filter(|result| {
            matches!(
                result["classification"].as_str(),
                Some("unexpected" | "expected_mismatch")
            )
        })
        .count();
    // A recorded failure whose expectedness Sentinel could not establish (or
    // never answered for) is unknown, never a held canary.
    let failures_unknown = analysis.sentinel.as_ref().is_some_and(|forward| {
        forward.error.is_some()
            || forward.results.len() < forward.observations as usize
            || forward
                .results
                .iter()
                .any(|result| !matches!(result["classification"].as_str(), Some("expected")))
    });
    let primary = &judged.primary;
    let (outcome, reason) = if expected_binary.is_none() || ran != expected_binary {
        (
            CanaryOutcomeV1::Unknown,
            format!(
                "the canary ran executable {}, not the activated {}",
                ran.unwrap_or("unknown"),
                expected_binary.unwrap_or("unknown")
            ),
        )
    } else if unexpected > 0 {
        (
            CanaryOutcomeV1::Violated,
            format!("{unexpected} run failure(s) Sentinel classified as unexpected"),
        )
    } else if let Some(lost) = judged
        .preserved
        .iter()
        .find(|condition| condition.computed_outcome == ValidationOutcomeV1::Regression)
    {
        (
            CanaryOutcomeV1::Violated,
            format!("a preserved condition regressed: {}", lost.reason),
        )
    } else if primary.computed_outcome == ValidationOutcomeV1::Regression {
        (
            CanaryOutcomeV1::Violated,
            format!("the planned metric regressed: {}", primary.reason),
        )
    } else if !comparable {
        (CanaryOutcomeV1::Unknown, judged.reason.clone())
    } else if failures_unknown {
        (
            CanaryOutcomeV1::Unknown,
            "run failures were recorded and their expectedness is not established".into(),
        )
    } else if let Some(unmeasured) = judged
        .preserved
        .iter()
        .find(|condition| condition.computed_outcome == ValidationOutcomeV1::Inconclusive)
    {
        (
            CanaryOutcomeV1::Unknown,
            format!("a preserved condition is unmeasured: {}", unmeasured.reason),
        )
    } else if primary.candidate.mean.is_none() || primary.candidate.n < plan.criterion.min_runs {
        (
            CanaryOutcomeV1::Unknown,
            format!(
                "the planned metric is unmeasured on the canary: {}",
                primary.reason
            ),
        )
    } else {
        // Measured and not regressed; the gain itself is not proved again.
        (
            CanaryOutcomeV1::Held,
            format!(
                "every preserved condition held and the planned metric did not regress ({:?}: {})",
                primary.computed_outcome, primary.reason
            ),
        )
    };
    CanaryV1 {
        execution_id: canary.execution_id.clone(),
        outcome,
        reason,
    }
}

enum Next {
    /// Persist this phase and continue at once.
    Phase(PromotionPhase),
    /// Nothing proved yet: poll again on the sweep.
    Wait(String),
    /// Dispatch an E2E execution (`name`, role, version under test).
    Run(&'static str, ExecutionRoleV1, String),
    /// The canary on the target completed and was imported: analyze it.
    Imported(String, AdmissionV1),
    /// The canary held or the rollback was proved: finish.
    Finish(OutcomeCodeV1, String),
}

fn step_name(phase: Option<&PromotionPhase>) -> &'static str {
    match phase {
        None => "source_approval",
        Some(PromotionPhase::SourceApproved(_)) => "pull_request",
        Some(PromotionPhase::PullRequest(_)) => "merge_request",
        Some(PromotionPhase::MergeRequested(_)) => "merge_read",
        Some(PromotionPhase::Merged(_)) => "rc_preview",
        Some(PromotionPhase::RcPlanned(_)) => "rc_confirm",
        Some(PromotionPhase::RcConfirmed(_)) => "rc_operation",
        Some(PromotionPhase::RcPublished(_)) => "artifact_validation",
        Some(PromotionPhase::ArtifactApproved(_)) => "finalize_preview",
        Some(PromotionPhase::FinalizePlanned(_)) => "finalize_confirm",
        Some(PromotionPhase::FinalizeConfirmed(_)) => "finalize_operation",
        Some(PromotionPhase::StablePublished(_)) => "rollback_destination",
        Some(PromotionPhase::RollbackDestination(_)) => "activation_request",
        Some(PromotionPhase::ActivationRequested(_)) => "activation_confirm",
        Some(PromotionPhase::ActivationConfirmed(_)) => "canary",
        Some(PromotionPhase::RollbackRequested(_)) => "rollback_confirm",
        Some(PromotionPhase::RollbackConfirmed(_)) => "rolled_back",
        Some(PromotionPhase::Stopped(_)) => "stopped",
    }
}

fn work(record: &CycleRecordV1, name: &str) -> CycleWorkV1 {
    // The official build reports no cost: it stays charged at the frozen
    // estimate.
    CycleWorkV1 {
        work_id: format!("{}:{name}", record.cycle_id),
        kind: CycleWorkKindV1::Build,
        estimated_usd: record
            .config
            .as_ref()
            .and_then(|stored| stored.config.release_build_estimate_usd)
            .unwrap_or(f64::NAN),
        candidate: false,
    }
}

const CANARY: &str = "canary";

/// The canary runs on the activated target itself, through that Engine's
/// own E2E (`where: harness`), never on a fresh copy. Its route and label
/// are recorded before the dispatch; it is polled and imported through the
/// target. A missing target E2E or an unknown result is unknown, never held.
async fn canary_on_target(
    deps: &Deps,
    record: &CycleRecordV1,
    client: &Arc<IIIClient>,
    target: &ActivationTarget,
) -> Result<Next, PromotionError> {
    // No coverage or traffic keeps the observation pending (no new E2E is
    // started to try again); a limit turns it into a restoration.
    let unknown = |reason: String| {
        Next::Wait(format!(
            "activated; the canary is unknown, observation pending (never claimed stable): {reason}"
        ))
    };
    let as_unknown =
        |error: crate::error::EvalError| PromotionError::inconclusive(error.to_string());
    let label = ids::execution_label(&record.cycle_id, CANARY);
    let work_id = format!("{}:{CANARY}", record.cycle_id);
    let namespace = target.namespace.as_str();
    let plan = record
        .plan
        .clone()
        .ok_or_else(|| PromotionError::inconclusive("no plan"))?;
    let entry = record
        .executions
        .iter()
        .find(|entry| entry.work_id.as_deref() == Some(work_id.as_str()))
        .cloned();
    let set_id = |execution_id: String| {
        let work_id = work_id.clone();
        async move {
            state::update_cycle(&deps.iii, &record.cycle_id, |stored| {
                let mut next = stored.clone();
                if let Some(entry) = next
                    .executions
                    .iter_mut()
                    .find(|entry| entry.work_id.as_deref() == Some(work_id.as_str()))
                {
                    entry.execution_id = Some(execution_id.clone());
                    entry.state = ExecutionStateV1::Running;
                }
                Ok((Some(next), ()))
            })
            .await
        }
    };
    let Some(entry) = entry else {
        let work = CycleWorkV1 {
            work_id: work_id.clone(),
            kind: CycleWorkKindV1::E2e,
            estimated_usd: plan.estimated_usd,
            candidate: false,
        };
        if let cost::Reservation::OverLimit(detail) =
            reserve(deps, record, &work).await.map_err(as_unknown)?
        {
            return Ok(unknown(format!("budget: {detail}")));
        }
        let now = ids::now_ms();
        state::update_cycle(&deps.iii, &record.cycle_id, |stored| {
            let mut next = stored.clone();
            next.executions.push(CycleExecutionV1 {
                role: ExecutionRoleV1::Canary,
                state: ExecutionStateV1::Dispatching,
                execution_id: None,
                label: Some(label.clone()),
                work_id: Some(work.work_id.clone()),
                dispatched_at: Some(now),
                admission_key: None,
                detail: None,
                route: Some(ExecutionRouteV1 {
                    engine_url: target.engine_url.clone(),
                    namespace: target.namespace.clone(),
                }),
            });
            Ok((Some(next), ()))
        })
        .await
        .map_err(as_unknown)?;
        let parameters = ExecutionParametersV1 {
            where_: WhereV1::Harness,
            stack: None,
            ..plan.parameters.clone()
        };
        return match call_on(
            client,
            namespace,
            "e2e::dashboard::execution-start",
            json!({ "label": label, "idempotency_key": work.work_id, "parameters": parameters }),
            START_TIMEOUT_MS,
        )
        .await
        {
            Ok(response) => match response["execution_id"].as_str() {
                Some(execution_id) => {
                    set_id(execution_id.to_string()).await.map_err(as_unknown)?;
                    Ok(Next::Wait(format!(
                        "canary {execution_id} running on the target"
                    )))
                }
                None => Ok(Next::Wait(
                    "the target E2E answered without an id; reconciling".into(),
                )),
            },
            Err(crate::error::EvalError::Unanswered(error)) => Ok(Next::Wait(format!(
                "canary dispatch unanswered ({error}); reconciling"
            ))),
            // The target's runner holds another execution: nothing started,
            // the next poll dispatches the same reserved canary again.
            Err(error) if crate::runtime::runner_busy(&error) => {
                state::update_cycle(&deps.iii, &record.cycle_id, |stored| {
                    let mut next = stored.clone();
                    next.executions
                        .retain(|entry| entry.work_id.as_deref() != Some(work.work_id.as_str()));
                    Ok((Some(next), ()))
                })
                .await
                .map_err(as_unknown)?;
                Ok(Next::Wait(
                    "the target E2E runner is busy; the canary waits for it".into(),
                ))
            }
            Err(error) => {
                settle(deps, &record.cycle_id, &work.work_id, Some(0.0))
                    .await
                    .map_err(as_unknown)?;
                Ok(unknown(format!(
                    "the target E2E did not start the canary: {error}"
                )))
            }
        };
    };
    let execution_id = match entry.execution_id.clone() {
        Some(execution_id) => execution_id,
        None => {
            let listed = call_on(
                client,
                namespace,
                "e2e::dashboard::executions-list",
                json!({ "query": work_id, "limit": 100 }),
                BUS_TIMEOUT_MS,
            )
            .await
            .map_err(as_unknown)?;
            let found = evidence::unique_execution_id(&listed, &work_id);
            return match found {
                Some(execution_id) => {
                    set_id(execution_id.clone()).await.map_err(as_unknown)?;
                    Ok(Next::Wait(format!(
                        "canary {execution_id} found by its idempotency key"
                    )))
                }
                None if ids::now_ms() - entry.dispatched_at.unwrap_or(record.updated_at)
                    < RECONCILE_GRACE_MS =>
                {
                    Ok(Next::Wait("canary dispatch unanswered; reconciling".into()))
                }
                None => Ok(unknown("the canary never appeared on the target".into())),
            };
        }
    };
    let bundle: Value = match call_on(
        client,
        namespace,
        "e2e::dashboard::execution-get",
        json!({ "execution_id": execution_id }),
        BUS_TIMEOUT_MS,
    )
    .await
    {
        Ok(bundle) => bundle,
        Err(error) => return Ok(unknown(format!("the target E2E cannot be read: {error}"))),
    };
    let Some(summary) = evidence::summary(&bundle, &execution_id) else {
        return Ok(unknown(
            "the target E2E did not return the canary summary".into(),
        ));
    };
    if evidence::text_at(summary, "/idempotency_key").as_deref() != Some(work_id.as_str()) {
        return Ok(unknown(
            "the target E2E canary has a different idempotency key".into(),
        ));
    }
    match evidence::readiness(&bundle, &execution_id) {
        Readiness::Open(why) => Ok(Next::Wait(format!("canary {execution_id}: {why}"))),
        Readiness::Abnormal(why) => Ok(unknown(format!("canary {execution_id}: {why}"))),
        Readiness::Complete => {
            let claim = claim_of(
                &bundle,
                &execution_id,
                &record.cycle_id,
                ExecutionRoleV1::Canary,
            );
            let (stored, _, _) = import(deps, &bundle, &claim).await.map_err(as_unknown)?;
            Ok(Next::Imported(label, stored))
        }
    }
}

/// Decides and performs the next promotion effect from the journal.
async fn next(
    deps: &Deps,
    record: &CycleRecordV1,
    promotion: &Promotion,
    client: &Arc<IIIClient>,
    journal: &PromotionJournal,
) -> Result<Next, PromotionError> {
    let approval =
        || approval_of(journal).ok_or_else(|| PromotionError::inconclusive("no source approval"));
    let pull = || {
        find(journal, |phase| match phase {
            PromotionPhase::PullRequest(pull) => Some(pull.clone()),
            _ => None,
        })
        .ok_or_else(|| PromotionError::inconclusive("no pull request"))
    };
    let id = &journal.promotion_id;
    if let Some(reason) = &record.recovery {
        // How long the recovery has run, from its own history event.
        let started = record
            .history
            .iter()
            .rev()
            .find(|event| event.detail.contains("restoring the prior executable"))
            .map_or(record.updated_at, |event| event.at);
        return recover(
            promotion,
            journal,
            record.activation_intent.as_deref(),
            ids::now_ms() - started,
            reason,
        )
        .await;
    }
    Ok(match journal.last() {
        None => {
            let decision = record
                .decision
                .as_ref()
                .ok_or_else(|| PromotionError::inconclusive("no source decision"))?;
            let state = record
                .candidate
                .as_ref()
                .ok_or_else(|| PromotionError::inconclusive("no candidate"))?;
            let commit = state
                .candidate
                .as_ref()
                .and_then(|candidate| candidate.commit.clone())
                .ok_or_else(|| PromotionError::inconclusive("no candidate commit"))?;
            let candidate_tree = state
                .candidate_tree
                .clone()
                .ok_or_else(|| PromotionError::inconclusive("no candidate tree"))?;
            Next::Phase(PromotionPhase::SourceApproved(SourceApproval {
                candidate_id: state.identity.candidate_id.clone(),
                outcome: match decision.outcome {
                    DecisionOutcomeV1::Accepted => ApprovalOutcome::Accepted,
                    DecisionOutcomeV1::Rejected => ApprovalOutcome::Rejected,
                    DecisionOutcomeV1::Inconclusive => ApprovalOutcome::Inconclusive,
                },
                policy_revision: decision.policy_revision.clone(),
                base_sha: commit.base_sha,
                candidate_sha: commit.candidate_sha,
                candidate_tree,
                evidence_urls: vec![
                    format!("e2e:execution:{}", decision.baseline_execution_id),
                    format!("e2e:execution:{}", decision.candidate_execution_id),
                ],
            }))
        }
        Some(PromotionPhase::SourceApproved(approval)) => Next::Phase(PromotionPhase::PullRequest(
            promotion.ensure_pull_request(approval).await?,
        )),
        Some(PromotionPhase::PullRequest(pr)) => Next::Phase(PromotionPhase::MergeRequested(
            promotion.fast_forward_main(&approval()?, pr.number).await?,
        )),
        Some(PromotionPhase::MergeRequested(_)) => Next::Phase(PromotionPhase::Merged(
            promotion.read_merge(&approval()?, pull()?.number).await?,
        )),
        Some(PromotionPhase::Merged(merge)) => Next::Phase(PromotionPhase::RcPlanned(
            promotion.preview_rc(merge).await?,
        )),
        Some(PromotionPhase::RcPlanned(plan)) | Some(PromotionPhase::FinalizePlanned(plan)) => {
            let finalize = matches!(journal.last(), Some(PromotionPhase::FinalizePlanned(_)));
            let name = if finalize {
                "finalize-build"
            } else {
                "rc-build"
            };
            if let cost::Reservation::OverLimit(detail) = reserve(deps, record, &work(record, name))
                .await
                .map_err(|error| PromotionError::inconclusive(error.to_string()))?
            {
                return Err(PromotionError::rejected(format!("budget: {detail}")));
            }
            let confirmed = promotion.confirm_plan(plan).await?;
            Next::Phase(if finalize {
                PromotionPhase::FinalizeConfirmed(confirmed)
            } else {
                PromotionPhase::RcConfirmed(confirmed)
            })
        }
        Some(PromotionPhase::RcConfirmed(confirmed))
        | Some(PromotionPhase::FinalizeConfirmed(confirmed)) => {
            let finalize = matches!(journal.last(), Some(PromotionPhase::FinalizeConfirmed(_)));
            match promotion.read_operation(confirmed).await? {
                OperationState::Converging { status } => {
                    Next::Wait(format!("release operation {status}"))
                }
                OperationState::Converged(deployment) => {
                    let artifact = promotion.read_artifact(&deployment).await?;
                    let name = if finalize {
                        "finalize-build"
                    } else {
                        "rc-build"
                    };
                    settle(deps, &record.cycle_id, &work(record, name).work_id, None)
                        .await
                        .map_err(|error| PromotionError::inconclusive(error.to_string()))?;
                    Next::Phase(if finalize {
                        check_republished(&validated_rc(journal)?, &artifact)?;
                        PromotionPhase::StablePublished(artifact)
                    } else {
                        PromotionPhase::RcPublished(artifact)
                    })
                }
                OperationState::Blocked => {
                    return Err(PromotionError::rejected("the release operation is blocked"))
                }
                OperationState::Discarded => {
                    return Err(PromotionError::rejected(
                        "the release operation was discarded",
                    ))
                }
            }
        }
        Some(PromotionPhase::RcPublished(rc)) => match &record.artifact_decision {
            None => Next::Run("artifact", ExecutionRoleV1::Artifact, rc.version.clone()),
            Some(decision) => {
                let identity = record
                    .analyses
                    .iter()
                    .rev()
                    .find(|analysis| analysis.role == ExecutionRoleV1::Artifact)
                    .map(|analysis| analysis.identity.clone())
                    .unwrap_or_default();
                let approval = ArtifactApproval {
                    outcome: match decision.outcome {
                        DecisionOutcomeV1::Accepted => ApprovalOutcome::Accepted,
                        DecisionOutcomeV1::Rejected => ApprovalOutcome::Rejected,
                        DecisionOutcomeV1::Inconclusive => ApprovalOutcome::Inconclusive,
                    },
                    policy_revision: decision.policy_revision.clone(),
                    version: rc.version.clone(),
                    source_sha: rc.source_sha.clone(),
                    archive_sha256: rc.archive_sha256.clone(),
                    binary_sha256: identity.harness_binary_sha256.as_deref().map(bare),
                };
                // Checked now so a refusal stops before anything is journaled
                // as approved.
                check_artifact_approval(&approval, rc)?;
                Next::Phase(PromotionPhase::ArtifactApproved(approval))
            }
        },
        Some(PromotionPhase::ArtifactApproved(_)) => {
            let rc =
                rc_of(journal).ok_or_else(|| PromotionError::inconclusive("no rc artifact"))?;
            let rc_plan =
                rc_plan_of(journal).ok_or_else(|| PromotionError::inconclusive("no rc plan"))?;
            Next::Phase(PromotionPhase::FinalizePlanned(
                promotion
                    .preview_finalize(&rc, &validated_rc(journal)?, &rc_plan)
                    .await?,
            ))
        }
        Some(PromotionPhase::StablePublished(stable)) => {
            let observed = promotion.observe_target().await?;
            let (Some(version), Some(binary)) =
                (observed.version.clone(), observed.binary_sha256.clone())
            else {
                return Err(PromotionError::inconclusive(format!(
                    "the target's running version or executable is unknown: {}",
                    observed
                        .binary_unknown_reason
                        .unwrap_or_else(|| "no pinned version".into())
                )));
            };
            if version == stable.version {
                return Err(PromotionError::rejected(
                    "the target already runs the new version: no validated prior to roll back to",
                ));
            }
            let prior = promotion
                .read_artifact(&DeploymentEvidence {
                    operation_id: String::new(),
                    deployment_id: String::new(),
                    version: version.clone(),
                    channel: String::new(),
                    from_version: None,
                    source_sha: String::new(),
                    descriptor_sha256: None,
                    build_run_id: None,
                })
                .await?;
            if binary != prior.binary_sha256 {
                return Err(PromotionError::rejected(
                    "the target's prior executable does not match its Registry artifact; rollback is not proved",
                ));
            }
            Next::Phase(PromotionPhase::RollbackDestination(ValidatedArtifact {
                version,
                archive_sha256: prior.archive_sha256,
                binary_sha256: prior.binary_sha256,
            }))
        }
        Some(PromotionPhase::RollbackDestination(prior)) => {
            let stable = validated_stable(journal)?;
            // The canary is reserved before anything is activated: an
            // activation that cannot be verified is never started.
            let plan = record
                .plan
                .as_ref()
                .ok_or_else(|| PromotionError::inconclusive("no plan"))?;
            let canary = CycleWorkV1 {
                work_id: format!("{}:{CANARY}", record.cycle_id),
                kind: CycleWorkKindV1::E2e,
                estimated_usd: plan.estimated_usd,
                candidate: false,
            };
            if let cost::Reservation::OverLimit(detail) = reserve(deps, record, &canary)
                .await
                .map_err(|error| PromotionError::inconclusive(error.to_string()))?
            {
                return Err(PromotionError::rejected(format!(
                    "budget cannot cover the canary ({detail}); nothing is activated"
                )));
            }
            let operation = compose_operation_id(id, "activate", &stable.version, 1);
            // The intent is durable before the pin is sent.
            if record.activation_intent.as_deref() != Some(operation.as_str()) {
                let persisted = state::update_cycle(&deps.iii, &record.cycle_id, |stored| {
                    if stored.step != record.step
                        || stored.status.is_terminal()
                        || stored.recovery.is_some()
                    {
                        return Ok((None, false));
                    }
                    let mut next = stored.clone();
                    next.activation_intent = Some(operation.clone());
                    Ok((Some(next), true))
                })
                .await
                .map_err(|error| PromotionError::inconclusive(error.to_string()))?
                .unwrap_or(false);
                if !persisted {
                    return Ok(Next::Wait(
                        "the cycle moved on before the activation".into(),
                    ));
                }
            }
            Next::Phase(PromotionPhase::ActivationRequested(
                promotion
                    .request_activation(prior, &stable, &operation)
                    .await?,
            ))
        }
        Some(PromotionPhase::ActivationRequested(request)) => {
            match promotion
                .confirm_running(&validated_stable(journal)?, &request.operation_id)
                .await?
            {
                Confirmation::Confirmed(running) => {
                    Next::Phase(PromotionPhase::ActivationConfirmed(running))
                }
                Confirmation::Pending { reason } => {
                    Next::Wait(format!("activation pending: {reason}"))
                }
            }
        }
        Some(PromotionPhase::ActivationConfirmed(_)) => match &record.canary {
            None => canary_on_target(deps, record, client, &promotion.config().target).await?,
            Some(canary) if canary.outcome == CanaryOutcomeV1::Violated => {
                let prior = prior_of(journal)
                    .ok_or_else(|| PromotionError::inconclusive("no rollback destination"))?;
                let stable = validated_stable(journal)?;
                let operation = compose_operation_id(id, "rollback", &prior.version, 1);
                Next::Phase(PromotionPhase::RollbackRequested(
                    promotion
                        .request_rollback(&prior, &stable, &operation)
                        .await?,
                ))
            }
            Some(canary) if canary.outcome == CanaryOutcomeV1::Held => {
                Next::Finish(OutcomeCodeV1::Promoted, canary.reason.clone())
            }
            // Unknown is never held: the observation stays pending until a
            // limit restores the prior; no new canary is started.
            Some(canary) => Next::Wait(format!(
                "activated; the canary is unknown, observation pending (never claimed stable): {}",
                canary.reason
            )),
        },
        Some(PromotionPhase::RollbackRequested(request)) => {
            let prior = prior_of(journal)
                .ok_or_else(|| PromotionError::inconclusive("no rollback destination"))?;
            match promotion
                .confirm_running(&prior, &request.operation_id)
                .await?
            {
                Confirmation::Confirmed(running) => {
                    Next::Phase(PromotionPhase::RollbackConfirmed(running))
                }
                Confirmation::Pending { reason } => {
                    Next::Wait(format!("rollback pending: {reason}"))
                }
            }
        }
        Some(PromotionPhase::RollbackConfirmed(running)) => Next::Finish(
            OutcomeCodeV1::RolledBack,
            format!(
                "canary violated ({}); prior {} proved running again (executable {})",
                record
                    .canary
                    .as_ref()
                    .map(|canary| canary.reason.as_str())
                    .unwrap_or_default(),
                running.version,
                running.binary_sha256
            ),
        ),
        Some(PromotionPhase::Stopped(stopped)) => {
            return Err(PromotionError {
                kind: stopped.kind,
                reason: stopped.reason.clone(),
            })
        }
    })
}

/// Effects that only read or re-ask an idempotent operation: an
/// inconclusive answer there is retried on the sweep.
fn retried(phase: Option<&PromotionPhase>) -> bool {
    matches!(
        phase,
        Some(
            PromotionPhase::PullRequest(_)
                | PromotionPhase::MergeRequested(_)
                | PromotionPhase::RcPlanned(_)
                | PromotionPhase::RcConfirmed(_)
                | PromotionPhase::FinalizePlanned(_)
                | PromotionPhase::FinalizeConfirmed(_)
                | PromotionPhase::RollbackDestination(_)
                | PromotionPhase::ActivationRequested(_)
                | PromotionPhase::ActivationConfirmed(_)
                | PromotionPhase::RollbackRequested(_)
        )
    )
}

/// The promotion step: one journaled effect, then the next step at once, a
/// poll on the sweep, an E2E dispatch, or the end. A stop is journaled with
/// its kind and ends the cycle without claiming what was not proved.
pub async fn promotion_stage(
    deps: &Deps,
    record: &CycleRecordV1,
) -> Result<Option<CycleRecordV1>, EvalError> {
    let journal = journal_of(record);
    let Some(config) = record
        .config
        .as_ref()
        .and_then(|stored| stored.config.promotion.clone())
    else {
        return end(
            deps,
            record,
            OutcomeCodeV1::Decided,
            "nothing released: promotion is not configured",
        )
        .await;
    };
    // ponytail: the release ships one worker, the Harness. A change to any
    // other unit stays approved on its branch; releasing every changed worker
    // (and proving each running digest) lifts this.
    let candidate = record.candidate.as_ref();
    let units = candidate
        .and_then(|state| state.candidate.as_ref()?.commit.as_ref())
        .map(|commit| commit.units.clone())
        .unwrap_or_default();
    if journal.last().is_none() && !units.is_empty() && units != [crate::promotion::WORKER] {
        let branch = candidate.map_or("", |state| state.identity.branch.as_str());
        return end(
            deps,
            record,
            OutcomeCodeV1::Decided,
            &format!(
                "approved, not released: the release ships only the {}, and this change is to {}; \
                 it is delivered from {branch} by hand",
                crate::promotion::WORKER,
                units.join(", ")
            ),
        )
        .await;
    }
    let origins = match deps.release_origins.clone() {
        Some(origins) => Ok(origins),
        None => crate::promotion::PromotionOrigins::from_deployment_env(),
    };
    let outcome = match (origins, deps.target(&config.target.engine_url).await) {
        (Err(error), _) => Err(error),
        // An Engine restarting is retried where the phase is retried.
        (_, Err(error)) => Err(PromotionError::inconclusive(error.to_string())),
        (Ok(origins), Ok(target)) => {
            match Promotion::new(config.clone(), origins, target.clone()) {
                Ok(promotion) => Box::pin(next(deps, record, &promotion, &target, &journal)).await,
                Err(error) => Err(error),
            }
        }
    };
    let step = step_name(journal.last());
    match outcome {
        Ok(Next::Phase(phase)) => {
            let mut journal = journal.clone();
            journal.push(phase);
            let detail = format!("promotion: {step} recorded");
            transition(
                deps,
                &record.cycle_id,
                record.step,
                CycleStatusV1::Running,
                CycleStageV1::Promotion,
                &detail,
                |next| {
                    next.promotion = Some(journal.clone());
                    Ok(())
                },
            )
            .await
        }
        Ok(Next::Wait(reason)) => {
            transition(
                deps,
                &record.cycle_id,
                record.step,
                CycleStatusV1::Waiting,
                CycleStageV1::Promotion,
                &format!("promotion: {reason}"),
                |next| {
                    next.promotion = Some(journal.clone());
                    Ok(())
                },
            )
            .await
        }
        Ok(Next::Run(name, role, version)) => {
            let (Some(plan), Some(validation)) = (
                record.plan.clone(),
                record
                    .config
                    .as_ref()
                    .and_then(|stored| stored.config.validation.clone()),
            ) else {
                return end(
                    deps,
                    record,
                    OutcomeCodeV1::Inconclusive,
                    "no plan or validation for the artifact run",
                )
                .await;
            };
            let parameters = ExecutionParametersV1 {
                where_: validation.where_,
                stack: Some(validation.artifact_stack(&version)),
                ..plan.parameters.clone()
            };
            let mut staged = record.clone();
            staged.promotion = Some(journal.clone());
            dispatch(deps, &staged, role, name, &parameters, plan.estimated_usd).await
        }
        Ok(Next::Imported(label, claim)) => {
            let detail = format!(
                "canary {} completed on the target and was imported",
                claim.execution_id
            );
            transition(
                deps,
                &record.cycle_id,
                record.step,
                CycleStatusV1::Running,
                CycleStageV1::Analysis,
                &detail,
                |next| {
                    next.promotion = Some(journal.clone());
                    if let Some(entry) = next
                        .executions
                        .iter_mut()
                        .find(|entry| entry.label.as_deref() == Some(label.as_str()))
                    {
                        entry.execution_id = Some(claim.execution_id.clone());
                        entry.state = ExecutionStateV1::Imported;
                        entry.admission_key = Some(claim.admission_key.clone());
                    }
                    Ok(())
                },
            )
            .await
        }
        Ok(Next::Finish(code, detail)) => end(deps, record, code, &detail).await,
        // A recovery is never abandoned on an unreachable or unproved target:
        // it is retried until the prior is proved (or a refusal stops it).
        Err(error)
            if (error.kind == StopKind::Inconclusive && retried(journal.last()))
                || (record.recovery.is_some() && error.kind != StopKind::Rejected) =>
        {
            transition(
                deps,
                &record.cycle_id,
                record.step,
                CycleStatusV1::Waiting,
                CycleStageV1::Promotion,
                &format!("promotion: {step} not proved yet: {}", error.reason),
                |_| Ok(()),
            )
            .await
        }
        Err(error) => {
            let activated = record.activation_intent.is_some()
                || journal
                    .phases
                    .iter()
                    .any(|phase| matches!(phase, PromotionPhase::ActivationRequested(_)));
            let detail = format!(
                "promotion stopped at {step} ({:?}): {}{}",
                error.kind,
                error.reason,
                if activated {
                    "; the target's running artifact is not proved"
                } else {
                    "; nothing was activated"
                }
            );
            let mut stopped = journal.clone();
            if !matches!(stopped.last(), Some(PromotionPhase::Stopped(_))) {
                stopped.push(PromotionPhase::Stopped(Stopped::at(step, &error)));
            }
            transition_with(deps, record, |next| {
                if next.recovery.is_none() && activation_in_flight(next) {
                    let reason = format!("promotion failed at {step}: {}", error.reason);
                    next.recovery = Some(reason.clone());
                    return Ok((
                        CycleStatusV1::Running,
                        CycleStageV1::Promotion,
                        format!("{reason}; restoring the prior executable"),
                    ));
                }
                next.promotion = Some(stopped.clone());
                next.outcome = Some(CycleOutcomeV1 {
                    code: OutcomeCodeV1::Inconclusive,
                    detail: detail.clone(),
                });
                Ok((CycleStatusV1::Completed, CycleStageV1::Done, detail.clone()))
            })
            .await
        }
    }
}
