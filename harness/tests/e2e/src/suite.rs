use std::path::PathBuf;

use uuid::Uuid;

use crate::context::ScenarioContext;
use crate::error::EvalError;
use crate::limits::E2eLimitsV1;
use crate::report::E2eRunReportV1;
use crate::scenarios::ScenarioId;
use crate::subject::ResolvedE2eSubjectV1;

pub struct SuiteRunConfig {
    pub url: String,
    pub subject: ResolvedE2eSubjectV1,
    pub output: PathBuf,
    pub scenarios: Vec<ScenarioId>,
    pub limits: E2eLimitsV1,
}

pub struct SuiteRunOutcome {
    pub report: E2eRunReportV1,
    pub run_dir: PathBuf,
}

pub async fn run_suite(config: SuiteRunConfig) -> Result<SuiteRunOutcome, EvalError> {
    config
        .limits
        .validate()
        .map_err(|error| EvalError::setup(error.to_string()))?;
    let run_id = Uuid::new_v4().simple().to_string();
    let subject_artifact = config.subject.artifact();
    let context = ScenarioContext::connect(
        &config.url,
        &run_id,
        config.subject,
        config.limits.execution.invocation_timeout(),
        config.limits.execution.scenario_timeout(),
    )
    .await?;

    let mut scenario_reports = Vec::with_capacity(config.scenarios.len());
    for scenario_id in config.scenarios {
        tracing::info!(
            scenario = scenario_id.as_str(),
            subject = %subject_artifact.subject_id,
            "running e2e scenario"
        );
        scenario_reports.push(scenario_id.run(&context, &run_id, &config.limits).await);
    }

    let report = E2eRunReportV1::new(run_id, subject_artifact, config.limits, scenario_reports);
    let write_result = report.write_to(&config.output);
    context.shutdown().await;
    let run_dir = write_result?;
    Ok(SuiteRunOutcome { report, run_dir })
}
