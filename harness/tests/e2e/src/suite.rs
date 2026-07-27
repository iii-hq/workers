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
    RunStatus,
};
use crate::scenarios::common;
use crate::scenarios::{CriterionAward, ScenarioId, ScenarioObservation, ScenarioSpec};

const MAX_RUNS: u32 = 20;

fn e2e_function_policy() -> FunctionPolicy {
    FunctionPolicy {
        allow: vec!["*".into()],
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
            runs.push(
                run_once(
                    &context,
                    *scenario_id,
                    &config.subject,
                    config.judge.as_ref(),
                )
                .await,
            );
        }
        scenario_reports.push(E2eScenarioReport::aggregate(
            definition.id,
            definition.threshold,
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
) -> E2eRunReport {
    let started = Instant::now();
    let run_id = Uuid::new_v4().simple().to_string();
    let session_id = format!("e2e_{run_id}");
    let spec = scenario_id.spec(&run_id);
    let mut report = E2eRunReport::new(run_id.clone(), session_id.clone(), spec.prompt.clone());

    if let Err(error) = execute(
        context,
        subject,
        judge_config,
        &run_id,
        &session_id,
        &spec,
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
            format!("harness::teardown: {error}"),
        );
    }
    if let Some(cleanup) = spec.cleanup {
        if let Err(error) = cleanup(context, &run_id).await {
            report.push_failure(
                RunStatus::InfrastructureError,
                FailurePhase::Cleanup,
                error.to_string(),
            );
        }
    }
    report.wall_time_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    if report.failures.is_empty() {
        if report.hard_gates.iter().any(|gate| !gate.passed) {
            report.finish(RunStatus::HardGateFailed);
        } else if let Some(score) = report.score {
            report.finish(if score >= spec.threshold {
                RunStatus::Passed
            } else {
                RunStatus::QualityFailed
            });
        } else {
            report.push_failure(
                RunStatus::InfrastructureError,
                FailurePhase::Evaluate,
                "evaluation completed without a score",
            );
        }
    }
    report
}

struct RunFailure {
    status: RunStatus,
    phase: FailurePhase,
    message: String,
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
    subject: &SubjectConfig,
    judge_config: Option<&JudgeConfig>,
    run_id: &str,
    session_id: &str,
    spec: &ScenarioSpec,
    report: &mut E2eRunReport,
) -> Result<(), RunFailure> {
    let scenario_timeout = Duration::from_secs(spec.execution.timeout_seconds);
    let scenario_deadline = tokio::time::Instant::now() + scenario_timeout;
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
                    max_output_tokens: Some(spec.execution.max_output_tokens),
                    max_total_tokens: Some(spec.execution.max_total_tokens),
                    functions: Some(e2e_function_policy()),
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

    context
        .wait_for_turn(session_id, &response.turn_id, remaining(scenario_deadline))
        .await
        .map_err(|error| subject_failure(FailurePhase::Execute, error.to_string()))?;
    let metrics = context
        .wait_for_complete_metrics(session_id, remaining(scenario_deadline))
        .await
        .map_err(|error| collection_failure(FailurePhase::Collect, error.to_string()))?;
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
                error.to_string(),
            )
        })?;
    validate_objective_awards(spec, &objective.awards).map_err(|error| {
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
            }
            Err(error) => {
                return Err(RunFailure::new(
                    RunStatus::JudgeError,
                    FailurePhase::Evaluate,
                    error.to_string(),
                ));
            }
        }
    }

    report.criteria = criterion_reports(spec, awards);
    update_score(report);
    Ok(())
}

fn remaining(deadline: tokio::time::Instant) -> Duration {
    deadline.saturating_duration_since(tokio::time::Instant::now())
}

fn is_resource_limit(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "token budget",
        "tokens remain",
        "max_total_tokens",
        "cost budget",
        "scenario exceeded",
        "maximum turn",
        "turn limit",
        "context length",
        "input limit",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
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

fn validate_objective_awards(spec: &ScenarioSpec, awards: &[CriterionAward]) -> Result<()> {
    if spec.needs_judge() {
        if awards.is_empty() {
            return Ok(());
        }
        bail!(
            "scenario {} delegates all criterion scores to the judge",
            spec.id
        );
    }
    let criteria: HashMap<_, _> = spec
        .criteria
        .iter()
        .map(|criterion| (criterion.id, criterion))
        .collect();
    let mut seen = HashSet::new();
    for award in awards {
        let criterion = criteria
            .get(award.id.as_str())
            .with_context(|| format!("unknown objective criterion {}", award.id))?;
        if award.awarded > criterion.weight {
            bail!(
                "criterion {} awarded {} of {} points",
                award.id,
                award.awarded,
                criterion.weight
            );
        }
        if !seen.insert(award.id.as_str()) {
            bail!("objective evaluator repeated criterion {}", criterion.id);
        }
    }
    for criterion in &spec.criteria {
        if !seen.contains(criterion.id) {
            bail!("objective evaluator omitted criterion {}", criterion.id);
        }
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
            prompt: "prompt".into(),
            execution: ExecutionPolicy {
                max_turns: 1,
                max_output_tokens: 1,
                max_total_tokens: 1,
                timeout_seconds: 1,
            },
            threshold: 80,
            criteria: vec![CriterionSpec {
                id: "objective",
                weight: 100,
                description: "objective",
            }],
            judge_reference: None,
            evaluate: evaluator as ScenarioEvaluator,
            cleanup: None,
        }
    }

    #[test]
    fn e2e_policy_allows_every_function_without_overrides() {
        let policy = e2e_function_policy();
        assert_eq!(policy.allow, ["*"]);
        assert!(policy.deny.is_empty());
        assert_eq!(policy.expose, Default::default());
    }

    #[test]
    fn objective_awards_must_be_complete_and_bounded() {
        let spec = spec();
        assert!(validate_objective_awards(
            &spec,
            &[CriterionAward {
                id: "objective".into(),
                awarded: 100,
                reason: "ok".into(),
            }]
        )
        .is_ok());
        assert!(validate_objective_awards(&spec, &[]).is_err());
        assert!(validate_objective_awards(
            &spec,
            &[CriterionAward {
                id: "objective".into(),
                awarded: 101,
                reason: "too high".into(),
            }]
        )
        .is_err());
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
        report.finish(
            if report.hard_gates.iter().all(|gate| gate.passed)
                && report.score.is_some_and(|score| score >= spec().threshold)
            {
                RunStatus::Passed
            } else {
                RunStatus::HardGateFailed
            },
        );
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
}
