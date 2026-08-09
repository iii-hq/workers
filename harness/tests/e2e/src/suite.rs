use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use harness::functions::send::{MessageInput, SendOptions, SendRequest, SendResponse, SessionInit};
use harness::types::model::Model;
use harness::types::turn::FunctionPolicy;
use serde_json::json;
use uuid::Uuid;

use crate::context::E2eContext;
use crate::judge::{self, JudgeConfig};
use crate::report::{
    CriterionReport, E2eReport, E2eRunReport, E2eScenarioReport, FailurePhase, ModelArtifact,
    RetryAttemptReport, RunStatus,
};
use crate::scenarios::common;
use crate::scenarios::{
    CriterionAward, ObjectiveEvaluation, ScenarioId, ScenarioObservation, ScenarioSpec,
};

const MAX_RUNS: u32 = 20;
const MAX_TECHNICAL_RETRIES: u8 = 3;

fn e2e_function_policy(spec: &ScenarioSpec) -> FunctionPolicy {
    FunctionPolicy {
        allow: vec!["*".into()],
        deny: spec
            .denied_functions
            .iter()
            .map(|function| (*function).to_string())
            .collect(),
        ..FunctionPolicy::default()
    }
}

#[derive(Debug, Clone)]
pub struct SubjectConfig {
    pub model: String,
    pub provider: String,
}

pub struct SuiteRunConfig {
    pub url: String,
    pub subject: SubjectConfig,
    pub judge: Option<JudgeConfig>,
    pub output: PathBuf,
    pub scenarios: Vec<ScenarioId>,
    pub runs: u32,
    pub technical_retries: u8,
    pub progress_interval: Option<Duration>,
}

pub struct SuiteRunOutcome {
    pub report: E2eReport,
    pub report_path: PathBuf,
}

pub async fn run_suite(config: SuiteRunConfig) -> Result<SuiteRunOutcome> {
    validate_config(&config)?;
    let context = E2eContext::connect(&config.url)
        .await
        .context("connect E2E runner")?;
    let subject_model = resolve_model(&context, &config.subject.model, &config.subject.provider)
        .await
        .context("resolve subject model")?;
    let judge_model = match config.judge.as_ref() {
        Some(judge) => Some(
            resolve_model(&context, &judge.model, &judge.provider)
                .await
                .context("resolve judge model")?,
        ),
        None => None,
    };
    let mut scenario_reports = Vec::with_capacity(config.scenarios.len());

    for scenario_id in &config.scenarios {
        let definition = scenario_id.spec("validation");
        let mut runs = Vec::with_capacity(config.runs as usize);
        for repetition in 0..config.runs {
            tracing::info!(
                scenario = scenario_id.as_str(),
                run = repetition + 1,
                total_runs = config.runs,
                "running E2E quality scenario"
            );
            let run = run_with_technical_retries(
                &context,
                *scenario_id,
                &config.subject,
                config.judge.as_ref(),
                config.technical_retries,
                config.progress_interval,
            )
            .await;
            let stop = run.status.is_technical_failure();
            runs.push(run);
            if stop {
                tracing::warn!(
                    scenario = scenario_id.as_str(),
                    "stopping scenario after a technical failure"
                );
                break;
            }
        }
        scenario_reports.push(E2eScenarioReport::aggregate(
            definition.id,
            definition.version,
            definition.execution,
            runs,
        ));
    }

    context.shutdown().await;
    let report = E2eReport::new(
        ModelArtifact::from(subject_model),
        judge_model.map(ModelArtifact::from),
        config
            .judge
            .as_ref()
            .map(|_| judge::JUDGE_PROTOCOL.to_string()),
        nonempty_env("HARNESS_E2E_ENGINE_REVISION"),
        scenario_reports,
    );
    let report_path = report.write_to(&config.output)?;
    Ok(SuiteRunOutcome {
        report,
        report_path,
    })
}

fn validate_config(config: &SuiteRunConfig) -> Result<()> {
    for (name, value) in [
        ("model", config.subject.model.as_str()),
        ("provider", config.subject.provider.as_str()),
    ] {
        if value.trim().is_empty() {
            bail!("{name} cannot be empty");
        }
    }
    if !(1..=MAX_RUNS).contains(&config.runs) {
        bail!("runs must be between 1 and {MAX_RUNS}");
    }
    if config.technical_retries > MAX_TECHNICAL_RETRIES {
        bail!("technical retries must be between 0 and {MAX_TECHNICAL_RETRIES}");
    }
    if config.scenarios.is_empty() {
        bail!("at least one scenario is required");
    }
    let mut needs_judge = false;
    for scenario in &config.scenarios {
        let spec = scenario.spec("validation");
        spec.validate()?;
        needs_judge |= spec.needs_judge();
    }
    if needs_judge && config.judge.is_none() {
        bail!("selected scenarios require --judge-model and --judge-provider");
    }
    if let Some(judge) = &config.judge {
        if judge.model.trim().is_empty() || judge.provider.trim().is_empty() {
            bail!("judge model and provider cannot be empty");
        }
    }
    Ok(())
}

async fn resolve_model(context: &E2eContext, model: &str, provider: &str) -> Result<Model> {
    let response = context
        .trigger_value(
            "router::models::get",
            json!({ "id": model, "provider": provider }),
        )
        .await
        .with_context(|| format!("query catalog for {provider}/{model}"))?;
    if response.is_null() {
        bail!("model {provider}/{model} is not registered in the router catalog");
    }
    let resolved: Model = serde_json::from_value(
        response
            .get("model")
            .cloned()
            .context("router::models::get response is missing model")?,
    )
    .context("decode router catalog model")?;
    if resolved.id != model || resolved.provider != provider {
        bail!(
            "catalog resolved {provider}/{model} as {}/{}; exact model identity is required",
            resolved.provider,
            resolved.id
        );
    }
    let stream_function = format!("provider::{provider}::stream");
    if !context
        .function_exists(&stream_function)
        .await
        .with_context(|| format!("check live provider function {stream_function}"))?
    {
        bail!(
            "model {provider}/{model} is present in the catalog, but provider {provider} is not \
             running; missing function {stream_function}"
        );
    }
    Ok(resolved)
}

fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

async fn run_once(
    context: &E2eContext,
    scenario_id: ScenarioId,
    subject: &SubjectConfig,
    judge_config: Option<&JudgeConfig>,
    progress_interval: Option<Duration>,
) -> E2eRunReport {
    let started = Instant::now();
    let run_id = Uuid::new_v4().simple().to_string();
    let session_id = format!("e2e_{run_id}");
    let spec = scenario_id.spec(&run_id);
    let mut report = E2eRunReport::new(run_id.clone(), session_id.clone(), spec.prompt.clone());

    if let Err(error) = execute(
        context,
        ExecutionRequest {
            subject,
            judge_config,
            run_id: &run_id,
            session_id: &session_id,
            spec: &spec,
            progress_interval,
        },
        &mut report,
    )
    .await
    {
        report.push_failure(error.status, error.phase, error.message);
    }

    if let Err(error) = context.teardown(&session_id).await {
        report.push_failure(
            RunStatus::InfrastructureError,
            FailurePhase::Cleanup,
            format!(
                "scenario '{}': harness::teardown failed: {error:#}",
                spec.id
            ),
        );
    }
    if let Some(cleanup) = spec.cleanup {
        if let Err(error) = cleanup(context, &run_id).await {
            report.push_failure(
                RunStatus::InfrastructureError,
                FailurePhase::Cleanup,
                format!("scenario '{}': scenario cleanup failed: {error:#}", spec.id),
            );
        }
    }
    report.wall_time_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    if report.failures.is_empty() {
        if report.hard_gates.iter().any(|gate| !gate.passed) {
            // Judge-backed scenarios skip the judge on a gate failure, leaving no
            // criterion awards; the run must still enter the aggregate as a score.
            report.score.get_or_insert(0);
            report.finish(RunStatus::HardGateFailed);
        } else if report.score.is_some() {
            report.finish(RunStatus::Passed);
        } else {
            report.push_failure(
                RunStatus::InfrastructureError,
                FailurePhase::Evaluate,
                format!(
                    "scenario '{}': evaluation completed without a score; expected criterion awards or a judge score",
                    spec.id
                ),
            );
        }
    }
    report.update_cost(spec.needs_judge());
    report
}

async fn run_with_technical_retries(
    context: &E2eContext,
    scenario_id: ScenarioId,
    subject: &SubjectConfig,
    judge_config: Option<&JudgeConfig>,
    technical_retries: u8,
    progress_interval: Option<Duration>,
) -> E2eRunReport {
    let mut retry_attempts = Vec::with_capacity(technical_retries as usize);
    loop {
        let mut report = run_once(
            context,
            scenario_id,
            subject,
            judge_config,
            progress_interval,
        )
        .await;
        if retry_attempts.len() < technical_retries as usize
            && is_retryable_technical_failure(&report)
        {
            let reason = report
                .failures
                .first()
                .map(|failure| failure.message.as_str())
                .unwrap_or("transient technical failure");
            tracing::warn!(
                scenario = scenario_id.as_str(),
                attempt = retry_attempts.len() + 1,
                max_retries = technical_retries,
                reason,
                "retrying E2E scenario after a transient technical failure"
            );
            retry_attempts.push(RetryAttemptReport::from(&report));
            continue;
        }
        report.attach_retry_attempts(retry_attempts);
        return report;
    }
}

struct RunFailure {
    status: RunStatus,
    phase: FailurePhase,
    message: String,
}

struct ExecutionRequest<'a> {
    subject: &'a SubjectConfig,
    judge_config: Option<&'a JudgeConfig>,
    run_id: &'a str,
    session_id: &'a str,
    spec: &'a ScenarioSpec,
    progress_interval: Option<Duration>,
}

impl RunFailure {
    fn new(status: RunStatus, phase: FailurePhase, message: impl Into<String>) -> Self {
        Self {
            status,
            phase,
            message: message.into(),
        }
    }
}

async fn execute(
    context: &E2eContext,
    request: ExecutionRequest<'_>,
    report: &mut E2eRunReport,
) -> Result<(), RunFailure> {
    let ExecutionRequest {
        subject,
        judge_config,
        run_id,
        session_id,
        spec,
        progress_interval,
    } = request;
    let stuck_timeout = Duration::from_secs(spec.execution.stuck_timeout_seconds);
    let filesystem_metadata = prepare_filesystem_root(spec)?;
    if let Some(setup) = spec.setup {
        setup(context, run_id).await.map_err(|error| {
            subject_failure(
                FailurePhase::Execute,
                format!("scenario setup failed: {error}"),
            )
        })?;
    }
    let response: SendResponse = context
        .trigger(
            "harness::send",
            SendRequest {
                session_id: Some(session_id.to_string()),
                message: MessageInput::Text(spec.prompt.clone()),
                model: Some(subject.model.clone()),
                provider: Some(subject.provider.clone()),
                idempotency_key: Some(format!("e2e:{run_id}:{}:send", spec.id)),
                session: Some(SessionInit {
                    title: Some(format!("Harness E2E: {}", spec.id)),
                    metadata: Some(json!({
                        "e2e_run_id": run_id,
                        "e2e_scenario": spec.id,
                    })),
                }),
                options: Some(SendOptions {
                    max_turns: Some(spec.execution.max_turns),
                    max_output_tokens: spec.execution.max_output_tokens,
                    max_total_tokens: Some(spec.execution.max_total_tokens),
                    functions: Some(e2e_function_policy(spec)),
                    metadata: filesystem_metadata,
                    ..SendOptions::default()
                }),
            },
        )
        .await
        .map_err(|error| subject_failure(FailurePhase::Execute, error.to_string()))?;
    if !response.accepted
        || response.session_id != session_id
        || response.merged == Some(true)
        || response.queued == Some(true)
    {
        return Err(RunFailure::new(
            RunStatus::SubjectError,
            FailurePhase::Execute,
            format!("harness::send returned an unexpected response: {response:?}"),
        ));
    }

    if let Err(error) = context
        .wait_for_turn(
            spec.id,
            session_id,
            &response.turn_id,
            stuck_timeout,
            progress_interval,
        )
        .await
    {
        capture_partial_observation(context, session_id, report).await;
        return Err(subject_failure(FailurePhase::Execute, error.to_string()));
    }
    let metrics = match context
        .wait_for_complete_metrics(spec.id, session_id, stuck_timeout, progress_interval)
        .await
    {
        Ok(metrics) => metrics,
        Err(error) => {
            capture_partial_observation(context, session_id, report).await;
            return Err(collection_failure(FailurePhase::Collect, error.to_string()));
        }
    };
    let transcript = context.transcript(session_id).await.map_err(|error| {
        RunFailure::new(
            RunStatus::InfrastructureError,
            FailurePhase::Collect,
            error.to_string(),
        )
    })?;
    let response = common::final_response(&transcript);
    let observation = ScenarioObservation {
        metrics,
        transcript,
        response,
    };
    report.transcript = Some(observation.transcript.clone());
    report.metrics = Some(observation.metrics.clone());
    let objective = (spec.evaluate)(context, &observation, run_id)
        .await
        .map_err(|error| {
            RunFailure::new(
                RunStatus::InfrastructureError,
                FailurePhase::Evaluate,
                format!("scenario '{}' evaluator failed: {error:#}", spec.id),
            )
        })?;
    validate_objective_evaluation(spec, &objective).map_err(|error| {
        RunFailure::new(
            RunStatus::InfrastructureError,
            FailurePhase::Evaluate,
            error.to_string(),
        )
    })?;
    report.hard_gates = objective.hard_gates;
    let mut awards = objective.awards;

    if spec.needs_judge() && report.hard_gates.iter().all(|gate| gate.passed) {
        let judge_config = judge_config.ok_or_else(|| {
            RunFailure::new(
                RunStatus::InfrastructureError,
                FailurePhase::Setup,
                "scenario requires a judge configuration",
            )
        })?;
        match judge::evaluate(context, judge_config, spec, &observation.response).await {
            Ok(outcome) => {
                awards = outcome.awards;
                report.judge_attempts = Some(outcome.attempts);
                report.judge_usage = outcome.usage;
            }
            Err(error) => {
                return Err(RunFailure::new(
                    RunStatus::JudgeError,
                    FailurePhase::Evaluate,
                    format!("scenario '{}' judge evaluation failed: {error:#}", spec.id),
                ));
            }
        }
    }

    report.criteria = criterion_reports(spec, awards);
    update_score(report);
    Ok(())
}

fn prepare_filesystem_root(spec: &ScenarioSpec) -> Result<Option<serde_json::Value>, RunFailure> {
    let Some(root) = spec.filesystem_root.as_ref() else {
        return Ok(None);
    };
    if !root.is_absolute() {
        return Err(RunFailure::new(
            RunStatus::InfrastructureError,
            FailurePhase::Execute,
            format!(
                "scenario {} filesystem root must be absolute: {}",
                spec.id,
                root.display()
            ),
        ));
    }
    std::fs::create_dir_all(root).map_err(|error| {
        RunFailure::new(
            RunStatus::InfrastructureError,
            FailurePhase::Execute,
            format!(
                "create scenario {} filesystem root {}: {error}",
                spec.id,
                root.display()
            ),
        )
    })?;
    let root = root.to_str().ok_or_else(|| {
        RunFailure::new(
            RunStatus::InfrastructureError,
            FailurePhase::Execute,
            format!(
                "scenario {} filesystem root is not valid UTF-8: {}",
                spec.id,
                root.display()
            ),
        )
    })?;
    Ok(Some(json!({ "fs_scope": { "root": root } })))
}

async fn capture_partial_observation(
    context: &E2eContext,
    session_id: &str,
    report: &mut E2eRunReport,
) {
    match context.metrics(session_id).await {
        Ok(metrics) => report.metrics = Some(metrics),
        Err(error) => tracing::warn!(
            session_id,
            %error,
            "could not capture partial E2E metrics"
        ),
    }
    match context.transcript(session_id).await {
        Ok(transcript) => report.transcript = Some(transcript),
        Err(error) => tracing::warn!(
            session_id,
            %error,
            "could not capture partial E2E transcript"
        ),
    }
}

fn is_resource_limit(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "token budget",
        "tokens remain",
        "max_total_tokens",
        "cost budget",
        "scenario exceeded",
        "no observable progress",
        "maximum turn",
        "turn limit",
        "context length",
        "input limit",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn is_retryable_technical_failure(report: &E2eRunReport) -> bool {
    if !matches!(
        report.status,
        RunStatus::SubjectError | RunStatus::JudgeError | RunStatus::InfrastructureError
    ) || report
        .failures
        .iter()
        .any(|failure| failure.phase == FailurePhase::Cleanup)
    {
        return false;
    }
    report.failures.iter().any(|failure| {
        let lower = failure.message.to_ascii_lowercase();
        [
            "stream ended without",
            "terminal frame",
            "connection reset",
            "connection closed",
            "broken pipe",
            "temporarily unavailable",
            "service unavailable",
            "rate limit",
            "too many requests",
            "http 429",
            "status 429",
            "status 502",
            "status 503",
            "status 504",
            "transport error",
            "network error",
            "timed out",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
    })
}

fn subject_failure(phase: FailurePhase, message: String) -> RunFailure {
    let status = if is_resource_limit(&message) {
        RunStatus::ResourceLimit
    } else {
        RunStatus::SubjectError
    };
    RunFailure::new(status, phase, message)
}

fn collection_failure(phase: FailurePhase, message: String) -> RunFailure {
    let status = if is_resource_limit(&message) {
        RunStatus::ResourceLimit
    } else {
        RunStatus::InfrastructureError
    };
    RunFailure::new(status, phase, message)
}

fn update_score(report: &mut E2eRunReport) {
    report.score = report.criteria.iter().try_fold(0_u8, |score, criterion| {
        criterion
            .awarded
            .and_then(|awarded| score.checked_add(awarded))
    });
}

fn validate_objective_evaluation(
    spec: &ScenarioSpec,
    evaluation: &ObjectiveEvaluation,
) -> Result<()> {
    let mut gate_ids = HashSet::new();
    for gate in &evaluation.hard_gates {
        if gate.id.trim().is_empty() {
            bail!(
                "scenario '{}': evaluation contract violation: hard gate id is empty; expected a stable non-empty identifier",
                spec.id
            );
        }
        if gate.reason.trim().is_empty() {
            bail!(
                "scenario '{}': evaluation contract violation: hard gate '{}' has an empty reason; include the observed evidence",
                spec.id, gate.id
            );
        }
        if !gate_ids.insert(gate.id.as_str()) {
            bail!(
                "scenario '{}': evaluation contract violation: hard gate '{}' was returned more than once; expected unique gate ids",
                spec.id, gate.id
            );
        }
    }

    if spec.needs_judge() {
        if evaluation.awards.is_empty() {
            return Ok(());
        }
        bail!(
            "scenario '{}': evaluation contract violation: judge-backed evaluator returned awards [{}]; expected no awards because the judge owns criterion scoring",
            spec.id,
            evaluation
                .awards
                .iter()
                .map(|award| format!("'{}'", award.id))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    let criteria: HashMap<_, _> = spec
        .criteria
        .iter()
        .map(|criterion| (criterion.id, criterion))
        .collect();
    let mut seen = HashSet::new();
    for award in &evaluation.awards {
        if award.id.trim().is_empty() {
            bail!(
                "scenario '{}': evaluation contract violation: criterion award id is empty; expected one award per configured criterion",
                spec.id
            );
        }
        if award.reason.trim().is_empty() {
            bail!(
                "scenario '{}': evaluation contract violation: criterion '{}' has an empty reason; include the observed evidence",
                spec.id, award.id
            );
        }
        let criterion = criteria
            .get(award.id.as_str())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "scenario '{}': evaluation contract violation: unknown criterion '{}'; expected one of [{}]; action: return exactly one award for each configured criterion",
                    spec.id,
                    award.id,
                    spec.criteria
                        .iter()
                        .map(|criterion| format!("'{}'", criterion.id))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
        if award.awarded > criterion.weight {
            bail!(
                "scenario '{}': evaluation contract violation: criterion '{}' awarded {}; expected awarded in 0..={}; action: reduce the award or change the configured weight",
                spec.id, award.id,
                award.awarded,
                criterion.weight
            );
        }
        if !seen.insert(award.id.as_str()) {
            bail!(
                "scenario '{}': evaluation contract violation: criterion '{}' was returned more than once; expected exactly one award per configured criterion",
                spec.id, criterion.id
            );
        }
    }
    let missing = spec
        .criteria
        .iter()
        .filter(|criterion| !seen.contains(criterion.id))
        .map(|criterion| format!("'{}'", criterion.id))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        let received = evaluation
            .awards
            .iter()
            .map(|award| format!("'{}'", award.id))
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "scenario '{}': evaluation contract violation: missing awards [{}]; received [{}]; expected exactly one award per configured criterion",
            spec.id,
            missing.join(", "),
            received
        );
    }
    Ok(())
}

fn criterion_reports(spec: &ScenarioSpec, awards: Vec<CriterionAward>) -> Vec<CriterionReport> {
    let mut awards: HashMap<_, _> = awards
        .into_iter()
        .map(|award| (award.id, (award.awarded, award.reason)))
        .collect();
    spec.criteria
        .iter()
        .map(|criterion| {
            let award = awards.remove(criterion.id);
            CriterionReport {
                id: criterion.id.to_string(),
                possible: criterion.weight,
                awarded: award.as_ref().map(|(awarded, _)| *awarded),
                reason: award
                    .map(|(_, reason)| reason)
                    .unwrap_or_else(|| "not evaluated".into()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::HardGateReport;
    use crate::scenarios::{CriterionSpec, ExecutionPolicy, ScenarioEvaluator};

    fn evaluator<'a>(
        _context: &'a E2eContext,
        _observation: &'a ScenarioObservation,
        _run_id: &'a str,
    ) -> crate::scenarios::EvaluationFuture<'a> {
        unreachable!()
    }

    fn spec() -> ScenarioSpec {
        ScenarioSpec {
            id: "case",
            version: 1,
            prompt: "prompt".into(),
            filesystem_root: None,
            execution: ExecutionPolicy {
                max_turns: 1,
                max_output_tokens: Some(1),
                max_total_tokens: 1,
                stuck_timeout_seconds: 1,
            },
            denied_functions: &[],
            criteria: vec![CriterionSpec {
                id: "objective",
                weight: 100,
                description: "objective",
            }],
            judge_reference: None,
            setup: None,
            evaluate: evaluator as ScenarioEvaluator,
            cleanup: None,
        }
    }

    #[test]
    fn e2e_policy_allows_every_function_without_overrides() {
        let policy = e2e_function_policy(&spec());
        assert_eq!(policy.allow, ["*"]);
        assert!(policy.deny.is_empty());
        assert_eq!(policy.expose, Default::default());
    }

    #[test]
    fn e2e_policy_applies_scenario_denies() {
        let mut scenario = spec();
        scenario.denied_functions = &["state::*"];
        let policy = e2e_function_policy(&scenario);

        assert_eq!(policy.allow, ["*"]);
        assert_eq!(policy.deny, ["state::*"]);
    }

    #[test]
    fn objective_awards_must_be_complete_and_bounded() {
        let spec = spec();
        assert!(validate_objective_evaluation(
            &spec,
            &ObjectiveEvaluation {
                hard_gates: Vec::new(),
                awards: vec![CriterionAward {
                    id: "objective".into(),
                    awarded: 100,
                    reason: "ok".into(),
                }],
            }
        )
        .is_ok());
        assert!(validate_objective_evaluation(
            &spec,
            &ObjectiveEvaluation {
                hard_gates: Vec::new(),
                awards: Vec::new(),
            }
        )
        .is_err());
        let error = validate_objective_evaluation(
            &spec,
            &ObjectiveEvaluation {
                hard_gates: Vec::new(),
                awards: vec![CriterionAward {
                    id: "objective".into(),
                    awarded: 101,
                    reason: "too high".into(),
                }],
            },
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "scenario 'case': evaluation contract violation: criterion 'objective' awarded 101; expected awarded in 0..=100; action: reduce the award or change the configured weight"
        );
    }

    #[test]
    fn objective_contract_errors_identify_the_invalid_ids_and_values() {
        let spec = spec();
        let unknown = validate_objective_evaluation(
            &spec,
            &ObjectiveEvaluation {
                hard_gates: Vec::new(),
                awards: vec![CriterionAward {
                    id: "unknown".into(),
                    awarded: 1,
                    reason: "observed".into(),
                }],
            },
        )
        .unwrap_err();
        assert_eq!(
            unknown.to_string(),
            "scenario 'case': evaluation contract violation: unknown criterion 'unknown'; expected one of ['objective']; action: return exactly one award for each configured criterion"
        );

        let duplicate = validate_objective_evaluation(
            &spec,
            &ObjectiveEvaluation {
                hard_gates: Vec::new(),
                awards: vec![
                    CriterionAward {
                        id: "objective".into(),
                        awarded: 1,
                        reason: "first".into(),
                    },
                    CriterionAward {
                        id: "objective".into(),
                        awarded: 1,
                        reason: "second".into(),
                    },
                ],
            },
        )
        .unwrap_err();
        assert_eq!(
            duplicate.to_string(),
            "scenario 'case': evaluation contract violation: criterion 'objective' was returned more than once; expected exactly one award per configured criterion"
        );

        let empty_gate = validate_objective_evaluation(
            &spec,
            &ObjectiveEvaluation {
                hard_gates: vec![HardGateReport {
                    id: String::new(),
                    passed: false,
                    reason: "observed".into(),
                }],
                awards: vec![CriterionAward {
                    id: "objective".into(),
                    awarded: 1,
                    reason: "observed".into(),
                }],
            },
        )
        .unwrap_err();
        assert_eq!(
            empty_gate.to_string(),
            "scenario 'case': evaluation contract violation: hard gate id is empty; expected a stable non-empty identifier"
        );
    }

    #[test]
    fn hard_gate_failure_prevents_a_passing_run() {
        let mut report = E2eRunReport::new("run".into(), "session".into(), "prompt".into());
        report.hard_gates = vec![HardGateReport {
            id: "gate".into(),
            passed: false,
            reason: "failed".into(),
        }];
        report.criteria = criterion_reports(
            &spec(),
            vec![CriterionAward {
                id: "objective".into(),
                awarded: 100,
                reason: "ok".into(),
            }],
        );
        update_score(&mut report);
        report.finish(if report.hard_gates.iter().all(|gate| gate.passed) {
            RunStatus::Passed
        } else {
            RunStatus::HardGateFailed
        });
        assert_eq!(report.status, RunStatus::HardGateFailed);
    }

    #[test]
    fn token_budget_failures_are_classified_as_resource_limits() {
        let failure = subject_failure(
            FailurePhase::Execute,
            "generation requires more tokens than remain in the token budget".into(),
        );
        assert_eq!(failure.status, RunStatus::ResourceLimit);
        let collection = collection_failure(
            FailurePhase::Collect,
            "scenario exceeded 600s while waiting for the complete session tree".into(),
        );
        assert_eq!(collection.status, RunStatus::ResourceLimit);
    }

    #[test]
    fn only_transient_technical_failures_are_retried() {
        let mut transient = E2eRunReport::new("run".into(), "session".into(), "prompt".into());
        transient.push_failure(
            RunStatus::SubjectError,
            FailurePhase::Execute,
            "zai stream ended without a terminal frame",
        );
        assert!(is_retryable_technical_failure(&transient));

        let mut deterministic = E2eRunReport::new("run".into(), "session".into(), "prompt".into());
        deterministic.push_failure(
            RunStatus::JudgeError,
            FailurePhase::Evaluate,
            "judge returned an invalid criterion set",
        );
        assert!(!is_retryable_technical_failure(&deterministic));

        let mut cleanup = E2eRunReport::new("run".into(), "session".into(), "prompt".into());
        cleanup.push_failure(
            RunStatus::InfrastructureError,
            FailurePhase::Cleanup,
            "connection reset during cleanup",
        );
        assert!(!is_retryable_technical_failure(&cleanup));

        let mut budget = E2eRunReport::new("run".into(), "session".into(), "prompt".into());
        budget.push_failure(
            RunStatus::ResourceLimit,
            FailurePhase::Execute,
            "scenario exceeded its deadline",
        );
        assert!(!is_retryable_technical_failure(&budget));
    }
}
