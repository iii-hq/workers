use std::sync::Arc;

use harness::functions::metrics::{SessionMetricsRequestV1, SessionMetricsResponseV1};
use harness::functions::send::{MessageInput, SendOptions, SendRequest, SendResponse, SessionInit};
use harness::functions::session_tree::{SessionTreeRequestV1, SessionTreeResponseV1};
use harness::functions::status::{StatusReport, StatusRequest};
use harness::functions::stop::{StopRequest, StopResponse};
use harness::functions::teardown::{TeardownRequestV1, TeardownResponseV1};
use harness::types::turn::TurnStatus;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

use crate::contract::{
    EvalCancelResponseV1, EvalDeleteResponseV1, EvalListRequestV1, EvalListResponseV1,
    EvalRerunRequestV1, EvalResultResponseV1, EvalRunStatusV1, EvalStartRequestV1,
    EvalStartResponseV1, EvalStatusV1, EvaluationIdRequestV1, EvaluatorInputV1,
    EvaluatorResponseV1, NormalizedEvalRequestV1, StepRequestV1, StepResponseV1, SweepResponseV1,
    VariantRoleV1, WakeEventV1, WakeResponseV1,
};
use crate::error::{EvalError, EvalFailureV1, EvalPhaseV1};
use crate::events::EvalEvents;
use crate::locks::EvalLocks;
use crate::report::{build_progress, build_report, EvalBenchmarkV1};
use crate::state::{EvalJobRecordV1, SessionIndexV1};
use crate::{ids, queue, state};

#[derive(Clone)]
pub struct Deps {
    pub iii: Arc<IIIClient>,
    pub locks: EvalLocks,
    pub events: EvalEvents,
}

pub async fn start(
    deps: &Deps,
    request: EvalStartRequestV1,
) -> Result<EvalStartResponseV1, EvalError> {
    let request = request.normalize()?;
    create_job(deps, request).await
}

async fn create_job(
    deps: &Deps,
    request: NormalizedEvalRequestV1,
) -> Result<EvalStartResponseV1, EvalError> {
    let evaluation_id = ids::evaluation_id();
    let now = ids::now_ms();
    let job = EvalJobRecordV1 {
        schema_version: "1".into(),
        evaluation_id: evaluation_id.clone(),
        runs: state::build_run_plan(&evaluation_id, &request),
        request,
        status: EvalStatusV1::Queued,
        step: 0,
        next_index: 0,
        active_index: None,
        active_waited_for_descendants: false,
        active_finalization_sent: false,
        report: None,
        error: None,
        created_at: now,
        updated_at: now,
        completed_at: None,
    };
    state::put_job(&deps.iii, &job).await?;
    queue::enqueue_step(&deps.iii, &evaluation_id, 0).await?;
    Ok(EvalStartResponseV1 {
        evaluation_id,
        status: EvalStatusV1::Queued,
    })
}

pub async fn rerun(
    deps: &Deps,
    request: EvalRerunRequestV1,
) -> Result<EvalStartResponseV1, EvalError> {
    let source = state::get_job(&deps.iii, &request.evaluation_id)
        .await?
        .ok_or_else(|| EvalError::NotFound(request.evaluation_id.clone()))?;
    if !source.status.is_terminal() {
        return Err(EvalError::Conflict(format!(
            "{} is still {:?}; rerun it after completion or cancellation",
            source.evaluation_id, source.status
        )));
    }
    let mut persisted = source.request.clone();
    persisted.source_evaluation_id = Some(source.evaluation_id);
    if request.reverse_order {
        persisted.execution_order = persisted.execution_order.reversed();
    }
    create_job(deps, persisted).await
}

pub async fn status(
    deps: &Deps,
    request: EvaluationIdRequestV1,
) -> Result<Option<crate::contract::EvalStatusResponseV1>, EvalError> {
    Ok(state::get_job(&deps.iii, &request.evaluation_id)
        .await?
        .map(|job| job.status_response()))
}

pub async fn list(
    deps: &Deps,
    request: EvalListRequestV1,
) -> Result<EvalListResponseV1, EvalError> {
    let limit = request.normalized_limit()?;
    let mut jobs = state::list_jobs(&deps.iii).await?;
    jobs.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.evaluation_id.cmp(&left.evaluation_id))
    });
    Ok(EvalListResponseV1 {
        evaluations: jobs
            .into_iter()
            .take(limit)
            .map(|job| job.summary_response())
            .collect(),
    })
}

pub async fn result(
    deps: &Deps,
    request: EvaluationIdRequestV1,
) -> Result<Option<EvalResultResponseV1>, EvalError> {
    Ok(state::get_job(&deps.iii, &request.evaluation_id)
        .await?
        .map(|job| EvalResultResponseV1 {
            status: job.status,
            progress: build_progress(&job.runs),
            request: job.request,
            report: job.report,
        }))
}

pub async fn cancel(
    deps: &Deps,
    request: EvaluationIdRequestV1,
) -> Result<EvalCancelResponseV1, EvalError> {
    let _guard = deps.locks.guard(&request.evaluation_id).await;
    let mut job = state::get_job(&deps.iii, &request.evaluation_id)
        .await?
        .ok_or_else(|| EvalError::NotFound(request.evaluation_id.clone()))?;
    if job.status.is_terminal() {
        return Ok(EvalCancelResponseV1 {
            cancelled: false,
            status: job.status,
        });
    }

    if let Some(index) = job.active_index {
        if let Some(run) = job.runs.get(index) {
            stop_harness_tree(deps, &run.session_id, &job).await;
            let _ = harness_teardown(deps, &run.session_id, &job).await;
            let _ = state::delete_session_index(&deps.iii, &run.session_id).await;
        }
    }

    let now = ids::now_ms();
    for run in &mut job.runs {
        if !run.status.is_terminal() {
            run.status = EvalRunStatusV1::Cancelled;
            run.completed_at = Some(now);
            run.failures.push(EvalFailureV1::new(
                EvalPhaseV1::Cancel,
                "evaluation cancelled",
            ));
        }
    }
    job.active_index = None;
    job.status = EvalStatusV1::Cancelled;
    job.updated_at = now;
    job.completed_at = Some(now);
    job.report = Some(build_report(
        &job.evaluation_id,
        &job.request,
        job.runs.clone(),
        job.created_at,
        now,
    ));
    state::put_job(&deps.iii, &job).await?;
    deps.events
        .emit_completed(&job.evaluation_id, job.status, Some(false))
        .await;
    Ok(EvalCancelResponseV1 {
        cancelled: true,
        status: job.status,
    })
}

pub async fn delete(
    deps: &Deps,
    request: EvaluationIdRequestV1,
) -> Result<EvalDeleteResponseV1, EvalError> {
    let _guard = deps.locks.guard(&request.evaluation_id).await;
    let Some(job) = state::get_job(&deps.iii, &request.evaluation_id).await? else {
        return Ok(EvalDeleteResponseV1 { deleted: false });
    };
    if !job.status.is_terminal() {
        return Err(EvalError::Conflict(format!(
            "{} is still {:?}; cancel or wait before deleting it",
            job.evaluation_id, job.status
        )));
    }
    for run in &job.runs {
        state::delete_session_index(&deps.iii, &run.session_id).await?;
    }
    state::delete_job(&deps.iii, &job.evaluation_id).await?;
    Ok(EvalDeleteResponseV1 { deleted: true })
}

pub fn exact(input: EvaluatorInputV1) -> Result<EvaluatorResponseV1, EvalError> {
    let expected = input.arguments.get("expected").ok_or_else(|| {
        EvalError::InvalidRequest(
            "eval::assert::exact requires evaluator arguments.expected".into(),
        )
    })?;
    let passed = &input.output == expected;
    Ok(EvaluatorResponseV1 {
        passed,
        score: Some(if passed { 1.0 } else { 0.0 }),
        reason: Some(if passed {
            "output exactly matched arguments.expected".into()
        } else {
            format!(
                "output did not exactly match arguments.expected (actual={}, expected={})",
                compact(&input.output),
                compact(expected)
            )
        }),
        details: None,
    })
}

pub fn normalized_text(input: EvaluatorInputV1) -> Result<EvaluatorResponseV1, EvalError> {
    let expected = input
        .arguments
        .get("expected")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            EvalError::InvalidRequest(
                "eval::assert::normalized_text requires string evaluator arguments.expected".into(),
            )
        })?;
    let Some(actual) = input.output.as_str() else {
        return Ok(EvaluatorResponseV1 {
            passed: false,
            score: Some(0.0),
            reason: Some(format!(
                "output was not text (actual={}, expected={})",
                compact(&input.output),
                compact(&Value::String(expected.into()))
            )),
            details: None,
        });
    };
    let actual_normalized = normalize_text(actual);
    let expected_normalized = normalize_text(expected);
    let passed = actual_normalized == expected_normalized;
    Ok(EvaluatorResponseV1 {
        passed,
        score: Some(if passed { 1.0 } else { 0.0 }),
        reason: Some(if passed {
            "normalized output matched arguments.expected".into()
        } else {
            format!(
                "normalized output did not match arguments.expected (actual={}, expected={})",
                compact(&Value::String(actual_normalized)),
                compact(&Value::String(expected_normalized))
            )
        }),
        details: None,
    })
}

fn normalize_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|character: char| character.is_ascii_punctuation())
        .to_lowercase()
}

pub async fn wake(deps: &Deps, event: WakeEventV1) -> Result<WakeResponseV1, EvalError> {
    if !event.terminal || !event.session_id.starts_with("eval_") {
        return Ok(WakeResponseV1 { woke: false });
    }
    let Some(index) = state::get_session_index(&deps.iii, &event.session_id).await? else {
        return Ok(WakeResponseV1 { woke: false });
    };
    let Some(job) = state::get_job(&deps.iii, &index.evaluation_id).await? else {
        return Ok(WakeResponseV1 { woke: false });
    };
    if job.status.is_terminal() {
        return Ok(WakeResponseV1 { woke: false });
    }
    queue::enqueue_step(&deps.iii, &job.evaluation_id, job.step).await?;
    Ok(WakeResponseV1 { woke: true })
}

pub async fn sweep(deps: &Deps) -> Result<SweepResponseV1, EvalError> {
    let jobs = state::list_jobs(&deps.iii).await?;
    let mut swept = 0;
    for job in jobs.iter().filter(|job| !job.status.is_terminal()) {
        match queue::enqueue_step(&deps.iii, &job.evaluation_id, job.step).await {
            Ok(()) => swept += 1,
            Err(error) => {
                tracing::warn!(
                    evaluation_id = %job.evaluation_id,
                    %error,
                    "eval recovery sweep could not enqueue step"
                );
            }
        }
    }
    Ok(SweepResponseV1 { swept })
}

pub async fn step(deps: &Deps, request: StepRequestV1) -> Result<StepResponseV1, EvalError> {
    let _guard = deps.locks.guard(&request.evaluation_id).await;
    let Some(mut job) = state::get_job(&deps.iii, &request.evaluation_id).await? else {
        return Ok(StepResponseV1 {
            skipped: true,
            status: EvalStatusV1::Failed,
        });
    };
    if job.status.is_terminal() || request.step != job.step {
        return Ok(StepResponseV1 {
            skipped: true,
            status: job.status,
        });
    }

    if job.status == EvalStatusV1::Queued {
        job.status = EvalStatusV1::Running;
        job.updated_at = ids::now_ms();
        state::put_job(&deps.iii, &job).await?;
    }

    if let Some(index) = job.active_index {
        if run_timed_out(&job, index) {
            time_out_active(deps, &mut job, index).await?;
            return advance(deps, job).await;
        }
        if job.runs[index].turn_id.is_none() {
            launch_active(deps, &mut job, index).await?;
            if job.runs[index].status.is_terminal() {
                return advance(deps, job).await;
            }
            return Ok(StepResponseV1 {
                skipped: false,
                status: job.status,
            });
        }
        match reconcile_active(deps, &mut job, index).await? {
            ReconcileOutcome::StillRunning => {
                return Ok(StepResponseV1 {
                    skipped: false,
                    status: job.status,
                })
            }
            ReconcileOutcome::Terminal => return advance(deps, job).await,
        }
    }

    if job.next_index < job.runs.len() {
        let index = job.next_index;
        job.next_index += 1;
        job.active_index = Some(index);
        job.runs[index].status = EvalRunStatusV1::Running;
        job.runs[index].started_at = ids::now_ms();
        job.updated_at = ids::now_ms();
        let session_index = SessionIndexV1 {
            evaluation_id: job.evaluation_id.clone(),
            run_id: job.runs[index].run_id.clone(),
        };
        state::put_session_index(&deps.iii, &job.runs[index].session_id, &session_index).await?;
        state::put_job(&deps.iii, &job).await?;
        launch_active(deps, &mut job, index).await?;
        if job.runs[index].status.is_terminal() {
            return advance(deps, job).await;
        }
        return Ok(StepResponseV1 {
            skipped: false,
            status: job.status,
        });
    }

    finalize(deps, &mut job).await?;
    Ok(StepResponseV1 {
        skipped: false,
        status: job.status,
    })
}

async fn launch_active(
    deps: &Deps,
    job: &mut EvalJobRecordV1,
    index: usize,
) -> Result<(), EvalError> {
    let run = job.runs[index].clone();
    let variant = match run.role {
        VariantRoleV1::Control => &job.request.control,
        VariantRoleV1::Treatment => &job.request.treatment,
    };
    let request = SendRequest {
        session_id: Some(run.session_id.clone()),
        message: MessageInput::Text(variant.prompt.clone()),
        model: Some(job.request.model.model.clone()),
        provider: job.request.model.provider.clone(),
        idempotency_key: Some(ids::send_idempotency_key(
            &job.evaluation_id,
            run.role,
            run.iteration,
        )),
        session: Some(SessionInit {
            title: Some(format!(
                "Evaluation {}: {} {}",
                job.evaluation_id,
                run.role.as_str(),
                run.iteration
            )),
            metadata: Some(json!({
                "evaluation_id": job.evaluation_id,
                "eval_run_id": run.run_id,
                "eval_role": run.role,
                "eval_iteration": run.iteration,
            })),
        }),
        options: Some(send_options(job, variant)),
    };

    let response: Result<SendResponse, EvalError> = trigger(
        deps,
        "harness::send",
        request,
        job.request.limits.execution.invocation_timeout_seconds,
    )
    .await;
    match response {
        Ok(response) if response.accepted && response.session_id == job.runs[index].session_id => {
            job.runs[index].turn_id = Some(response.turn_id);
            job.updated_at = ids::now_ms();
            state::put_job(&deps.iii, job).await
        }
        Ok(response) => {
            fail_run(
                &mut job.runs[index],
                EvalRunStatusV1::Failed,
                EvalFailureV1::function(
                    EvalPhaseV1::Send,
                    "harness::send",
                    format!(
                        "unexpected response: accepted={}, session_id={}",
                        response.accepted, response.session_id
                    ),
                ),
            );
            state::put_job(&deps.iii, job).await
        }
        Err(error) => {
            // If the response was lost after acceptance, the deterministic
            // session may already exist. Preserve it for reconciliation.
            let status = harness_status(deps, &job.runs[index].session_id, job).await?;
            if let Some(status) = status {
                job.runs[index].turn_id = status.turn_id;
                state::put_job(&deps.iii, job).await
            } else {
                fail_run(
                    &mut job.runs[index],
                    EvalRunStatusV1::Failed,
                    EvalFailureV1::function(EvalPhaseV1::Send, "harness::send", error.to_string()),
                );
                state::put_job(&deps.iii, job).await
            }
        }
    }
}

enum ReconcileOutcome {
    StillRunning,
    Terminal,
}

async fn reconcile_active(
    deps: &Deps,
    job: &mut EvalJobRecordV1,
    index: usize,
) -> Result<ReconcileOutcome, EvalError> {
    let session_id = job.runs[index].session_id.clone();
    let Some(status) = harness_status(deps, &session_id, job).await? else {
        return Ok(ReconcileOutcome::StillRunning);
    };
    if !status.status.is_terminal() {
        return Ok(ReconcileOutcome::StillRunning);
    }

    let now = ids::now_ms();
    let wall_time_ms = elapsed_ms(job.runs[index].started_at, now);
    let metrics = harness_metrics(deps, &session_id, job).await?;
    if !metrics.complete {
        if !job.active_waited_for_descendants {
            job.active_waited_for_descendants = true;
            job.updated_at = now;
            state::put_job(&deps.iii, job).await?;
        }
        return Ok(ReconcileOutcome::StillRunning);
    }

    if needs_finalization(
        status.status,
        status.result_error.is_none(),
        status.expects_wake,
        job.active_waited_for_descendants,
        job.active_finalization_sent,
    ) {
        send_finalization(deps, job, index).await?;
        return Ok(ReconcileOutcome::StillRunning);
    }

    let output = status.result.clone().unwrap_or(Value::Null);
    job.runs[index].output = Some(output.clone());
    job.runs[index].completed_at = Some(now);
    match status.status {
        TurnStatus::Completed if status.result_error.is_none() => {
            let limit_failures = job.request.limits.failures(&metrics);
            job.runs[index].benchmark = EvalBenchmarkV1::from_metrics(&metrics, wall_time_ms);
            job.runs[index].metrics = Some(metrics.clone());
            if output.is_null() || job.request.evaluator.is_none() {
                job.runs[index].passed = None;
                job.runs[index].failures.extend(limit_failures);
                job.runs[index].status = EvalRunStatusV1::Completed;
            } else {
                let evaluator = job
                    .request
                    .evaluator
                    .clone()
                    .expect("evaluator checked above");
                let evaluator_input = EvaluatorInputV1 {
                    evaluation_id: job.evaluation_id.clone(),
                    run_id: job.runs[index].run_id.clone(),
                    role: job.runs[index].role,
                    session_id: session_id.clone(),
                    output,
                    metrics: metrics.clone(),
                    arguments: evaluator.arguments,
                };
                let evaluation: Result<EvaluatorResponseV1, EvalError> = trigger(
                    deps,
                    &evaluator.function_id,
                    evaluator_input,
                    job.request.limits.execution.invocation_timeout_seconds,
                )
                .await;
                match evaluation.and_then(EvaluatorResponseV1::validate) {
                    Ok(evaluation) => {
                        job.runs[index].passed =
                            Some(evaluation.passed && limit_failures.is_empty());
                        job.runs[index].evaluation = Some(evaluation);
                        job.runs[index].failures.extend(limit_failures);
                        job.runs[index].status = EvalRunStatusV1::Completed;
                    }
                    Err(error) => {
                        fail_run(
                            &mut job.runs[index],
                            EvalRunStatusV1::Failed,
                            EvalFailureV1::function(
                                EvalPhaseV1::Evaluate,
                                evaluator.function_id,
                                error.to_string(),
                            ),
                        );
                    }
                }
            }
        }
        TurnStatus::Cancelled => {
            store_metrics(job, index, metrics, wall_time_ms);
            fail_run(
                &mut job.runs[index],
                EvalRunStatusV1::Cancelled,
                EvalFailureV1::function(
                    EvalPhaseV1::Await,
                    "harness::status",
                    status
                        .result_error
                        .or_else(|| Some("harness run was cancelled".into()))
                        .unwrap_or_default(),
                ),
            );
        }
        _ => {
            store_metrics(job, index, metrics, wall_time_ms);
            fail_run(
                &mut job.runs[index],
                EvalRunStatusV1::Failed,
                EvalFailureV1::function(
                    EvalPhaseV1::Await,
                    "harness::status",
                    status
                        .result_error
                        .unwrap_or_else(|| format!("harness run ended as {:?}", status.status)),
                ),
            );
        }
    }
    job.updated_at = now;
    state::put_job(&deps.iii, job).await?;
    Ok(ReconcileOutcome::Terminal)
}

fn needs_finalization(
    status: TurnStatus,
    result_ok: bool,
    expects_wake: bool,
    waited_for_descendants: bool,
    finalization_sent: bool,
) -> bool {
    status == TurnStatus::Completed
        && result_ok
        && (expects_wake || waited_for_descendants)
        && !finalization_sent
}

async fn send_finalization(
    deps: &Deps,
    job: &mut EvalJobRecordV1,
    index: usize,
) -> Result<(), EvalError> {
    let run = job.runs[index].clone();
    let variant = match run.role {
        VariantRoleV1::Control => &job.request.control,
        VariantRoleV1::Treatment => &job.request.treatment,
    };
    let response: SendResponse = trigger(
        deps,
        "harness::send",
        SendRequest {
            session_id: Some(run.session_id.clone()),
            message: MessageInput::Text(
                "[eval-finalize] All descendant sessions are now terminal. Inspect the durable \
                 outcomes, re-check every completion condition, finish any required final report, \
                 remove every trigger or subscription created by this run, and return the \
                 definitive result. Do not return another progress update or start unrelated work."
                    .into(),
            ),
            model: Some(job.request.model.model.clone()),
            provider: job.request.model.provider.clone(),
            idempotency_key: Some(ids::finalization_idempotency_key(
                &job.evaluation_id,
                run.role,
                run.iteration,
            )),
            session: None,
            options: Some(send_options(job, variant)),
        },
        job.request.limits.execution.invocation_timeout_seconds,
    )
    .await?;
    if !response.accepted || response.session_id != run.session_id {
        return Err(EvalError::Dependency(format!(
            "harness::send finalization returned accepted={}, session_id={}",
            response.accepted, response.session_id
        )));
    }
    job.runs[index].turn_id = Some(response.turn_id);
    job.active_finalization_sent = true;
    job.updated_at = ids::now_ms();
    state::put_job(&deps.iii, job).await
}

fn send_options(job: &EvalJobRecordV1, variant: &crate::contract::EvalVariantV1) -> SendOptions {
    SendOptions {
        system_prompt: variant.system_prompt.clone(),
        system_prompt_strategy: if variant.system_prompt.is_none() {
            Some(harness::prompt::SystemPromptStrategy::Disabled)
        } else {
            Some(job.request.model.system_prompt_strategy)
        },
        mode: job.request.model.mode,
        max_turns: Some(job.request.limits.execution.max_turns),
        max_output_tokens: Some(job.request.limits.execution.max_output_tokens_per_call),
        max_total_tokens: job.request.limits.execution.max_total_tokens,
        max_cost_usd: job.request.limits.execution.max_cost_usd,
        thinking_level: job.request.model.thinking_level,
        provider_options: job.request.model.provider_options.clone(),
        output: Some(job.request.output.clone()),
        functions: Some(job.request.functions.clone()),
        metadata: job.request.metadata.clone(),
        max_validation_retries: None,
    }
}

fn store_metrics(
    job: &mut EvalJobRecordV1,
    index: usize,
    metrics: SessionMetricsResponseV1,
    wall_time_ms: u64,
) {
    job.runs[index].benchmark = EvalBenchmarkV1::from_metrics(&metrics, wall_time_ms);
    job.runs[index].metrics = Some(metrics);
}

async fn time_out_active(
    deps: &Deps,
    job: &mut EvalJobRecordV1,
    index: usize,
) -> Result<(), EvalError> {
    let run = job.runs[index].clone();
    stop_harness_tree(deps, &run.session_id, job).await;
    fail_run(
        &mut job.runs[index],
        EvalRunStatusV1::Failed,
        EvalFailureV1::new(
            EvalPhaseV1::Await,
            format!(
                "run exceeded {}s wall-clock limit",
                job.request.limits.execution.scenario_timeout_seconds
            ),
        ),
    );
    state::put_job(&deps.iii, job).await
}

async fn advance(deps: &Deps, mut job: EvalJobRecordV1) -> Result<StepResponseV1, EvalError> {
    if let Some(index) = job.active_index.take() {
        let session_id = job.runs[index].session_id.clone();
        if let Err(error) = harness_teardown(deps, &session_id, &job).await {
            job.runs[index].failures.push(EvalFailureV1::function(
                EvalPhaseV1::Collect,
                "harness::teardown",
                error.to_string(),
            ));
        }
        state::delete_session_index(&deps.iii, &job.runs[index].session_id).await?;
    }
    job.active_waited_for_descendants = false;
    job.active_finalization_sent = false;
    job.step = job.step.saturating_add(1);
    job.updated_at = ids::now_ms();
    if job.next_index >= job.runs.len() {
        finalize(deps, &mut job).await?;
    } else {
        state::put_job(&deps.iii, &job).await?;
        queue::enqueue_step(&deps.iii, &job.evaluation_id, job.step).await?;
    }
    Ok(StepResponseV1 {
        skipped: false,
        status: job.status,
    })
}

async fn finalize(deps: &Deps, job: &mut EvalJobRecordV1) -> Result<(), EvalError> {
    let now = ids::now_ms();
    job.status = EvalStatusV1::Completed;
    job.updated_at = now;
    job.completed_at = Some(now);
    job.report = Some(build_report(
        &job.evaluation_id,
        &job.request,
        job.runs.clone(),
        job.created_at,
        now,
    ));
    state::put_job(&deps.iii, job).await?;
    let eligible = job.report.as_ref().and_then(|report| report.eligible);
    deps.events
        .emit_completed(&job.evaluation_id, job.status, eligible)
        .await;
    Ok(())
}

fn fail_run(
    run: &mut crate::report::EvalRunReportV1,
    status: EvalRunStatusV1,
    failure: EvalFailureV1,
) {
    run.status = status;
    run.passed = None;
    run.completed_at = Some(ids::now_ms());
    run.failures.push(failure);
}

fn run_timed_out(job: &EvalJobRecordV1, index: usize) -> bool {
    let started_at = job.runs[index].started_at;
    started_at > 0
        && ids::now_ms().saturating_sub(started_at)
            > job.request.limits.execution.scenario_timeout_seconds as i64 * 1_000
}

async fn harness_status(
    deps: &Deps,
    session_id: &str,
    job: &EvalJobRecordV1,
) -> Result<Option<StatusReport>, EvalError> {
    trigger(
        deps,
        "harness::status",
        StatusRequest {
            session_id: session_id.into(),
        },
        job.request.limits.execution.invocation_timeout_seconds,
    )
    .await
}

async fn harness_metrics(
    deps: &Deps,
    session_id: &str,
    job: &EvalJobRecordV1,
) -> Result<SessionMetricsResponseV1, EvalError> {
    trigger(
        deps,
        "harness::metrics",
        SessionMetricsRequestV1 {
            root_session_id: session_id.into(),
        },
        job.request.limits.execution.invocation_timeout_seconds,
    )
    .await
}

async fn harness_teardown(
    deps: &Deps,
    session_id: &str,
    job: &EvalJobRecordV1,
) -> Result<TeardownResponseV1, EvalError> {
    trigger(
        deps,
        "harness::teardown",
        TeardownRequestV1 {
            root_session_id: session_id.into(),
        },
        job.request.limits.execution.invocation_timeout_seconds,
    )
    .await
}

async fn stop_harness_tree(deps: &Deps, root_session_id: &str, job: &EvalJobRecordV1) {
    let timeout_seconds = job.request.limits.execution.invocation_timeout_seconds;
    let tree: Result<SessionTreeResponseV1, EvalError> = trigger(
        deps,
        "harness::session-tree",
        SessionTreeRequestV1 {
            root_session_id: root_session_id.into(),
        },
        timeout_seconds,
    )
    .await;
    let mut session_ids = tree
        .map(|tree| {
            tree.sessions
                .into_iter()
                .rev()
                .map(|node| node.session_id)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|_| vec![root_session_id.into()]);
    if !session_ids
        .iter()
        .any(|session_id| session_id == root_session_id)
    {
        session_ids.push(root_session_id.into());
    }
    for session_id in session_ids {
        let _: Result<StopResponse, _> = trigger(
            deps,
            "harness::stop",
            StopRequest {
                session_id,
                turn_id: None,
            },
            timeout_seconds,
        )
        .await;
    }
}

async fn trigger<I, O>(
    deps: &Deps,
    function_id: &str,
    input: I,
    timeout_seconds: u64,
) -> Result<O, EvalError>
where
    I: Serialize,
    O: DeserializeOwned,
{
    let timeout = std::time::Duration::from_secs(timeout_seconds);
    let request = TriggerRequest {
        function_id: function_id.into(),
        payload: serde_json::to_value(input)?,
        action: None,
        timeout_ms: Some(timeout.as_millis().min(u64::MAX as u128) as u64),
    };
    let value = match tokio::time::timeout(timeout, deps.iii.trigger(request)).await {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            return Err(EvalError::Dependency(format!(
                "{function_id} failed: {error}"
            )))
        }
        Err(_) => {
            return Err(EvalError::Dependency(format!(
                "{function_id} exceeded {timeout_seconds}s invocation timeout"
            )))
        }
    };
    serde_json::from_value(value)
        .map_err(|error| EvalError::Serialization(format!("{function_id} response: {error}")))
}

fn elapsed_ms(started_at: i64, completed_at: i64) -> u64 {
    completed_at.saturating_sub(started_at).max(0) as u64
}

fn compact(value: &Value) -> String {
    let rendered = serde_json::to_string(value).unwrap_or_else(|_| "<unserializable>".into());
    const LIMIT: usize = 512;
    if rendered.chars().count() <= LIMIT {
        rendered
    } else {
        format!("{}…", rendered.chars().take(LIMIT).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use harness::functions::metrics::{SessionMetricsResponseV1, SessionUsageTotalsV1};

    use super::*;

    fn evaluator_input(output: Value, expected: Value) -> EvaluatorInputV1 {
        EvaluatorInputV1 {
            evaluation_id: "eval".into(),
            run_id: "run".into(),
            role: VariantRoleV1::Control,
            session_id: "session".into(),
            output,
            metrics: SessionMetricsResponseV1 {
                root_session_id: "session".into(),
                complete: true,
                totals: SessionUsageTotalsV1::default(),
                by_session: Vec::new(),
                traces: None,
            },
            arguments: json!({ "expected": expected }),
        }
    }

    #[test]
    fn exact_evaluator_compares_json_values() {
        assert!(
            exact(evaluator_input(json!({"ok": true}), json!({"ok": true})))
                .unwrap()
                .passed
        );
        assert!(
            !exact(evaluator_input(json!("OK"), json!("NO")))
                .unwrap()
                .passed
        );
    }

    #[test]
    fn exact_evaluator_requires_expected_argument() {
        let mut input = evaluator_input(json!("OK"), json!("OK"));
        input.arguments = json!({});
        assert!(exact(input).is_err());
    }

    #[test]
    fn normalized_text_ignores_case_whitespace_and_surrounding_punctuation() {
        let result = normalized_text(evaluator_input(
            json!("  Olá,   mundo!  "),
            json!("olá, mundo"),
        ))
        .unwrap();
        assert!(result.passed);
    }

    #[test]
    fn normalized_text_rejects_different_text_and_non_text_output() {
        assert!(
            !normalized_text(evaluator_input(json!("hello"), json!("goodbye")))
                .unwrap()
                .passed
        );
        assert!(
            !normalized_text(evaluator_input(json!({"text": "hello"}), json!("hello")))
                .unwrap()
                .passed
        );
    }

    #[test]
    fn normalized_text_requires_string_expected_argument() {
        assert!(
            normalized_text(evaluator_input(json!("hello"), json!({"text": "hello"}))).is_err()
        );
    }

    #[test]
    fn finalization_runs_once_after_an_early_root_result() {
        assert!(needs_finalization(
            TurnStatus::Completed,
            true,
            false,
            true,
            false
        ));
        assert!(!needs_finalization(
            TurnStatus::Completed,
            true,
            false,
            false,
            false
        ));
        assert!(needs_finalization(
            TurnStatus::Completed,
            true,
            true,
            false,
            false
        ));
        assert!(!needs_finalization(
            TurnStatus::Completed,
            true,
            true,
            true,
            true
        ));
        assert!(!needs_finalization(
            TurnStatus::Failed,
            true,
            true,
            true,
            false
        ));
        assert!(!needs_finalization(
            TurnStatus::Completed,
            false,
            true,
            true,
            false
        ));
    }
}
