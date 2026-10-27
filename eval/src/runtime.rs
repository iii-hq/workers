//! The loop cycle: both inputs converge on one record that moves through
//! coverage → baseline → awaiting_execution → analysis → candidate. Every
//! step runs on the `eval-run` queue under the cycle's lock and writes the
//! record with compare-and-set; the sweep recovers lost steps and events.

use std::collections::BTreeMap;
use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

use crate::contract::*;
use crate::error::EvalError;
use crate::evidence::{self, Readiness};
use crate::locks::EvalLocks;
use crate::{cost, coverage, decision, ids, queue, state};

pub(crate) const BUS_TIMEOUT_MS: u64 = 10_000;
/// `execution-start` prepares the run before it answers.
pub(crate) const START_TIMEOUT_MS: u64 = 60_000;
const TESTS_PAGE: u16 = 100;
const EXECUTIONS_PAGE: u16 = 100;
const MAX_PAGES: usize = 50;
/// How long an unanswered dispatch may stay absent from the E2E's listing
/// before it is declared lost (never dispatched again).
pub(crate) const RECONCILE_GRACE_MS: i64 = 10 * 60 * 1_000;
const SENTINEL_INGEST: &str = "sentinel::reports::ingest";
/// Observations per ingest call: Sentinel refuses a call above 500, and a
/// smaller batch also answers well inside the bus timeout.
const SENTINEL_BATCH: usize = 100;

#[derive(Clone)]
pub struct Deps {
    pub iii: Arc<IIIClient>,
    pub locks: EvalLocks,
    /// The GitHub API the pull request is opened on; `None` is
    /// [`crate::pull_request::GITHUB_API_URL`]. Never from a payload.
    github_api_url: Option<String>,
    /// The trusted candidate repository and worktrees roots; `None` reads
    /// the deployment's (`EVAL_CANDIDATE_REPOSITORY`,
    /// `EVAL_CANDIDATE_WORKTREES_DIR`). Never from a payload.
    pub(crate) candidate_roots: Option<CandidateRoots>,
}

/// Where a candidate may be checked out: fixed by the deployment, so a
/// cycle caller can never point the actor or `git` at another repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateRoots {
    pub repository: String,
    pub worktrees_dir: String,
}

pub const CANDIDATE_REPOSITORY_ENV: &str = "EVAL_CANDIDATE_REPOSITORY";
pub const CANDIDATE_WORKTREES_ENV: &str = "EVAL_CANDIDATE_WORKTREES_DIR";

fn canonical(path: &str) -> Option<std::path::PathBuf> {
    std::fs::canonicalize(path).ok()
}

impl Deps {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self {
            iii,
            locks: EvalLocks::default(),
            github_api_url: None,
            candidate_roots: None,
        }
    }

    /// Pins the trusted candidate roots (local fixtures in tests).
    pub fn with_candidate_roots(mut self, roots: CandidateRoots) -> Self {
        self.candidate_roots = Some(roots);
        self
    }

    fn candidate_roots(&self) -> Option<CandidateRoots> {
        self.candidate_roots.clone().or_else(|| {
            Some(CandidateRoots {
                repository: std::env::var(CANDIDATE_REPOSITORY_ENV).ok()?,
                worktrees_dir: std::env::var(CANDIDATE_WORKTREES_ENV).ok()?,
            })
        })
    }

    /// `Ok` only when `config` names exactly the trusted roots (canonical
    /// paths); checked before a configuration is stored and before any
    /// proposal or candidate work, so a changed deployment stops a cycle.
    pub(crate) fn check_candidate(
        &self,
        config: &crate::candidate::CandidateConfigV1,
    ) -> Result<(), EvalError> {
        let Some(roots) = self.candidate_roots() else {
            return Err(EvalError::InvalidRequest(format!(
                "candidate work needs the deployment's {CANDIDATE_REPOSITORY_ENV} and {CANDIDATE_WORKTREES_ENV}"
            )));
        };
        let same = |given: &str, trusted: &str| matches!((canonical(given), canonical(trusted)), (Some(given), Some(trusted)) if given == trusted);
        if !same(&config.repository, &roots.repository)
            || !same(&config.worktrees_dir, &roots.worktrees_dir)
        {
            return Err(EvalError::InvalidRequest(
                "candidate.repository and candidate.worktrees_dir must be the deployment's trusted candidate roots".into(),
            ));
        }
        Ok(())
    }

    /// Pins the GitHub API origin (a local fixture in tests).
    pub fn with_github_api(mut self, url: impl Into<String>) -> Self {
        self.github_api_url = Some(url.into());
        self
    }

    pub(crate) fn github_api_url(&self) -> String {
        self.github_api_url
            .clone()
            .unwrap_or_else(|| crate::pull_request::GITHUB_API_URL.into())
    }
}

pub(crate) async fn call<I: Serialize, O: DeserializeOwned>(
    deps: &Deps,
    function_id: &str,
    input: I,
    timeout_ms: u64,
) -> Result<O, EvalError> {
    let timeout = std::time::Duration::from_millis(timeout_ms.max(1));
    let request = TriggerRequest {
        function_id: function_id.into(),
        payload: serde_json::to_value(input)?,
        action: None,
        timeout_ms: Some(timeout_ms.max(1)),
    };
    let value = match tokio::time::timeout(timeout, deps.iii.trigger(request)).await {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            let message = format!("{function_id} failed: {error}");
            return Err(if matches!(error, iii_sdk::errors::Error::Timeout) {
                EvalError::Unanswered(message)
            } else {
                EvalError::Dependency(message)
            });
        }
        Err(_) => {
            return Err(EvalError::Unanswered(format!(
                "{function_id} exceeded its {timeout_ms} ms timeout"
            )))
        }
    };
    serde_json::from_value(value)
        .map_err(|error| EvalError::Serialization(format!("{function_id} response: {error}")))
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

pub fn config_revision(config: &LoopConfigV1) -> String {
    ids::sha256_json(&(RULES_VERSION, config))
}

pub async fn configure(
    deps: &Deps,
    request: ConfigureRequestV1,
) -> Result<StoredConfigV1, EvalError> {
    request.config.validate()?;
    if let Some(candidate) = &request.config.candidate {
        deps.check_candidate(candidate)?;
    }
    let stored = StoredConfigV1 {
        revision: config_revision(&request.config),
        config: request.config,
        updated_at: ids::now_ms(),
    };
    state::put_config(&deps.iii, &stored).await?;
    Ok(stored)
}

pub async fn config(deps: &Deps, _request: ConfigRequestV1) -> Result<ConfigResponseV1, EvalError> {
    Ok(ConfigResponseV1 {
        config: state::get_config(&deps.iii).await?,
        rules_version: RULES_VERSION.into(),
    })
}

// ---------------------------------------------------------------------------
// Public cycle functions
// ---------------------------------------------------------------------------

fn event(at: i64, status: CycleStatusV1, stage: CycleStageV1, detail: &str) -> CycleEventV1 {
    CycleEventV1 {
        at,
        status,
        stage,
        detail: detail.into(),
    }
}

fn source(
    execution_id: &str,
    state: ExecutionStateV1,
    admission_key: Option<String>,
) -> CycleExecutionV1 {
    CycleExecutionV1 {
        role: ExecutionRoleV1::Source,
        state,
        execution_id: Some(execution_id.into()),
        label: None,
        work_id: None,
        dispatched_at: None,
        admission_key,
        detail: None,
    }
}

fn new_cycle(
    cycle_id: String,
    input: CycleInputV1,
    executions: Vec<CycleExecutionV1>,
    config: Option<StoredConfigV1>,
    detail: &str,
) -> CycleRecordV1 {
    let now = ids::now_ms();
    let stage = match input {
        CycleInputV1::Objective(_) => CycleStageV1::Coverage,
        CycleInputV1::ExecutionId(_) => CycleStageV1::Analysis,
    };
    CycleRecordV1 {
        schema_version: CYCLE_SCHEMA_VERSION,
        cycle_id,
        input,
        status: CycleStatusV1::Queued,
        stage,
        step: 0,
        created_at: now,
        updated_at: now,
        completed_at: None,
        history: vec![event(now, CycleStatusV1::Queued, stage, detail)],
        coverage: None,
        plan: None,
        executions,
        analyses: Vec::new(),
        baseline: None,
        next: None,
        decision: None,
        outcome: None,
        failure: None,
        config,
        triage: None,
        proposal: None,
        candidate: None,
        pull_request: None,
        retry_of: None,
        insight: None,
    }
}

/// Persists the cycle, then enqueues its first step (a lost enqueue is
/// resumed by the sweep).
async fn persist_new(deps: &Deps, record: &CycleRecordV1) -> Result<(), EvalError> {
    if !state::create_cycle(&deps.iii, record).await? {
        return Err(EvalError::Conflict(format!(
            "cycle {} exists",
            record.cycle_id
        )));
    }
    queue::enqueue_step(&deps.iii, &record.cycle_id, record.step).await
}

pub async fn cycle_start(
    deps: &Deps,
    request: CycleStartRequestV1,
) -> Result<CycleStartResponseV1, EvalError> {
    if let Some(retry) = request.retry.as_deref().map(str::trim) {
        if request.objective.is_some() || request.execution_id.is_some() || retry.is_empty() {
            return Err(EvalError::InvalidRequest(
                "give exactly one of objective, execution_id or retry".into(),
            ));
        }
        return cycle_retry(deps, retry).await;
    }
    let input = request.input()?;
    let (executions, detail) = match &input {
        CycleInputV1::Objective(_) => (Vec::new(), "started from an objective"),
        CycleInputV1::ExecutionId(execution_id) => (
            vec![source(execution_id, ExecutionStateV1::Running, None)],
            "started from an existing E2E execution",
        ),
    };
    let config = state::get_config(&deps.iii).await?;
    let record = new_cycle(ids::cycle_id(), input, executions, config, detail);
    persist_new(deps, &record).await?;
    Ok(CycleStartResponseV1 {
        cycle_id: record.cycle_id,
        status: record.status,
    })
}

/// A new cycle from an ended one's finding: an execution is not imported or
/// triaged again (its analysis and a triage that proceeded are copied), so it
/// starts at coverage and plans under the current configuration.
async fn cycle_retry(deps: &Deps, cycle_id: &str) -> Result<CycleStartResponseV1, EvalError> {
    let ended = state::get_cycle(&deps.iii, cycle_id)
        .await?
        .ok_or_else(|| EvalError::NotFound(format!("cycle {cycle_id}")))?;
    if !ended.status.is_terminal() {
        return Err(EvalError::Conflict(format!(
            "cycle {cycle_id} is still {:?}: retry it once it ends",
            ended.status
        )));
    }
    let analyses: Vec<_> = ended
        .analyses
        .iter()
        .filter(|analysis| analysis.role == ExecutionRoleV1::Source)
        .cloned()
        .collect();
    let executions = match &ended.input {
        CycleInputV1::Objective(_) => Vec::new(),
        CycleInputV1::ExecutionId(execution_id) => {
            let Some(analysis) = analyses.first() else {
                return Err(EvalError::Conflict(format!(
                    "cycle {cycle_id} never analyzed execution {execution_id}: start a cycle from a new execution"
                )));
            };
            vec![source(
                execution_id,
                ExecutionStateV1::Imported,
                Some(analysis.admission_key.clone()),
            )]
        }
    };
    let config = state::get_config(&deps.iii).await?;
    let mut record = new_cycle(
        ids::cycle_id(),
        ended.input.clone(),
        executions,
        config,
        &format!("retry of cycle {cycle_id}: measured again from its finding"),
    );
    record.stage = CycleStageV1::Coverage;
    record.history[0].stage = CycleStageV1::Coverage;
    record.analyses = analyses;
    record.triage = ended
        .triage
        .filter(|triage| triage.outcome == TriageOutcomeV1::Proceed);
    record.retry_of = Some(cycle_id.into());
    persist_new(deps, &record).await?;
    Ok(CycleStartResponseV1 {
        cycle_id: record.cycle_id,
        status: record.status,
    })
}

pub async fn cycle_status(
    deps: &Deps,
    request: CycleIdRequestV1,
) -> Result<Option<CycleRecordV1>, EvalError> {
    state::get_cycle(&deps.iii, &request.cycle_id).await
}

pub async fn cycle_result(
    deps: &Deps,
    request: CycleIdRequestV1,
) -> Result<Option<CycleResultResponseV1>, EvalError> {
    let Some(cycle) = state::get_cycle(&deps.iii, &request.cycle_id).await? else {
        return Ok(None);
    };
    let usage = state::get_ledger(&deps.iii, &request.cycle_id)
        .await?
        .map(|ledger| cost::usage(&ledger, cycle.completed_at.unwrap_or_else(ids::now_ms)));
    Ok(Some(CycleResultResponseV1 { cycle, usage }))
}

pub async fn cycle_list(
    deps: &Deps,
    request: CycleListRequestV1,
) -> Result<CycleListResponseV1, EvalError> {
    let limit = request.normalized_limit()?;
    let mut cycles = state::list_cycles(&deps.iii).await?;
    cycles.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.cycle_id.cmp(&left.cycle_id))
    });
    cycles.truncate(limit);
    let now = ids::now_ms();
    let ends: std::collections::HashMap<&str, i64> = cycles
        .iter()
        .map(|cycle| (cycle.cycle_id.as_str(), cycle.completed_at.unwrap_or(now)))
        .collect();
    let usage = state::list_ledgers(&deps.iii)
        .await?
        .into_iter()
        .filter_map(|ledger| {
            let end = *ends.get(ledger.cycle_id.as_str())?;
            Some((ledger.cycle_id.clone(), cost::usage(&ledger, end)))
        })
        .collect();
    Ok(CycleListResponseV1 { cycles, usage })
}

/// The finished executions `eval::executions` offers, at most.
const EXECUTION_OPTIONS: usize = 12;

/// Recent finished E2E executions to start a cycle from, newest first. One
/// a cycle dispatched is left out; one already imported names its cycle;
/// any other is read to say what a cycle would restore.
pub async fn executions(
    deps: &Deps,
    request: ExecutionsRequestV1,
) -> Result<ExecutionsResponseV1, EvalError> {
    let listed: Value = call(
        deps,
        "e2e::dashboard::executions-list",
        json!({ "query": request.query, "limit": EXECUTIONS_PAGE }),
        BUS_TIMEOUT_MS,
    )
    .await?;
    let finished = listed["executions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| evidence::TERMINAL_STATUSES.contains(&evidence::status_of(entry).as_str()))
        .filter(|entry| {
            !evidence::text_at(entry, "/idempotency_key")
                .and_then(|key| key.split_once(':').map(|(owner, _)| ids::is_cycle(owner)))
                .unwrap_or(false)
        })
        .filter_map(|entry| Some((evidence::text_at(entry, "/id")?, entry)))
        .take(EXECUTION_OPTIONS);
    // ponytail: one execution-get per unclaimed option; a listing that named
    // what each run lost would spare them.
    let options =
        futures_util::future::join_all(finished.map(|(execution_id, entry)| async move {
            let claim = state::get_admission(&deps.iii, &admission_key(entry, &execution_id))
                .await
                .ok()
                .flatten();
            let scenarios = match claim {
                Some(_) => Vec::new(),
                None => execution_get(deps, &execution_id)
                    .await
                    .map_or_else(|_| Vec::new(), |bundle| scenarios_of(&bundle, entry)),
            };
            ExecutionOptionV1 {
                status: evidence::status_of(entry),
                started_at: evidence::text_at(entry, "/started_at"),
                cycle_id: claim.map(|claim| claim.cycle_id),
                scenarios,
                execution_id,
            }
        }))
        .await;
    Ok(ExecutionsResponseV1 {
        executions: options,
    })
}

/// Each reported scenario with its mean score and what its runs lost.
fn scenarios_of(bundle: &Value, summary: &Value) -> Vec<ExecutionScenarioV1> {
    let mut ids: Vec<String> = bundle["detail"]["reports"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|report| evidence::text_at(report, "/scenario_id"))
        .collect();
    ids.dedup();
    ids.into_iter()
        .map(|scenario_id| ExecutionScenarioV1 {
            mean_score: mean_score(summary, &scenario_id),
            lost: coverage::lost(bundle, &scenario_id).map(|(_, _, why)| why),
            scenario_id,
        })
        .collect()
}

/// A listed scenario's mean score, from the summary's subjects.
fn mean_score(summary: &Value, scenario_id: &str) -> Option<f64> {
    summary["subjects"]
        .as_array()?
        .iter()
        .flat_map(|subject| subject["scenarios"].as_array().into_iter().flatten())
        .find(|scenario| scenario["id"] == scenario_id)?["mean_score"]
        .as_f64()
}

/// Takes the cycle's lock, so a step dispatching work finishes first; the
/// bumped `step` makes any queued step stale. Executions the cycle
/// dispatched and that still run are asked to stop.
pub async fn cycle_cancel(
    deps: &Deps,
    request: CycleIdRequestV1,
) -> Result<CycleCancelResponseV1, EvalError> {
    let _guard = deps.locks.guard(&request.cycle_id).await;
    let now = ids::now_ms();
    let changed = state::update_cycle(&deps.iii, &request.cycle_id, |stored| {
        if stored.status.is_terminal() {
            return Ok((None, (false, stored.clone())));
        }
        let mut next = stored.clone();
        apply(
            &mut next,
            now,
            CycleStatusV1::Cancelled,
            CycleStageV1::Done,
            "cancelled",
        );
        Ok((Some(next.clone()), (true, next)))
    })
    .await?
    .ok_or_else(|| EvalError::NotFound(request.cycle_id.clone()))?;
    let (cancelled, record) = changed;
    if cancelled {
        crate::stages::stop_actors(deps, &record).await;
        stop_executions(deps, &record).await;
    }
    Ok(CycleCancelResponseV1 {
        cancelled,
        status: record.status,
    })
}

/// Ends a waiting cycle past a budget limit as `budget`, under its lock and
/// from the stored record, never a snapshot; its actors and executions are
/// asked to stop, as a cancel does.
async fn expire(
    deps: &Deps,
    cycle_id: &str,
    reason: &str,
) -> Result<Option<CycleRecordV1>, EvalError> {
    let _guard = deps.locks.guard(cycle_id).await;
    let now = ids::now_ms();
    let written = state::update_cycle(&deps.iii, cycle_id, |stored| {
        if stored.status != CycleStatusV1::Waiting {
            return Ok((None, None));
        }
        let mut next = stored.clone();
        next.outcome = Some(CycleOutcomeV1 {
            code: OutcomeCodeV1::Budget,
            detail: reason.into(),
        });
        apply(
            &mut next,
            now,
            CycleStatusV1::Completed,
            CycleStageV1::Done,
            &format!("{:?}: {reason}", OutcomeCodeV1::Budget),
        );
        Ok((Some(next.clone()), Some(next)))
    })
    .await?
    .flatten();
    if let Some(record) = &written {
        crate::stages::stop_actors(deps, record).await;
        stop_executions(deps, record).await;
    }
    Ok(written)
}

/// Asks every execution the cycle dispatched and that still runs to stop
/// (best effort).
pub(crate) async fn stop_executions(deps: &Deps, record: &CycleRecordV1) {
    for execution in record.executions.iter().filter(|execution| {
        execution.role != ExecutionRoleV1::Source && execution.state == ExecutionStateV1::Running
    }) {
        let Some(execution_id) = &execution.execution_id else {
            continue;
        };
        let payload = json!({ "execution_id": execution_id });
        let result = call::<_, Value>(
            deps,
            "e2e::dashboard::execution-cancel",
            payload,
            BUS_TIMEOUT_MS,
        )
        .await;
        if let Err(error) = result {
            tracing::warn!(%execution_id, %error, "could not stop a cycle's execution");
        }
    }
}

/// Moves `record` to `status`/`stage`: the step advances (a queued step of
/// the old one is stale) and the history records why.
fn apply(
    record: &mut CycleRecordV1,
    now: i64,
    status: CycleStatusV1,
    stage: CycleStageV1,
    detail: &str,
) {
    record.status = status;
    record.stage = stage;
    record.step += 1;
    record.updated_at = now;
    if status.is_terminal() {
        record.completed_at = Some(now);
        record.next = None;
    }
    record.history.push(event(now, status, stage, detail));
}

/// Applies `mutate` and the move to the cycle still at `step`, then
/// enqueues the next step when it keeps running. `None`: the cycle moved on
/// (cancelled, or another step won) and nothing was written.
pub(crate) async fn transition(
    deps: &Deps,
    cycle_id: &str,
    step: u64,
    status: CycleStatusV1,
    stage: CycleStageV1,
    detail: &str,
    mut mutate: impl FnMut(&mut CycleRecordV1) -> Result<(), EvalError>,
) -> Result<Option<CycleRecordV1>, EvalError> {
    let now = ids::now_ms();
    let written = state::update_cycle(&deps.iii, cycle_id, |stored| {
        if stored.step != step || stored.status.is_terminal() {
            return Ok((None, None));
        }
        let mut next = stored.clone();
        mutate(&mut next)?;
        apply(&mut next, now, status, stage, detail);
        Ok((Some(next.clone()), Some(next)))
    })
    .await?
    .flatten();
    if let Some(record) = &written {
        if record.status == CycleStatusV1::Running {
            queue::enqueue_step(&deps.iii, cycle_id, record.step).await?;
        }
    }
    Ok(written)
}

pub(crate) async fn end(
    deps: &Deps,
    record: &CycleRecordV1,
    code: OutcomeCodeV1,
    detail: &str,
) -> Result<Option<CycleRecordV1>, EvalError> {
    transition(
        deps,
        &record.cycle_id,
        record.step,
        CycleStatusV1::Completed,
        CycleStageV1::Done,
        &format!("{code:?}: {detail}"),
        |next| {
            next.outcome = Some(CycleOutcomeV1 {
                code,
                detail: detail.into(),
            });
            Ok(())
        },
    )
    .await
}

pub(crate) async fn fail(
    deps: &Deps,
    record: &CycleRecordV1,
    reason: &str,
) -> Result<Option<CycleRecordV1>, EvalError> {
    transition(
        deps,
        &record.cycle_id,
        record.step,
        CycleStatusV1::Failed,
        CycleStageV1::Done,
        reason,
        |next| {
            next.failure = Some(reason.into());
            Ok(())
        },
    )
    .await
}

// ---------------------------------------------------------------------------
// The durable step
// ---------------------------------------------------------------------------

fn skipped(status: CycleStatusV1) -> StepResponseV1 {
    StepResponseV1 {
        skipped: true,
        status,
    }
}

/// Stages a waiting cycle steps through on the sweep's cadence: an
/// unanswered dispatch to reconcile, an actor or an official operation to
/// poll.
const POLLED: [CycleStageV1; 5] = [
    CycleStageV1::Baseline,
    CycleStageV1::Proposal,
    CycleStageV1::Candidate,
    CycleStageV1::Validation,
    CycleStageV1::PullRequest,
];

pub async fn step(deps: &Deps, request: StepRequestV1) -> Result<StepResponseV1, EvalError> {
    let _guard = deps.locks.guard(&request.cycle_id).await;
    let Some(record) = state::get_cycle(&deps.iii, &request.cycle_id).await? else {
        return Ok(skipped(CycleStatusV1::Cancelled));
    };
    let runnable = match record.status {
        CycleStatusV1::Queued | CycleStatusV1::Running => true,
        CycleStatusV1::Waiting => POLLED.contains(&record.stage),
        _ => false,
    };
    if !runnable || record.step != request.step {
        return Ok(skipped(record.status));
    }
    // Each stage's future lives on the heap: together they are far larger
    // than a worker thread's stack in an unoptimized build.
    let written = match record.stage {
        CycleStageV1::Coverage => Box::pin(coverage_stage(deps, &record)).await?,
        CycleStageV1::Baseline => Box::pin(baseline_stage(deps, &record)).await?,
        CycleStageV1::Analysis => Box::pin(analysis_stage(deps, &record)).await?,
        CycleStageV1::Triage => Box::pin(crate::stages::triage_stage(deps, &record)).await?,
        CycleStageV1::Insight => Box::pin(crate::stages::insight_stage(deps, &record)).await?,
        CycleStageV1::Proposal => Box::pin(crate::stages::proposal_stage(deps, &record)).await?,
        CycleStageV1::Candidate => Box::pin(crate::stages::candidate_stage(deps, &record)).await?,
        CycleStageV1::Validation => {
            Box::pin(crate::stages::validation_stage(deps, &record)).await?
        }
        CycleStageV1::PullRequest => Box::pin(crate::pull_request::stage(deps, &record)).await?,
        CycleStageV1::AwaitingExecution | CycleStageV1::Done => None,
    };
    Ok(match written {
        Some(record) => StepResponseV1 {
            skipped: false,
            status: record.status,
        },
        None => skipped(record.status),
    })
}

/// Every `tests-list` row, page by page.
async fn tests_catalog(deps: &Deps) -> Result<(Vec<Value>, String), EvalError> {
    let (mut rows, mut cursor) = (Vec::new(), None::<String>);
    for _ in 0..MAX_PAGES {
        let page: Value = call(
            deps,
            "e2e::dashboard::tests-list",
            json!({ "limit": TESTS_PAGE, "cursor": cursor }),
            BUS_TIMEOUT_MS,
        )
        .await?;
        rows.extend(page["rows"].as_array().cloned().unwrap_or_default());
        cursor = page["next_cursor"].as_str().map(str::to_string);
        if cursor.is_none() {
            return Ok((rows, page["revision"].as_str().unwrap_or_default().into()));
        }
    }
    Err(EvalError::Dependency(format!(
        "tests-list still had pages after {MAX_PAGES}"
    )))
}

/// Opens the cycle's budget ledger from its frozen configuration, once.
pub(crate) async fn open_ledger(deps: &Deps, record: &CycleRecordV1) -> Result<(), EvalError> {
    let Some(stored) = &record.config else {
        return Err(EvalError::State(format!(
            "cycle {} has no frozen configuration",
            record.cycle_id
        )));
    };
    state::update_ledger(&deps.iii, &record.cycle_id, |current| {
        Ok(match current {
            Some(_) => (None, ()),
            None => (
                Some(cost::new_ledger(
                    &record.cycle_id,
                    stored.config.budget.clone(),
                    record.created_at,
                )),
                (),
            ),
        })
    })
    .await
}

/// Coverage from the whole catalog, then the plan frozen and the budget
/// opened before anything is dispatched. An objective maps to its need; an
/// execution with pertinent findings maps to what its own runs lost, on its
/// executed scenarios still current. No coverage spends nothing.
async fn coverage_stage(
    deps: &Deps,
    record: &CycleRecordV1,
) -> Result<Option<CycleRecordV1>, EvalError> {
    let stored = record.config.clone();
    let (rows, revision) = tests_catalog(deps).await?;
    let catalog: Value = call(deps, "e2e::scenarios-list", json!({}), BUS_TIMEOUT_MS).await?;
    let (max, controls) = stored.as_ref().map_or((1, Vec::new()), |stored| {
        (
            stored.config.max_scenarios as usize,
            stored.config.controls.clone(),
        )
    });
    let now = ids::now_ms();
    // Recorded events per run of each test, which size a count's proof.
    let mut observed_rate: std::collections::BTreeMap<String, f64> = Default::default();
    let (found, goal) = match &record.input {
        CycleInputV1::Objective(objective) => {
            let model = stored
                .as_ref()
                .map(|stored| stored.config.subject.model.as_str());
            let mut observed = std::collections::BTreeMap::new();
            let mut unread = Vec::new();
            // Measured from history, a test counts only if the window's runs
            // ran it, and only those runs are counted: nothing will be
            // dispatched to measure another.
            let mut rows = rows.clone();
            let mut window: Option<std::collections::BTreeSet<String>> = None;
            if let (Some(days), Some(model)) = (
                stored
                    .as_ref()
                    .and_then(|stored| stored.config.history_days),
                model,
            ) {
                let listed = history_window(deps, days, model).await?;
                window = Some(listed.iter().map(|(id, _)| id.clone()).collect());
                let ran: std::collections::BTreeSet<String> = listed
                    .into_iter()
                    .flat_map(|(_, scenarios)| scenarios)
                    .collect();
                rows.retain(|row| {
                    row["test_id"]
                        .as_str()
                        .is_some_and(|test_id| ran.contains(test_id))
                });
                unread.push(format!(
                    "history_days: only the {} test(s) the GitHub runs of the last {days} day(s) ran are considered",
                    rows.len()
                ));
            }
            if let (Some(true), Some(model)) = (
                coverage::need_of(objective).map(|need| need.observed),
                model,
            ) {
                for test_id in rows.iter().filter_map(|row| row["test_id"].as_str()) {
                    // One unreadable history leaves that test out, not the objective.
                    match call::<_, Value>(
                        deps,
                        "e2e::dashboard::test-history-get",
                        json!({"test_id": test_id, "subject_model": model, "limit": 100}),
                        BUS_TIMEOUT_MS,
                    )
                    .await
                    {
                        Ok(mut history) => {
                            if let Some(window) = &window {
                                if let Some(observations) = history["observations"].as_array_mut()
                                {
                                    observations.retain(|observation| {
                                        observation["plan_execution_id"]
                                            .as_str()
                                            .is_some_and(|id| window.contains(id))
                                    });
                                }
                            }
                            observed
                                .insert(test_id.to_string(), coverage::observed_errors(&history));
                        }
                        Err(error) => unread.push(format!(
                            "{test_id}: its recorded runs could not be read ({error}); not considered"
                        )),
                    }
                }
            }
            // What the plan's stack runs: its containers and the workers the
            // registry brings with each. Unknown: no test is left out for it.
            let mut provided = None;
            if let Some(validation) = stored
                .as_ref()
                .and_then(|stored| stored.config.validation.as_ref())
            {
                let declared = coverage::declared_workers(&validation.source_stack_template);
                let mut run = declared.clone();
                for worker in &declared {
                    match call::<_, Value>(
                        deps,
                        "e2e::dashboard::worker-resolve",
                        json!({"worker": worker}),
                        BUS_TIMEOUT_MS,
                    )
                    .await
                    {
                        Ok(resolved) => run.extend(
                            resolved["dependencies"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                                .map(str::to_string),
                        ),
                        Err(error) => {
                            unread.push(format!(
                                "the stack's {worker} could not be resolved ({error}); tests were not checked against the stack"
                            ));
                            run.clear();
                            break;
                        }
                    }
                }
                provided = (!run.is_empty()).then_some(run);
            }
            // Jev judges how well each test with recorded runs exercises the
            // objective: many calls, a complex task. Without Jev the
            // recorded failures alone rank them.
            if let Some(judge) = stored
                .as_ref()
                .and_then(|stored| stored.config.judge.clone())
                .filter(|_| !observed.is_empty())
            {
                open_ledger(deps, record).await?;
                let work = CycleWorkV1 {
                    work_id: format!("{}:fit", record.cycle_id),
                    kind: CycleWorkKindV1::Jev,
                    estimated_usd: judge.estimated_usd,
                    candidate: false,
                };
                if let cost::Reservation::OverLimit(detail) = reserve(deps, record, &work).await? {
                    return end(deps, record, OutcomeCodeV1::Budget, &detail).await;
                }
                let question = coverage::fit_question();
                let evaluations: Vec<judge_contract::Evaluation> = rows
                    .iter()
                    .filter_map(|row| {
                        let test_id = row["test_id"].as_str()?;
                        let seen = observed.get(test_id).filter(|seen| seen.runs > 0)?;
                        Some(judge_contract::Evaluation {
                            id: test_id.into(),
                            state: coverage::fit_state(objective, row, seen),
                            questions: std::collections::BTreeMap::from([(
                                "fit".into(),
                                question.clone(),
                            )]),
                        })
                    })
                    .collect();
                let mut payload = serde_json::to_value(judge_contract::EvaluateRequest {
                    options: Default::default(),
                    request_id: Some(format!("{}-fit", record.cycle_id)),
                    model: None,
                    timeout_ms: 60_000,
                    expires_at_unix_ms: None,
                    evaluations,
                })?;
                payload["provider"] = json!(judge.provider);
                let answered: Result<judge_contract::EvaluateResponse, EvalError> = call(
                    deps,
                    judge_contract::FUNCTION_ID,
                    payload,
                    60_000 + BUS_TIMEOUT_MS,
                )
                .await;
                settle(deps, &record.cycle_id, &work.work_id, None).await?;
                match answered {
                    Ok(judge_contract::EvaluateResponse::Ok { results, .. }) => {
                        for (test_id, result) in results {
                            let answer = result.answers.get("fit").filter(|answer| {
                                judge_contract::validate_answer(&question, answer).is_ok()
                            });
                            if let (Some(judge_contract::Answer::Score { score, .. }), Some(seen)) =
                                (answer, observed.get_mut(&test_id))
                            {
                                seen.fit = Some(*score);
                            }
                        }
                    }
                    Ok(judge_contract::EvaluateResponse::Error { code, .. }) => {
                        unread.push(format!(
                            "Jev did not rank the tests ({code:?}); ranked by recorded failures"
                        ))
                    }
                    Err(error) => unread.push(format!(
                        "Jev did not rank the tests ({error}); ranked by recorded failures"
                    )),
                }
            }
            observed_rate = observed
                .iter()
                .filter(|(_, seen)| seen.runs > 0)
                .map(|(test_id, seen)| (test_id.clone(), seen.total / f64::from(seen.runs)))
                .collect();
            let mut found = coverage::select(
                objective,
                &rows,
                &revision,
                &catalog,
                max,
                &controls,
                &observed,
                provided.as_ref(),
                now,
            );
            found.gaps.extend(unread);
            let goal = coverage::need_goal(&found)
                .filter(|_| found.covered)
                .map(|goal| (objective.clone(), goal));
            (found, goal)
        }
        CycleInputV1::ExecutionId(_) => {
            let key = record
                .analyses
                .iter()
                .find(|analysis| analysis.role == ExecutionRoleV1::Source)
                .map(|analysis| analysis.admission_key.clone())
                .ok_or_else(|| {
                    EvalError::State("coverage of an execution needs its analysis".into())
                })?;
            let source = state::get_evidence(&deps.iii, &key)
                .await?
                .ok_or_else(|| EvalError::State(format!("evidence {key} is missing")))?;
            let (found, goal) =
                coverage::derive(&source, &rows, &revision, &catalog, max, &controls, now);
            let goal = goal.map(|(objective, metric, criterion_id)| {
                (objective, (metric, DirectionV1::Increase, criterion_id))
            });
            (found, goal)
        }
    };
    let with_coverage = |next: &mut CycleRecordV1| -> Result<(), EvalError> {
        next.coverage = Some(found.clone());
        Ok(())
    };
    let Some((objective, goal)) = goal else {
        let detail = format!("no coverage: {}", found.gaps.join("; "));
        return transition(
            deps,
            &record.cycle_id,
            record.step,
            CycleStatusV1::Completed,
            CycleStageV1::Done,
            &detail,
            |next| {
                with_coverage(next)?;
                next.outcome = Some(CycleOutcomeV1 {
                    code: OutcomeCodeV1::NoCoverage,
                    detail: found.gaps.join("; "),
                });
                Ok(())
            },
        )
        .await;
    };
    let Some(stored) = stored else {
        let reason = "eval was not configured when the cycle started: call eval::configure with the E2E subject, where/stack and budget, then start a new cycle";
        return transition(
            deps,
            &record.cycle_id,
            record.step,
            CycleStatusV1::Failed,
            CycleStageV1::Done,
            reason,
            |next| {
                with_coverage(next)?;
                next.failure = Some(reason.into());
                Ok(())
            },
        )
        .await;
    };
    let Some(mut plan) = coverage::plan(&objective, &found, goal, &stored, now) else {
        return fail(deps, record, "the coverage maps to no plan").await;
    };
    // A rare count is proven only over enough rounds: measured from history,
    // as many as give the base about PROOF_EVENTS events, within bounds.
    if stored.config.history_days.is_some() && decision::is_count(plan.criterion.metric) {
        if let Some(rate) = observed_rate
            .get(plan.criterion.scenario_id.as_str())
            .copied()
            .filter(|rate| *rate > 0.0)
        {
            let planned = plan.sampling.runs_per_side;
            let rounds = ((PROOF_EVENTS / rate).ceil() as u32).clamp(planned, MAX_PROOF_ROUNDS);
            plan.estimated_usd *= f64::from(rounds) / f64::from(planned.max(1));
            plan.sampling.runs_per_side = rounds;
            plan.parameters.runs = rounds;
        }
    }
    open_ledger(deps, record).await?;
    let detail = format!(
        "plan frozen under policy {} over {}",
        stored.revision,
        plan.parameters.scenarios.join(", ")
    );
    // An execution's findings are triaged only once covered and planned.
    let stage = match (&record.input, &record.triage) {
        (CycleInputV1::ExecutionId(_), None) => CycleStageV1::Triage,
        _ => CycleStageV1::Baseline,
    };
    transition(
        deps,
        &record.cycle_id,
        record.step,
        CycleStatusV1::Running,
        stage,
        &detail,
        |next| {
            with_coverage(next)?;
            decision::freeze(next, plan.clone())
        },
    )
    .await
}
/// The id of the execution listed under an immutable dispatch key, if the E2E
/// lists it exactly once on a complete page.
pub(crate) async fn reconcile_dispatch(
    deps: &Deps,
    idempotency_key: &str,
) -> Result<Option<String>, EvalError> {
    let listed: Value = call(
        deps,
        "e2e::dashboard::executions-list",
        json!({ "query": idempotency_key, "limit": EXECUTIONS_PAGE }),
        BUS_TIMEOUT_MS,
    )
    .await?;
    Ok(evidence::unique_execution_id(&listed, idempotency_key))
}

pub(crate) fn running(execution_id: String) -> impl FnMut(&mut CycleExecutionV1) {
    move |entry| {
        entry.execution_id = Some(execution_id.clone());
        entry.state = ExecutionStateV1::Running;
    }
}

pub(crate) fn update_entry(
    record: &mut CycleRecordV1,
    label: &str,
    mut change: impl FnMut(&mut CycleExecutionV1),
) {
    if let Some(entry) = record
        .executions
        .iter_mut()
        .find(|entry| entry.label.as_deref() == Some(label))
    {
        change(entry);
    }
}

fn update_work_entry(
    record: &mut CycleRecordV1,
    work_id: &str,
    mut change: impl FnMut(&mut CycleExecutionV1),
) {
    if let Some(entry) = record
        .executions
        .iter_mut()
        .find(|entry| entry.work_id.as_deref() == Some(work_id))
    {
        change(entry);
    }
}

pub(crate) fn awaiting(detail: &str) -> CycleNextV1 {
    CycleNextV1 {
        action: NextActionV1::AwaitExecution,
        detail: detail.into(),
        findings: Vec::new(),
    }
}

/// The E2E's refusal while its one `where: harness` runner holds another
/// execution: nothing was started, so the same work can be dispatched again.
// ponytail: matched on the E2E's message; switch to a structured code once
// execution-start returns one.
const RUNNER_BUSY: &str = "is still running; wait for it to finish or cancel it";

pub(crate) fn runner_busy(error: &EvalError) -> bool {
    error.to_string().contains(RUNNER_BUSY)
}

/// Reserves `work` on the cycle's ledger (opened from the frozen
/// configuration if needed). Only `Reserved` is new; `Existing` was
/// reserved before a redelivery or restart.
pub(crate) async fn reserve(
    deps: &Deps,
    record: &CycleRecordV1,
    work: &CycleWorkV1,
) -> Result<cost::Reservation, EvalError> {
    open_ledger(deps, record).await?;
    let now = ids::now_ms();
    state::update_ledger(&deps.iii, &record.cycle_id, |ledger| {
        let mut ledger = ledger.ok_or_else(|| {
            EvalError::State(format!("cycle {} has no budget ledger", record.cycle_id))
        })?;
        Ok(match cost::reserve(&mut ledger, work, now)? {
            reserved @ cost::Reservation::Reserved(_) => (Some(ledger), reserved),
            other => (None, other),
        })
    })
    .await
}

async fn baseline_stage(
    deps: &Deps,
    record: &CycleRecordV1,
) -> Result<Option<CycleRecordV1>, EvalError> {
    let Some(plan) = record.plan.clone() else {
        return fail(deps, record, "a baseline needs a frozen plan").await;
    };
    if let Some(days) = record
        .config
        .as_ref()
        .and_then(|stored| stored.config.history_days)
    {
        return Box::pin(history_stage(deps, record, &plan, days)).await;
    }
    dispatch(
        deps,
        record,
        ExecutionRoleV1::Baseline,
        "baseline",
        &plan.parameters,
        plan.estimated_usd,
    )
    .await
}

/// Reserve, record ownership, then `execution-start` with `parameters` and
/// the deterministic label `eval:<cycle_id>:<name>` and immutable idempotency
/// key `work_id`. A reservation found
/// again never dispatches twice: an entry already `dispatching` is
/// reconciled by its idempotency key in the E2E listing. Answered, the cycle waits
/// for the execution; unanswered, it waits at its current stage, polled.
/// Events a count's proof is sized to expect on the base.
const PROOF_EVENTS: f64 = 8.0;
/// The most rounds per side a sized proof runs.
const MAX_PROOF_ROUNDS: u32 = 20;

/// At most this many past runs are read into one history baseline.
const MAX_HISTORY_RUNS: usize = 40;

/// The past E2E runs the loop reads instead of dispatching: finished GitHub
/// executions (the release-control campaigns) of `model`, completed in the
/// last `days`, newest first, each with the scenarios it ran.
pub(crate) async fn history_window(
    deps: &Deps,
    days: u32,
    model: &str,
) -> Result<Vec<(String, Vec<String>)>, EvalError> {
    let since = ids::date_days_before(ids::now_ms(), days);
    let mut found = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let page: Value = call(
            deps,
            "e2e::dashboard::executions-list",
            json!({ "limit": EXECUTIONS_PAGE, "cursor": cursor }),
            BUS_TIMEOUT_MS,
        )
        .await?;
        let rows = page["executions"].as_array().cloned().unwrap_or_default();
        let mut older = false;
        for row in &rows {
            let completed = row["completed_at"].as_str().unwrap_or_default();
            if !completed.is_empty() && completed < since.as_str() {
                older = true;
                continue;
            }
            let finished = matches!(
                row["status"].as_str(),
                Some("passed" | "failed" | "incomplete")
            );
            if row["source"]["kind"] == "github" && finished && row["parameters"]["model"] == model
            {
                let scenarios = row["parameters"]["scenarios"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                found.push((
                    row["id"].as_str().unwrap_or_default().to_string(),
                    scenarios,
                ));
            }
        }
        cursor = page["next_cursor"].as_str().map(str::to_string);
        if cursor.is_none()
            || (older
                && rows.iter().all(|row| {
                    row["completed_at"]
                        .as_str()
                        .is_some_and(|at| !at.is_empty() && at < since.as_str())
                }))
        {
            break;
        }
    }
    Ok(found)
}

/// The baseline from runs already made: every execution of the window that
/// ran a planned scenario, read into one evidence the insight and the room
/// check read like a dispatched baseline's. Nothing is dispatched.
async fn history_stage(
    deps: &Deps,
    record: &CycleRecordV1,
    plan: &CyclePlanV1,
    days: u32,
) -> Result<Option<CycleRecordV1>, EvalError> {
    let window: std::collections::BTreeSet<String> =
        history_window(deps, days, &plan.parameters.model)
            .await?
            .into_iter()
            .map(|(execution_id, _)| execution_id)
            .collect();
    // Run by run, never a whole execution: a campaign's full report can pass
    // the bus frame limit. Each run's projection and its retained transcript.
    let mut reports = Vec::new();
    let mut read = std::collections::BTreeSet::new();
    let mut runs_read = 0;
    for scenario_id in &plan.parameters.scenarios {
        let history: Value = call(
            deps,
            "e2e::dashboard::test-history-get",
            json!({"test_id": scenario_id, "subject_model": plan.parameters.model, "limit": 100}),
            BUS_TIMEOUT_MS,
        )
        .await?;
        let mut runs = Vec::new();
        for observation in history["observations"].as_array().into_iter().flatten() {
            let plan_execution = observation["plan_execution_id"]
                .as_str()
                .unwrap_or_default();
            let native = observation["execution_id"].as_str().unwrap_or_default();
            if !window.contains(plan_execution) || native.is_empty() {
                continue;
            }
            for run in observation["runs"].as_array().into_iter().flatten() {
                if runs_read == MAX_HISTORY_RUNS {
                    break;
                }
                let (Some(run_id), Some(attempt_id)) =
                    (run["run_id"].as_str(), run["attempt_id"].as_str())
                else {
                    continue;
                };
                let Ok(mut projected) = call::<_, Value>(
                    deps,
                    "e2e::dashboard::attempt-get",
                    json!({"execution_id": native, "run_id": run_id, "attempt_id": attempt_id}),
                    BUS_TIMEOUT_MS,
                )
                .await
                else {
                    continue;
                };
                let transcript = call::<_, Value>(
                    deps,
                    "e2e::dashboard::evidence-read",
                    json!({"execution_id": native,
                        "path": format!("evidence/{run_id}/{attempt_id}/transcript.json")}),
                    BUS_TIMEOUT_MS,
                )
                .await
                .ok()
                .and_then(|file| {
                    use base64::Engine as _;
                    base64::engine::general_purpose::STANDARD
                        .decode(file["base64"].as_str()?)
                        .ok()
                })
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
                projected["run_id"] = json!(run_id);
                projected["attempt_id"] = json!(attempt_id);
                if let Some(transcript) = transcript {
                    projected["transcript"] = transcript;
                }
                runs.push(projected);
                runs_read += 1;
                read.insert(plan_execution.to_string());
            }
        }
        if !runs.is_empty() {
            reports.push(json!({"scenario_id": scenario_id, "available": true,
                "report": {"scenarios": [{"scenario_id": scenario_id, "runs": runs}]}}));
        }
    }
    let wanted: Vec<String> = read.into_iter().collect();
    if wanted.is_empty() {
        let detail = format!(
            "no GitHub run of {} with {} finished in the last {days} day(s): import the \
             release-control runs, or widen history_days",
            plan.parameters.scenarios.join(", "),
            plan.parameters.model
        );
        return end(deps, record, OutcomeCodeV1::Inconclusive, &detail).await;
    }
    let summaries: Vec<Value> = wanted
        .iter()
        .map(|id| json!({"id": id, "status": "passed"}))
        .collect();
    let execution_id = format!("history:{}", wanted.len());
    let merged = json!({"manifest": {"executions": summaries}, "detail": {"reports": reports}});
    let evidence = ExecutionEvidenceV1 {
        admission_key: format!("{}:history", record.cycle_id),
        execution_id: execution_id.clone(),
        imported_at: ids::now_ms(),
        bundle_digest: ids::sha256_json(&merged),
        identity: evidence::identity(&merged, &execution_id),
        bundle: merged,
    };
    state::put_evidence_once(&deps.iii, &evidence).await?;
    let measured = decision::baseline_from_history(plan, &execution_id, &evidence.bundle);
    let analysis = analysis_of(&evidence, ExecutionRoleV1::Baseline, None);
    let reads = reads_transcripts(plan);
    let room = (!reads)
        .then(|| {
            decision::no_room(
                &plan.criterion,
                plan.sampling.runs_per_side,
                &measured.primary,
            )
        })
        .flatten();
    let candidate_work = record
        .config
        .as_ref()
        .is_some_and(|stored| stored.config.candidate.is_some());
    let (status, stage, outcome) = if !measured.usable {
        (
            CycleStatusV1::Completed,
            CycleStageV1::Done,
            Some((
                OutcomeCodeV1::Inconclusive,
                format!("history unusable: {}", measured.reason),
            )),
        )
    } else if reads {
        (CycleStatusV1::Running, CycleStageV1::Insight, None)
    } else if !candidate_work {
        (
            CycleStatusV1::Completed,
            CycleStageV1::Done,
            Some((
                OutcomeCodeV1::Inconclusive,
                format!(
                    "history measured ({}); candidate work is not configured (config.candidate)",
                    measured.reason
                ),
            )),
        )
    } else if let Some(reason) = room {
        (
            CycleStatusV1::Completed,
            CycleStageV1::Done,
            Some((
                OutcomeCodeV1::NoAction,
                format!("history measured; {reason}"),
            )),
        )
    } else {
        (CycleStatusV1::Running, CycleStageV1::Proposal, None)
    };
    let detail = outcome.as_ref().map_or_else(
        || {
            format!(
                "history measured from {} past execution(s): {}",
                wanted.len(),
                measured.reason
            )
        },
        |(_, detail)| detail.clone(),
    );
    transition(
        deps,
        &record.cycle_id,
        record.step,
        status,
        stage,
        &detail,
        |next| {
            next.executions.push(CycleExecutionV1 {
                role: ExecutionRoleV1::Baseline,
                state: ExecutionStateV1::Imported,
                execution_id: Some(execution_id.clone()),
                label: Some(format!("history of {days} day(s)")),
                work_id: None,
                dispatched_at: None,
                admission_key: Some(evidence.admission_key.clone()),
                detail: Some(
                    wanted
                        .iter()
                        .map(|id| id.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            });
            next.analyses.push(analysis.clone());
            next.baseline = Some(measured.clone());
            if let Some((code, detail)) = &outcome {
                next.outcome = Some(CycleOutcomeV1 {
                    code: *code,
                    detail: detail.clone(),
                });
            }
            Ok(())
        },
    )
    .await
}

/// A plan on a Harness metric the insight reads in the transcripts: it runs
/// before the room and candidate checks, since what the runs show is its own
/// result.
fn reads_transcripts(plan: &CyclePlanV1) -> bool {
    plan.criterion.metric == CriterionMetricV1::FunctionCallErrors
}

pub(crate) async fn dispatch(
    deps: &Deps,
    record: &CycleRecordV1,
    role: ExecutionRoleV1,
    name: &str,
    parameters: &ExecutionParametersV1,
    estimated_usd: f64,
) -> Result<Option<CycleRecordV1>, EvalError> {
    let (cycle_id, step, stage) = (record.cycle_id.as_str(), record.step, record.stage);
    let label = ids::execution_label(cycle_id, name);
    let work_id = format!("{cycle_id}:{name}");
    let wait = |detail: String, change: Option<String>| {
        let label = label.clone();
        async move {
            transition(
                deps,
                cycle_id,
                step,
                CycleStatusV1::Waiting,
                if change.is_some() {
                    CycleStageV1::AwaitingExecution
                } else {
                    stage
                },
                &detail,
                |next| {
                    if let Some(execution_id) = &change {
                        update_entry(next, &label, running(execution_id.clone()));
                    }
                    next.next = Some(awaiting(&detail));
                    Ok(())
                },
            )
            .await
        }
    };
    if let Some(entry) = record
        .executions
        .iter()
        .find(|entry| entry.work_id.as_deref() == Some(&work_id))
    {
        if let Some(execution_id) = &entry.execution_id {
            return wait(
                format!("{name} {execution_id} running"),
                Some(execution_id.clone()),
            )
            .await;
        }
        if let Some(execution_id) = reconcile_dispatch(deps, &work_id).await? {
            return wait(
                format!("{name} {execution_id} found by its idempotency key"),
                Some(execution_id),
            )
            .await;
        }
        if ids::now_ms() - entry.dispatched_at.unwrap_or(record.updated_at) < RECONCILE_GRACE_MS {
            return wait(
                format!("dispatch of {label} unanswered; reconciling by idempotency key"),
                None,
            )
            .await;
        }
        settle(deps, cycle_id, &work_id, None).await?;
        let detail = format!("the dispatch {label} never appeared in the E2E");
        return transition(
            deps,
            cycle_id,
            step,
            CycleStatusV1::Completed,
            CycleStageV1::Done,
            &detail,
            |next| {
                update_entry(next, &label, |entry| entry.state = ExecutionStateV1::Lost);
                next.outcome = Some(CycleOutcomeV1 {
                    code: OutcomeCodeV1::Inconclusive,
                    detail: detail.clone(),
                });
                Ok(())
            },
        )
        .await;
    }
    let work = CycleWorkV1 {
        work_id: work_id.clone(),
        kind: CycleWorkKindV1::E2e,
        estimated_usd,
        candidate: false,
    };
    if let cost::Reservation::OverLimit(detail) = reserve(deps, record, &work).await? {
        return end(deps, record, OutcomeCodeV1::Budget, &detail).await;
    }
    // Reserved now, or before a crash that came before any ownership (and so
    // before any dispatch): record ownership, then dispatch.
    let now = ids::now_ms();
    let owned = state::update_cycle(&deps.iii, cycle_id, |stored| {
        if stored.step != step || stored.status.is_terminal() {
            return Ok((None, false));
        }
        let mut next = stored.clone();
        next.executions.push(CycleExecutionV1 {
            role,
            state: ExecutionStateV1::Dispatching,
            execution_id: None,
            label: Some(label.clone()),
            work_id: Some(work_id.clone()),
            dispatched_at: Some(now),
            admission_key: None,
            detail: None,
        });
        next.updated_at = now;
        Ok((Some(next), true))
    })
    .await?
    .unwrap_or(false);
    if !owned {
        return Ok(None);
    }
    let started: Result<Value, EvalError> = call(
        deps,
        "e2e::dashboard::execution-start",
        json!({ "label": label, "idempotency_key": work_id, "parameters": parameters }),
        START_TIMEOUT_MS,
    )
    .await;
    match started {
        Ok(response) => match response["execution_id"].as_str() {
            Some(execution_id) => {
                wait(
                    format!("{name} {execution_id} dispatched"),
                    Some(execution_id.into()),
                )
                .await
            }
            None => {
                wait(
                    format!("execution-start answered without an id; reconciling {label}"),
                    None,
                )
                .await
            }
        },
        Err(EvalError::Unanswered(error)) => {
            wait(
                format!("execution-start unanswered ({error}); reconciling {label}"),
                None,
            )
            .await
        }
        Err(error) if runner_busy(&error) => {
            // Nothing started: the ownership entry goes, the reservation
            // stays, and the next poll dispatches the same work id again.
            let detail = format!("the E2E runner is busy; {label} waits for it");
            transition(
                deps,
                cycle_id,
                step,
                CycleStatusV1::Waiting,
                stage,
                &detail,
                |next| {
                    next.executions
                        .retain(|entry| entry.work_id.as_deref() != Some(work_id.as_str()));
                    next.next = Some(awaiting(&detail));
                    Ok(())
                },
            )
            .await
        }
        Err(error) => {
            // An explicit refusal started nothing and cost nothing.
            settle(deps, cycle_id, &work_id, Some(0.0)).await?;
            let reason = format!("the E2E refused the {name} execution: {error}");
            transition(
                deps,
                cycle_id,
                step,
                CycleStatusV1::Failed,
                CycleStageV1::Done,
                &reason,
                |next| {
                    update_entry(next, &label, |entry| {
                        entry.state = ExecutionStateV1::Lost;
                        entry.detail = Some(reason.clone());
                    });
                    next.failure = Some(reason.clone());
                    Ok(())
                },
            )
            .await
        }
    }
}
pub(crate) async fn settle(
    deps: &Deps,
    cycle_id: &str,
    work_id: &str,
    cost_usd: Option<f64>,
) -> Result<(), EvalError> {
    let now = ids::now_ms();
    state::update_ledger(&deps.iii, cycle_id, |ledger| {
        let Some(mut ledger) = ledger else {
            return Ok((None, ()));
        };
        if !ledger.reservations.contains_key(work_id) {
            return Ok((None, ()));
        }
        let settled = cost::settle(&mut ledger, work_id, cost_usd, now)?;
        Ok((settled.then_some(ledger), ()))
    })
    .await
}

// ---------------------------------------------------------------------------
// Import and analysis
// ---------------------------------------------------------------------------

pub(crate) async fn execution_get(deps: &Deps, execution_id: &str) -> Result<Value, EvalError> {
    call(
        deps,
        "e2e::dashboard::execution-get",
        json!({ "execution_id": execution_id }),
        BUS_TIMEOUT_MS,
    )
    .await
}

/// One claim per execution/run/attempt, from the execution's summary.
fn admission_key(summary: &Value, execution_id: &str) -> String {
    let run_id = evidence::text_at(summary, "/run_id");
    let attempt_id = evidence::text_at(summary, "/attempt");
    ids::sha256_json(&(execution_id, &run_id, &attempt_id))
}

pub(crate) fn claim_of(
    bundle: &Value,
    execution_id: &str,
    cycle_id: &str,
    role: ExecutionRoleV1,
) -> AdmissionV1 {
    let summary = evidence::summary(bundle, execution_id)
        .cloned()
        .unwrap_or(Value::Null);
    let run_id = evidence::text_at(&summary, "/run_id");
    let attempt_id = evidence::text_at(&summary, "/attempt");
    AdmissionV1 {
        admission_key: admission_key(&summary, execution_id),
        execution_id: execution_id.into(),
        run_id,
        attempt_id,
        cycle_id: cycle_id.into(),
        role,
        claimed_at: ids::now_ms(),
    }
}

/// Claims the execution/run/attempt once and copies its complete bundle
/// into the loop (the first copy is kept). Returns the stored claim (maybe
/// another cycle's), whether this call claimed it, and the stored copy.
pub(crate) async fn import(
    deps: &Deps,
    bundle: &Value,
    claim: &AdmissionV1,
) -> Result<(AdmissionV1, bool, ExecutionEvidenceV1), EvalError> {
    let (stored, claimed) = state::claim_admission(&deps.iii, claim).await?;
    let copy = ExecutionEvidenceV1 {
        admission_key: stored.admission_key.clone(),
        execution_id: stored.execution_id.clone(),
        imported_at: ids::now_ms(),
        bundle_digest: ids::sha256_json(bundle),
        bundle: bundle.clone(),
        identity: evidence::identity(bundle, &stored.execution_id),
    };
    state::put_evidence_once(&deps.iii, &copy).await?;
    let evidence = state::get_evidence(&deps.iii, &stored.admission_key)
        .await?
        .ok_or_else(|| {
            EvalError::State(format!("evidence {} was not stored", stored.admission_key))
        })?;
    Ok((stored, claimed, evidence))
}

/// Forwards the runs' recorded failures to the central Sentinel once
/// (its dedupe key makes a re-import a no-op) and keeps what it answered.
async fn forward_to_sentinel(
    deps: &Deps,
    evidence: &ExecutionEvidenceV1,
    role: ExecutionRoleV1,
) -> Option<SentinelForwardV1> {
    let side = match role {
        ExecutionRoleV1::Candidate => "candidate",
        ExecutionRoleV1::Control => "control",
        ExecutionRoleV1::Source | ExecutionRoleV1::Baseline => "baseline",
    };
    let identity = &evidence.identity;
    let revision = identity
        .workers_revision
        .clone()
        .or_else(|| identity.harness_version.clone())
        .unwrap_or_else(|| "unknown".into());
    let at_ms = evidence::summary(&evidence.bundle, &evidence.execution_id)
        .and_then(|summary| evidence::text_at(summary, "/completed_at"))
        .and_then(|at| ids::parse_ms(&at))
        .unwrap_or(evidence.imported_at);
    let observations = evidence::sentinel_observations(evidence, side, &revision, at_ms);
    if observations.is_empty() {
        return None;
    }
    let mut forward = SentinelForwardV1 {
        observations: observations.len() as u32,
        recorded: None,
        deduped: None,
        skipped: None,
        results: Vec::new(),
        error: None,
    };
    for batch in observations.chunks(SENTINEL_BATCH) {
        match call::<_, Value>(
            deps,
            SENTINEL_INGEST,
            json!({ "report_id": evidence.admission_key, "namespace": "e2e", "observations": batch }),
            BUS_TIMEOUT_MS,
        )
        .await
        {
            Ok(response) => {
                let add = |total: Option<u64>, key: &str| {
                    response[key].as_u64().map(|count| total.unwrap_or(0) + count)
                };
                forward.recorded = add(forward.recorded, "recorded");
                forward.deduped = add(forward.deduped, "deduped");
                forward.skipped = add(forward.skipped, "skipped");
                forward
                    .results
                    .extend(response["results"].as_array().cloned().unwrap_or_default());
            }
            Err(error) => {
                forward.error.get_or_insert(error.to_string());
            }
        }
    }
    Some(forward)
}

pub(crate) fn analysis_of(
    evidence: &ExecutionEvidenceV1,
    role: ExecutionRoleV1,
    sentinel: Option<SentinelForwardV1>,
) -> CycleAnalysisV1 {
    let (findings, unknowns) = evidence::first_pass(evidence);
    CycleAnalysisV1 {
        execution_id: evidence.execution_id.clone(),
        admission_key: evidence.admission_key.clone(),
        role,
        identity: evidence.identity.clone(),
        findings,
        unknowns,
        child_transcripts_complete: false,
        sentinel,
        scenarios: evidence::scenario_context(&evidence.bundle),
        analyzed_at: ids::now_ms(),
    }
}

pub(crate) fn pertinent(analysis: &CycleAnalysisV1) -> Vec<String> {
    analysis
        .findings
        .iter()
        .filter(|finding| finding.class.is_pertinent())
        .map(|finding| finding.fingerprint.clone())
        .collect()
}

pub(crate) fn class_counts(analysis: &CycleAnalysisV1) -> String {
    let mut counts = std::collections::BTreeMap::new();
    for finding in &analysis.findings {
        *counts.entry(format!("{:?}", finding.class)).or_insert(0u32) += 1;
    }
    counts
        .into_iter()
        .map(|(class, count)| format!("{count} {class}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The cheap first pass over one imported execution of the cycle, then its
/// durable next stage: candidate work only on pertinent evidence (or a
/// usable baseline), a decision once a candidate returns, otherwise an
/// explicit outcome. No model is called and nothing is dispatched.
async fn analysis_stage(
    deps: &Deps,
    record: &CycleRecordV1,
) -> Result<Option<CycleRecordV1>, EvalError> {
    let analyzed = |key: &str| {
        record
            .analyses
            .iter()
            .any(|analysis| analysis.admission_key == key)
    };
    let pending = record.executions.iter().find(|entry| {
        entry.state == ExecutionStateV1::Imported
            && entry
                .admission_key
                .as_deref()
                .is_some_and(|key| !analyzed(key))
    });
    let (entry, evidence) = match pending {
        Some(entry) => {
            let key = entry.admission_key.clone().unwrap_or_default();
            let evidence = state::get_evidence(&deps.iii, &key)
                .await?
                .ok_or_else(|| EvalError::State(format!("evidence {key} is missing")))?;
            (entry.clone(), evidence)
        }
        None => {
            let Some(entry) = record.executions.iter().find(|entry| {
                entry.role == ExecutionRoleV1::Source && entry.state == ExecutionStateV1::Running
            }) else {
                // Woken with nothing new while its own work still runs: wait for it.
                if record.executions.iter().any(|entry| {
                    matches!(
                        entry.state,
                        ExecutionStateV1::Running | ExecutionStateV1::Dispatching
                    )
                }) {
                    let detail = "nothing new to analyze; waiting for the cycle's executions";
                    return transition(
                        deps,
                        &record.cycle_id,
                        record.step,
                        CycleStatusV1::Waiting,
                        CycleStageV1::AwaitingExecution,
                        detail,
                        |next| {
                            next.next = Some(awaiting(detail));
                            Ok(())
                        },
                    )
                    .await;
                }
                return fail(deps, record, "no imported execution is left to analyze").await;
            };
            let execution_id = entry.execution_id.clone().unwrap_or_default();
            let bundle = execution_get(deps, &execution_id).await?;
            match evidence::readiness(&bundle, &execution_id) {
                Readiness::Open(why) => {
                    let detail = format!("execution {execution_id} is not complete yet: {why}");
                    return transition(
                        deps,
                        &record.cycle_id,
                        record.step,
                        CycleStatusV1::Waiting,
                        CycleStageV1::AwaitingExecution,
                        &detail,
                        |next| {
                            next.next = Some(awaiting(&detail));
                            Ok(())
                        },
                    )
                    .await;
                }
                Readiness::Abnormal(why) => {
                    return end(
                        deps,
                        record,
                        OutcomeCodeV1::Inconclusive,
                        &format!("execution {execution_id}: {why}"),
                    )
                    .await;
                }
                Readiness::Complete => {}
            }
            // The same ownership as an admitted event: a plan's native child
            // or a cycle's own dispatch never opens another cycle.
            let summary = evidence::summary(&bundle, &execution_id);
            let at =
                |pointer: &str| summary.and_then(|summary| evidence::text_at(summary, pointer));
            let owner = at("/parent_plan_execution_id")
                .map(|parent| format!("native child execution belongs to plan {parent}"))
                .or_else(|| {
                    at("/idempotency_key")
                        .and_then(|key| key.split_once(':').map(|(owner, _)| owner.to_string()))
                        .filter(|owner| ids::is_cycle(owner))
                        .map(|owner| format!("execution {execution_id} is work of cycle {owner}"))
                });
            if let Some(owner) = owner {
                return end(deps, record, OutcomeCodeV1::Inconclusive, &owner).await;
            }
            let claim = claim_of(
                &bundle,
                &execution_id,
                &record.cycle_id,
                ExecutionRoleV1::Source,
            );
            let (stored, _, evidence) = import(deps, &bundle, &claim).await?;
            if stored.cycle_id != record.cycle_id {
                let detail = format!(
                    "execution {execution_id} is already admitted by cycle {}",
                    stored.cycle_id
                );
                return end(deps, record, OutcomeCodeV1::Inconclusive, &detail).await;
            }
            let mut entry = entry.clone();
            entry.state = ExecutionStateV1::Imported;
            entry.admission_key = Some(evidence.admission_key.clone());
            (entry, evidence)
        }
    };
    let sentinel = forward_to_sentinel(deps, &evidence, entry.role).await;
    let analysis = analysis_of(&evidence, entry.role, sentinel);
    if let Some(work_id) = &entry.work_id {
        settle(
            deps,
            &record.cycle_id,
            work_id,
            evidence::reported_cost(&evidence.bundle),
        )
        .await?;
    }
    let findings = pertinent(&analysis);
    let counts = class_counts(&analysis);
    let config = record.config.as_ref().map(|stored| &stored.config);
    let candidate_config = config.and_then(|config| config.candidate.as_ref());
    let now = ids::now_ms();
    // What comes next: (status, stage, detail), an outcome, and what the
    // final write records.
    let mut outcome: Option<(OutcomeCodeV1, String)> = None;
    let mut next_stage: Option<(CycleStageV1, String)> = None;
    let mut baseline = None;
    let mut judged: Option<(
        DecisionSideV1,
        DecisionSideV1,
        Vec<EvidenceV1>,
        Option<CycleLedgerV1>,
    )> = None;
    match entry.role {
        ExecutionRoleV1::Source if findings.is_empty() => {
            outcome = Some((
                OutcomeCodeV1::NoAction,
                format!("no pertinent finding: {counts}"),
            ));
        }
        ExecutionRoleV1::Source if candidate_config.is_none() => {
            outcome = Some((
                OutcomeCodeV1::Inconclusive,
                format!(
                    "{} pertinent finding(s) ({counts}); candidate work is not configured (config.candidate)",
                    findings.len()
                ),
            ));
        }
        ExecutionRoleV1::Source => {
            // Coverage first (cheap): an execution whose findings map to no
            // current scenario criterion never reaches a paid judgment.
            next_stage = Some((
                CycleStageV1::Coverage,
                format!("{} pertinent finding(s) ({counts})", findings.len()),
            ));
        }
        ExecutionRoleV1::Baseline => {
            let plan = record
                .plan
                .as_ref()
                .ok_or_else(|| EvalError::State("a baseline without a plan".into()))?;
            let measured = decision::baseline(plan, &evidence.execution_id, &evidence.bundle);
            if !measured.usable {
                // The E2E's own failure says why no run was measured.
                let failure = crate::evidence::summary(&evidence.bundle, &evidence.execution_id)
                    .and_then(|summary| crate::evidence::text_at(summary, "/first_failure/message"))
                    .map(|message| format!("; the execution failed: {message}"))
                    .unwrap_or_default();
                outcome = Some((
                    OutcomeCodeV1::Inconclusive,
                    format!("baseline unusable: {}{failure}", measured.reason),
                ));
            } else if reads_transcripts(plan) {
                // A Harness metric is read in the transcripts first, always:
                // what the runs did is the result even when no change could
                // be proven or tried (the insight checks both after).
                next_stage = Some((
                    CycleStageV1::Insight,
                    format!("baseline measured; {} ({counts})", measured.reason),
                ));
            } else if candidate_config.is_none() {
                outcome = Some((
                    OutcomeCodeV1::Inconclusive,
                    format!(
                        "baseline measured ({}); candidate work is not configured (config.candidate)",
                        measured.reason
                    ),
                ));
            } else if let Some(reason) = decision::no_room(
                &plan.criterion,
                plan.sampling.runs_per_side,
                &measured.primary,
            ) {
                outcome = Some((
                    OutcomeCodeV1::NoAction,
                    format!("baseline measured; {reason}"),
                ));
            } else {
                next_stage = Some((
                    CycleStageV1::Proposal,
                    format!("baseline measured; {} ({counts})", measured.reason),
                ));
            }
            baseline = Some(measured);
        }
        ExecutionRoleV1::Candidate | ExecutionRoleV1::Control => 'proof: {
            let plan = record
                .plan
                .clone()
                .ok_or_else(|| EvalError::State("a comparison without a plan".into()))?;
            // Every round of both sides, in round order; this import is not
            // in `record` yet.
            let batched = config
                .and_then(|config| config.validation.as_ref())
                .is_some_and(|validation| validation.where_ == WhereV1::Github);
            let schedule = crate::stages::proof_schedule(plan.sampling.runs_per_side, batched);
            let this = entry.work_id.clone();
            let mut rounds: Vec<(ExecutionRoleV1, Option<ExecutionEvidenceV1>)> = Vec::new();
            for (role, name) in &schedule {
                let work_id = format!("{}:{name}", record.cycle_id);
                let found = if this.as_deref() == Some(work_id.as_str()) {
                    Some(evidence.clone())
                } else {
                    match record.executions.iter().find(|stored| {
                        stored.work_id.as_deref() == Some(work_id.as_str())
                            && stored.state == ExecutionStateV1::Imported
                    }) {
                        Some(stored) => {
                            let key = stored.admission_key.clone().ok_or_else(|| {
                                EvalError::State(format!("proof round {name} has no evidence key"))
                            })?;
                            Some(state::get_evidence(&deps.iii, &key).await?.ok_or_else(|| {
                                EvalError::State(format!("the evidence of {name} is missing"))
                            })?)
                        }
                        None => None,
                    }
                };
                rounds.push((*role, found));
            }
            let imported = rounds.iter().filter(|(_, found)| found.is_some()).count();
            if imported < rounds.len() {
                next_stage = Some((
                    CycleStageV1::Validation,
                    format!(
                        "{} imported; {imported} of {} proof executions in",
                        entry.label.as_deref().unwrap_or("a proof round"),
                        rounds.len()
                    ),
                ));
                break 'proof;
            }
            let of = |wanted: ExecutionRoleV1| -> Vec<ExecutionEvidenceV1> {
                rounds
                    .iter()
                    .filter(|(role, _)| *role == wanted)
                    .filter_map(|(_, found)| found.clone())
                    .collect()
            };
            let (controls, candidates) =
                (of(ExecutionRoleV1::Control), of(ExecutionRoleV1::Candidate));
            let validation = config.and_then(|config| config.validation.clone());
            let side = |evidence: &ExecutionEvidenceV1, expected_revision: Option<String>| {
                // The recorded stack must be the frozen template with only
                // the commit put in.
                let (expected_stack, observed_stack) = match (&validation, &expected_revision) {
                    (Some(validation), Some(commit)) => {
                        stack_proof(evidence, &validation.source_stack(commit).yaml)
                    }
                    _ => (None, None),
                };
                DecisionSideV1 {
                    execution_id: evidence.execution_id.clone(),
                    identity: evidence.identity.clone(),
                    seeds: evidence::seeds(&evidence.bundle),
                    policy_revision: Some(plan.policy_revision.clone()),
                    expected_revision,
                    expected_stack,
                    observed_stack,
                }
            };
            // With a pinned stack the control must have run the base commit.
            let base_revision = config
                .and_then(|config| config.validation.as_ref().zip(config.candidate.as_ref()))
                .map(|(_, candidate)| candidate.base_sha.clone());
            let candidate_revision = crate::stages::candidate_sha(record).ok_or_else(|| {
                EvalError::State("a candidate execution without a candidate commit".into())
            })?;
            let merged = |runs: &[ExecutionEvidenceV1], revision: Option<String>| {
                let sides: Vec<DecisionSideV1> = runs
                    .iter()
                    .map(|evidence| side(evidence, revision.clone()))
                    .collect();
                decision::merge_sides(&sides)
                    .ok_or_else(|| EvalError::State("a proof side has no execution".into()))
            };
            let other = merged(&candidates, Some(candidate_revision))?;
            let bundles = |runs: &[ExecutionEvidenceV1]| -> Vec<(String, Value)> {
                runs.iter()
                    .map(|evidence| (evidence.execution_id.clone(), evidence.bundle.clone()))
                    .collect()
            };
            let pairs =
                decision::pair_evidence(&plan, &bundles(&controls), &bundles(&candidates), now);
            let base_side = merged(&controls, base_revision)?;
            let ledger = state::get_ledger(&deps.iii, &record.cycle_id).await?;
            judged = Some((base_side, other, pairs, ledger));
        }
    }
    transition_with(deps, record, |next| {
        if let Some(stored) = next.executions.iter_mut().find(|stored| {
            stored.label == entry.label
                && stored.execution_id == entry.execution_id
                && stored.role == entry.role
        }) {
            *stored = entry.clone();
        }
        next.analyses.push(analysis.clone());
        if let Some(measured) = &baseline {
            next.baseline = Some(measured.clone());
        }
        let mut outcome = outcome.clone();
        let mut next_stage = next_stage.clone();
        if let Some((base, other, pairs, ledger)) = &judged {
            let decision = decision::decide_cycle(next, ledger.as_ref(), base, other, pairs, now)?;
            let detail = format!("source {:?}: {}", decision.outcome, decision.reason);
            if decision.outcome == DecisionOutcomeV1::Accepted {
                next_stage = Some((CycleStageV1::PullRequest, detail));
            } else {
                outcome = Some((OutcomeCodeV1::Decided, detail));
            }
        }
        Ok(match (outcome, next_stage) {
            (Some((code, detail)), _) => {
                next.outcome = Some(CycleOutcomeV1 {
                    code,
                    detail: detail.clone(),
                });
                (
                    CycleStatusV1::Completed,
                    CycleStageV1::Done,
                    format!("{code:?}: {detail}"),
                )
            }
            (None, Some((stage, detail))) => {
                next.next = Some(CycleNextV1 {
                    action: NextActionV1::CandidateWork,
                    detail: detail.clone(),
                    findings: findings.clone(),
                });
                (CycleStatusV1::Running, stage, detail)
            }
            (None, None) => (
                CycleStatusV1::Failed,
                CycleStageV1::Done,
                "analysis reached no next stage".to_string(),
            ),
        })
    })
    .await
}

/// (sha256 of the frozen template, sha256 of the recorded stack YAML with
/// only `pin` replaced by `placeholder`); unknown when nothing was recorded.
/// The pins of the frozen template with its commit put in (`expected`) and
/// the pins the execution recorded for the same containers, as digests of
/// `{container: "commit:<sha>" | "version:<v>"}`. Pins, not YAML: the E2E
/// replaces the YAML it was sent by the one it assembled (path:// builds,
/// iii and runner pinned), so the YAML never matches; its summary's `stack`
/// keeps each container's commit or requested version.
fn stack_proof(evidence: &ExecutionEvidenceV1, expected: &str) -> (Option<String>, Option<String>) {
    let digest = |pins: &BTreeMap<String, String>| {
        ids::sha256_text(&serde_json::to_string(pins).unwrap_or_default())
    };
    let pinned = |commit: Option<&str>, version: Option<String>| {
        commit
            .map(|commit| format!("commit:{commit}"))
            .or(version.map(|version| format!("version:{version}")))
    };
    let Some(expected) = serde_yaml::from_str::<serde_yaml::Value>(expected)
        .ok()
        .and_then(|stack| {
            stack["containers"].as_mapping().map(|containers| {
                containers
                    .iter()
                    .map(|(name, container)| {
                        let version = match &container["version"] {
                            serde_yaml::Value::String(version) => Some(version.clone()),
                            serde_yaml::Value::Number(version) => Some(version.to_string()),
                            _ => None,
                        };
                        Some((
                            name.as_str()?.to_string(),
                            pinned(container["commit"].as_str(), version)?,
                        ))
                    })
                    .collect::<Option<BTreeMap<_, _>>>()
            })
        })
        .flatten()
    else {
        return (Some(ids::sha256_text(expected.trim())), None);
    };
    let observed = evidence::summary(&evidence.bundle, &evidence.execution_id)
        .and_then(|summary| summary["stack"].as_array())
        .and_then(|recorded| {
            expected
                .keys()
                .map(|name| {
                    let entry = recorded
                        .iter()
                        .find(|entry| entry["name"] == name.as_str())?;
                    Some((
                        name.clone(),
                        pinned(
                            entry["commit"].as_str(),
                            entry["requested"].as_str().map(str::to_string),
                        )?,
                    ))
                })
                .collect::<Option<BTreeMap<_, _>>>()
        });
    (Some(digest(&expected)), observed.as_ref().map(digest))
}

/// [`transition`] whose target status, stage and detail are decided inside
/// the write, from the stored record (e.g. by a decision recorded there).
pub(crate) async fn transition_with(
    deps: &Deps,
    record: &CycleRecordV1,
    mut mutate: impl FnMut(
        &mut CycleRecordV1,
    ) -> Result<(CycleStatusV1, CycleStageV1, String), EvalError>,
) -> Result<Option<CycleRecordV1>, EvalError> {
    let now = ids::now_ms();
    let step = record.step;
    let written = state::update_cycle(&deps.iii, &record.cycle_id, |stored| {
        if stored.step != step || stored.status.is_terminal() {
            return Ok((None, None));
        }
        let mut next = stored.clone();
        let (status, stage, detail) = mutate(&mut next)?;
        apply(&mut next, now, status, stage, &detail);
        Ok((Some(next.clone()), Some(next)))
    })
    .await?
    .flatten();
    if let Some(record) = &written {
        if record.status == CycleStatusV1::Running {
            queue::enqueue_step(&deps.iii, &record.cycle_id, record.step).await?;
        }
    }
    Ok(written)
}

// ---------------------------------------------------------------------------
// Execution input: `e2e::dashboard::changed`, re-read through execution-get
// ---------------------------------------------------------------------------

fn not_admitted(
    reason: impl Into<String>,
    cycle_id: Option<String>,
) -> ExecutionAdmissionResponseV1 {
    ExecutionAdmissionResponseV1 {
        admitted: false,
        cycle_id,
        reason: reason.into(),
    }
}

pub async fn execution_changed(
    deps: &Deps,
    event: ExecutionChangedEventV1,
) -> Result<ExecutionAdmissionResponseV1, EvalError> {
    match event
        .execution_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        Some(execution_id) => Ok(Box::pin(admit_execution(deps, execution_id)).await?.0),
        None => {
            let admitted = admission_sweep(deps).await?;
            Ok(not_admitted(
                format!("no execution_id: the sweep admitted {admitted}"),
                None,
            ))
        }
    }
}

/// Moves a waiting cycle back to analysis once its execution is imported.
async fn wake(
    deps: &Deps,
    cycle_id: &str,
    mark: Option<(String, AdmissionV1)>,
    detail: &str,
) -> Result<bool, EvalError> {
    let Some(record) = state::get_cycle(&deps.iii, cycle_id).await? else {
        return Ok(false);
    };
    if record.status != CycleStatusV1::Waiting
        || (record.stage != CycleStageV1::AwaitingExecution && mark.is_none())
    {
        return Ok(false);
    }
    let written = transition(
        deps,
        cycle_id,
        record.step,
        CycleStatusV1::Running,
        CycleStageV1::Analysis,
        detail,
        |next| {
            if let Some((work_id, claim)) = &mark {
                update_work_entry(next, work_id, |entry| {
                    entry.execution_id = Some(claim.execution_id.clone());
                    entry.state = ExecutionStateV1::Imported;
                    entry.admission_key = Some(claim.admission_key.clone());
                });
            }
            next.next = None;
            Ok(())
        },
    )
    .await?;
    Ok(written.is_some())
}

/// Re-reads `execution-get` (the source of truth) and admits a complete
/// execution once per execution/run/attempt. An execution a cycle dispatched
/// (its immutable idempotency key names work recorded before the dispatch)
/// returns to that cycle and never starts another; any other
/// normal terminal execution starts one cycle, or wakes the cycle already
/// started from it. The flag tells the sweep to hold its checkpoint.
pub async fn admit_execution(
    deps: &Deps,
    execution_id: &str,
) -> Result<(ExecutionAdmissionResponseV1, bool), EvalError> {
    // With admission off only the loop's own executions are read: a foreign
    // one (an imported CI campaign can be tens of MB) is never fetched.
    if state::get_config(&deps.iii)
        .await?
        .is_some_and(|stored| !stored.config.admit_executions)
        && !state::list_open_cycles(&deps.iii)
            .await?
            .iter()
            .any(|cycle| {
                cycle
                    .executions
                    .iter()
                    .any(|entry| entry.execution_id.as_deref() == Some(execution_id))
            })
    {
        return Ok((
            not_admitted(
                "automatic admission is off (config.admit_executions): a cycle starts only when asked",
                None,
            ),
            false,
        ));
    }
    let bundle = execution_get(deps, execution_id).await?;
    let Some(summary) = evidence::summary(&bundle, execution_id) else {
        return Ok((
            not_admitted("execution-get does not list the execution", None),
            false,
        ));
    };
    let label = evidence::text_at(summary, "/label").unwrap_or_default();
    let idempotency_key = evidence::text_at(summary, "/idempotency_key");
    let readiness = evidence::readiness(&bundle, execution_id);
    if let Some(parent) = evidence::text_at(summary, "/parent_plan_execution_id") {
        return Ok((
            not_admitted(
                format!("native child execution belongs to plan {parent}"),
                None,
            ),
            false,
        ));
    }
    if let Some((owner, _)) = idempotency_key
        .as_deref()
        .and_then(|key| key.split_once(':'))
    {
        if ids::is_cycle(owner) {
            return admit_owned(
                deps,
                owner,
                idempotency_key.as_deref().unwrap_or_default(),
                &label,
                execution_id,
                &bundle,
                readiness,
            )
            .await;
        }
    }
    if let Some(owner) = ids::label_cycle(&label) {
        return Ok((
            not_admitted(
                "a display label cannot establish ownership without the recorded idempotency key",
                Some(owner.to_string()),
            ),
            false,
        ));
    }
    match readiness {
        Readiness::Open(why) => return Ok((not_admitted(why, None), true)),
        Readiness::Abnormal(why) => return Ok((not_admitted(why, None), false)),
        Readiness::Complete => {}
    }
    // Only an open cycle is still waiting for it; a closed one that imported
    // it holds the claim, which refuses another cycle below.
    let started_from = state::list_open_cycles(&deps.iii)
        .await?
        .into_iter()
        .find(|cycle| {
            cycle.executions.iter().any(|entry| {
                entry.role == ExecutionRoleV1::Source
                    && entry.execution_id.as_deref() == Some(execution_id)
            })
        });
    if let Some(cycle) = started_from {
        // Only a source still running wakes its cycle: one already imported
        // was analyzed, and its repeated completion is not new work.
        let running = cycle.executions.iter().any(|entry| {
            entry.role == ExecutionRoleV1::Source
                && entry.execution_id.as_deref() == Some(execution_id)
                && entry.state == ExecutionStateV1::Running
        });
        let woken = running
            && cycle.stage == CycleStageV1::AwaitingExecution
            && wake(
                deps,
                &cycle.cycle_id,
                None,
                &format!("execution {execution_id} completed"),
            )
            .await?;
        return Ok((
            not_admitted(
                if woken {
                    "returned to the cycle started from it"
                } else {
                    "already in a cycle"
                },
                Some(cycle.cycle_id),
            ),
            false,
        ));
    }
    let claim = claim_of(
        &bundle,
        execution_id,
        &ids::cycle_id(),
        ExecutionRoleV1::Source,
    );
    let (stored, claimed, _) = import(deps, &bundle, &claim).await?;
    if state::get_cycle(&deps.iii, &stored.cycle_id)
        .await?
        .is_none()
    {
        let config = state::get_config(&deps.iii).await?;
        let record = new_cycle(
            stored.cycle_id.clone(),
            CycleInputV1::ExecutionId(execution_id.into()),
            vec![source(
                execution_id,
                ExecutionStateV1::Imported,
                Some(stored.admission_key.clone()),
            )],
            config,
            "admitted from a terminal E2E execution",
        );
        if state::create_cycle(&deps.iii, &record).await? {
            queue::enqueue_step(&deps.iii, &record.cycle_id, record.step).await?;
        }
    }
    Ok((
        ExecutionAdmissionResponseV1 {
            admitted: claimed,
            reason: if claimed {
                "admitted".into()
            } else {
                "already admitted".into()
            },
            cycle_id: Some(stored.cycle_id),
        },
        false,
    ))
}

async fn admit_owned(
    deps: &Deps,
    owner: &str,
    work_id: &str,
    label: &str,
    execution_id: &str,
    bundle: &Value,
    readiness: Readiness,
) -> Result<(ExecutionAdmissionResponseV1, bool), EvalError> {
    let owner_id = Some(owner.to_string());
    let Some(record) = state::get_cycle(&deps.iii, owner).await? else {
        return Ok((
            not_admitted(
                format!("its idempotency key names unknown cycle {owner}"),
                None,
            ),
            false,
        ));
    };
    let Some(entry) = record
        .executions
        .iter()
        .find(|entry| entry.work_id.as_deref() == Some(work_id))
        .cloned()
    else {
        return Ok((
            not_admitted(
                format!("cycle {owner} never recorded idempotency key {work_id}"),
                owner_id,
            ),
            false,
        ));
    };
    if let Some(expected) = entry.execution_id.as_deref() {
        if expected != execution_id {
            return Ok((
                not_admitted(
                    format!("{label} belongs to execution {expected}, not {execution_id}"),
                    owner_id,
                ),
                false,
            ));
        }
    } else if reconcile_dispatch(deps, work_id).await?.as_deref() != Some(execution_id) {
        return Ok((
            not_admitted(
                format!("{work_id} does not identify only execution {execution_id}"),
                owner_id,
            ),
            true,
        ));
    }
    if entry.state == ExecutionStateV1::Imported {
        return Ok((
            not_admitted("already imported into its cycle", owner_id),
            false,
        ));
    }
    match readiness {
        Readiness::Open(why) => {
            if entry.execution_id.is_none() {
                state::update_cycle(&deps.iii, owner, |stored| {
                    let mut next = stored.clone();
                    update_work_entry(&mut next, work_id, running(execution_id.into()));
                    Ok((Some(next), ()))
                })
                .await?;
            }
            Ok((not_admitted(why, owner_id), true))
        }
        Readiness::Abnormal(why) => {
            if record.status == CycleStatusV1::Waiting {
                let detail = format!("{label} ({execution_id}) ended abnormally: {why}");
                transition(
                    deps,
                    owner,
                    record.step,
                    CycleStatusV1::Completed,
                    CycleStageV1::Done,
                    &detail,
                    |next| {
                        update_work_entry(next, work_id, |entry| {
                            entry.execution_id = Some(execution_id.into());
                            entry.state = ExecutionStateV1::Lost;
                        });
                        next.outcome = Some(CycleOutcomeV1 {
                            code: OutcomeCodeV1::Inconclusive,
                            detail: detail.clone(),
                        });
                        Ok(())
                    },
                )
                .await?;
            }
            Ok((not_admitted(why, owner_id), false))
        }
        Readiness::Complete => {
            let claim = claim_of(bundle, execution_id, owner, entry.role);
            let (stored, claimed, _) = import(deps, bundle, &claim).await?;
            let woken = wake(
                deps,
                owner,
                Some((work_id.to_string(), stored)),
                &format!("{label} ({execution_id}) completed and was imported"),
            )
            .await?;
            Ok((
                ExecutionAdmissionResponseV1 {
                    admitted: claimed,
                    cycle_id: owner_id,
                    reason: if woken {
                        "returned to its cycle".into()
                    } else {
                        "imported; its cycle is not waiting".into()
                    },
                },
                false,
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Recovery
// ---------------------------------------------------------------------------

/// Recovers `e2e::dashboard::changed` events that never arrived. The first
/// sweep only sets the checkpoint (history is analyzed through
/// `eval::start`); later ones admit every execution started after it. The
/// checkpoint never passes an execution still open, one whose
/// `execution-get` is not complete yet, or one whose admission failed.
async fn admission_sweep(deps: &Deps) -> Result<u64, EvalError> {
    let now = ids::now_ms();
    let Some(checkpoint) = state::get_admission_checkpoint(&deps.iii).await? else {
        state::put_admission_checkpoint(
            &deps.iii,
            &AdmissionCheckpointV1 {
                since: now,
                updated_at: now,
            },
        )
        .await?;
        return Ok(0);
    };
    let since = checkpoint.since;
    let (mut admitted, mut newest, mut hold) = (0u64, since, None::<i64>);
    let mut cursor = None::<String>;
    for _ in 0..MAX_PAGES {
        let listed: Value = match call(
            deps,
            "e2e::dashboard::executions-list",
            json!({ "limit": EXECUTIONS_PAGE, "cursor": cursor }),
            BUS_TIMEOUT_MS,
        )
        .await
        {
            Ok(listed) => listed,
            Err(error) => {
                tracing::debug!(%error, "admission sweep: E2E executions unavailable");
                return Ok(admitted);
            }
        };
        for entry in listed["executions"].as_array().into_iter().flatten() {
            let Some(execution_id) = evidence::text_at(entry, "/id") else {
                continue;
            };
            let started = evidence::text_at(entry, "/started_at").and_then(|at| ids::parse_ms(&at));
            if started.is_some_and(|at| at <= since) {
                continue;
            }
            let at = started.unwrap_or(since);
            let status = evidence::status_of(entry);
            if evidence::OPEN_STATUSES.contains(&status.as_str()) {
                hold = Some(hold.map_or(at, |held| held.min(at)));
                continue;
            }
            if !evidence::TERMINAL_STATUSES.contains(&status.as_str()) {
                newest = newest.max(at);
                continue;
            }
            match admit_execution(deps, &execution_id).await {
                Ok((_, true)) => hold = Some(hold.map_or(at, |held| held.min(at))),
                Ok((response, false)) => {
                    admitted += u64::from(response.admitted);
                    newest = newest.max(at);
                }
                Err(error) => {
                    tracing::warn!(%execution_id, %error, "admission sweep could not admit");
                    hold = Some(hold.map_or(at, |held| held.min(at)));
                }
            }
        }
        cursor = listed["next_cursor"].as_str().map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }
    let next = hold
        .map_or(newest, |held| (held - 1).min(newest))
        .max(since);
    if next != since {
        state::put_admission_checkpoint(
            &deps.iii,
            &AdmissionCheckpointV1 {
                since: next,
                updated_at: now,
            },
        )
        .await?;
    }
    Ok(admitted)
}

/// Requeues running cycles, reconciles unanswered dispatches, wakes cycles
/// whose execution completed without an event, ends waiting cycles past a
/// budget limit, then runs the admission sweep.
pub async fn sweep(deps: &Deps) -> Result<SweepResponseV1, EvalError> {
    let mut response = SweepResponseV1::default();
    let now = ids::now_ms();
    for cycle in state::list_open_cycles(&deps.iii).await? {
        let result: Result<(), EvalError> = async {
            match cycle.status {
                CycleStatusV1::Queued | CycleStatusV1::Running => {
                    queue::enqueue_step(&deps.iii, &cycle.cycle_id, cycle.step).await?;
                    response.requeued += 1;
                }
                CycleStatusV1::Waiting => {
                    if let Some(reason) = state::get_ledger(&deps.iii, &cycle.cycle_id)
                        .await?
                        .and_then(|ledger| cost::exceeded(&ledger, now))
                    {
                        if expire(deps, &cycle.cycle_id, &reason)
                            .await?
                            .is_some_and(|ended| ended.status.is_terminal())
                        {
                            response.expired += 1;
                        }
                        return Ok(());
                    }
                    match cycle.stage {
                        stage if POLLED.contains(&stage) => {
                            queue::enqueue_step(&deps.iii, &cycle.cycle_id, cycle.step).await?;
                            response.reconciled += 1;
                        }
                        CycleStageV1::AwaitingExecution => {
                            for entry in cycle.executions.iter().filter(|entry| {
                                matches!(
                                    entry.state,
                                    ExecutionStateV1::Running | ExecutionStateV1::Dispatching
                                )
                            }) {
                                let Some(execution_id) = &entry.execution_id else {
                                    continue;
                                };
                                if entry.role != ExecutionRoleV1::Source {
                                    let (_, open) = admit_execution(deps, execution_id).await?;
                                    response.reconciled += u64::from(!open);
                                    continue;
                                }
                                let bundle = execution_get(deps, execution_id).await?;
                                if !matches!(
                                    evidence::readiness(&bundle, execution_id),
                                    Readiness::Open(_)
                                ) && wake(
                                    deps,
                                    &cycle.cycle_id,
                                    None,
                                    &format!("execution {execution_id} completed"),
                                )
                                .await?
                                {
                                    response.reconciled += 1;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            tracing::warn!(cycle_id = %cycle.cycle_id, %error, "sweep could not recover the cycle");
        }
    }
    match admission_sweep(deps).await {
        Ok(admitted) => response.admitted = admitted,
        Err(error) => tracing::warn!(%error, "admission sweep failed; the next sweep retries"),
    }
    Ok(response)
}
