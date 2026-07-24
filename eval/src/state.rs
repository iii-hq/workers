use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::contract::{
    ActiveRunV1, EvalRunStatusV1, EvalStatusResponseV1, EvalStatusV1, EvalSummaryV1,
    NormalizedEvalRequestV1,
};
use crate::error::EvalError;
use crate::report::{EvalReportV1, EvalRunReportV1};

pub const JOB_SCOPE: &str = "eval_job";
pub const SESSION_SCOPE: &str = "eval_session";
const DISPATCH_TIMEOUT_MS: u64 = 30_000;

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalJobRecordV1 {
    pub schema_version: String,
    pub evaluation_id: String,
    pub request: NormalizedEvalRequestV1,
    pub status: EvalStatusV1,
    pub step: u64,
    pub next_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_index: Option<usize>,
    /// The active root returned while descendants were still running. Once
    /// they settle, the eval asks that same root for one definitive result.
    #[serde(default, skip_serializing_if = "is_false")]
    pub active_waited_for_descendants: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub active_finalization_sent: bool,
    pub runs: Vec<EvalRunReportV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<EvalReportV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
}

impl EvalJobRecordV1 {
    pub fn status_response(&self) -> EvalStatusResponseV1 {
        let terminal_runs = self
            .runs
            .iter()
            .filter(|run| run.status.is_terminal())
            .count() as u32;
        let passed_runs = self
            .runs
            .iter()
            .filter(|run| run.passed == Some(true))
            .count() as u32;
        let failed_runs = self
            .runs
            .iter()
            .filter(|run| run.status.is_terminal() && run.passed == Some(false))
            .count() as u32;
        let active = self.active_index.and_then(|index| {
            self.runs.get(index).map(|run| ActiveRunV1 {
                run_id: run.run_id.clone(),
                role: run.role,
                iteration: run.iteration,
                session_id: run.session_id.clone(),
                turn_id: run.turn_id.clone(),
                started_at: run.started_at,
            })
        });
        EvalStatusResponseV1 {
            evaluation_id: self.evaluation_id.clone(),
            status: self.status,
            total_runs: self.runs.len() as u32,
            terminal_runs,
            passed_runs,
            failed_runs,
            active,
            created_at: self.created_at,
            updated_at: self.updated_at,
            completed_at: self.completed_at,
            error: self.error.clone(),
        }
    }

    pub fn summary_response(&self) -> EvalSummaryV1 {
        let progress = self.status_response();
        EvalSummaryV1 {
            evaluation_id: self.evaluation_id.clone(),
            status: self.status,
            dimension: self.request.dimension,
            model: self.request.model.model.clone(),
            provider: self.request.model.provider.clone(),
            control_label: self.request.control.label.clone(),
            treatment_label: self.request.treatment.label.clone(),
            total_runs: progress.total_runs,
            terminal_runs: progress.terminal_runs,
            passed_runs: progress.passed_runs,
            failed_runs: progress.failed_runs,
            eligible: self.report.as_ref().and_then(|report| report.eligible),
            created_at: self.created_at,
            updated_at: self.updated_at,
            completed_at: self.completed_at,
            error: self.error.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionIndexV1 {
    pub evaluation_id: String,
    pub run_id: String,
}

pub fn build_run_plan(
    evaluation_id: &str,
    request: &NormalizedEvalRequestV1,
) -> Vec<EvalRunReportV1> {
    let mut runs = Vec::with_capacity(request.runs as usize * 2);
    for iteration in 1..=request.runs {
        for (pair_index, role) in request
            .execution_order
            .roles(iteration)
            .into_iter()
            .enumerate()
        {
            runs.push(EvalRunReportV1 {
                run_id: crate::ids::run_id(role, iteration),
                role,
                iteration,
                execution_position: runs.len() as u32 + 1,
                pair_position: pair_index as u8 + 1,
                status: EvalRunStatusV1::Pending,
                session_id: crate::ids::session_id(evaluation_id, role, iteration),
                turn_id: None,
                passed: None,
                started_at: 0,
                completed_at: None,
                output: None,
                metrics: None,
                evaluation: None,
                benchmark: None,
                failures: Vec::new(),
            });
        }
    }
    runs
}

pub async fn get_job(
    iii: &IIIClient,
    evaluation_id: &str,
) -> Result<Option<EvalJobRecordV1>, EvalError> {
    let value = state_get(iii, JOB_SCOPE, evaluation_id).await?;
    if value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(|error| EvalError::State(format!("parse {JOB_SCOPE}/{evaluation_id}: {error}")))
}

pub async fn put_job(iii: &IIIClient, job: &EvalJobRecordV1) -> Result<(), EvalError> {
    state_set(
        iii,
        JOB_SCOPE,
        &job.evaluation_id,
        serde_json::to_value(job)?,
    )
    .await
}

pub async fn list_jobs(iii: &IIIClient) -> Result<Vec<EvalJobRecordV1>, EvalError> {
    let value = state_list(iii, JOB_SCOPE).await?;
    Ok(parse_list(&value))
}

pub async fn delete_job(iii: &IIIClient, evaluation_id: &str) -> Result<(), EvalError> {
    state_delete(iii, JOB_SCOPE, evaluation_id).await
}

pub async fn put_session_index(
    iii: &IIIClient,
    session_id: &str,
    index: &SessionIndexV1,
) -> Result<(), EvalError> {
    state_set(iii, SESSION_SCOPE, session_id, serde_json::to_value(index)?).await
}

pub async fn get_session_index(
    iii: &IIIClient,
    session_id: &str,
) -> Result<Option<SessionIndexV1>, EvalError> {
    let value = state_get(iii, SESSION_SCOPE, session_id).await?;
    if value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(|error| EvalError::State(format!("parse {SESSION_SCOPE}/{session_id}: {error}")))
}

pub async fn delete_session_index(iii: &IIIClient, session_id: &str) -> Result<(), EvalError> {
    state_delete(iii, SESSION_SCOPE, session_id).await
}

async fn state_get(iii: &IIIClient, scope: &str, key: &str) -> Result<Value, EvalError> {
    iii.trigger(TriggerRequest {
        function_id: "state::get".into(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: Some(DISPATCH_TIMEOUT_MS),
    })
    .await
    .map_err(|error| EvalError::State(format!("state::get {scope}/{key}: {error}")))
}

async fn state_set(iii: &IIIClient, scope: &str, key: &str, value: Value) -> Result<(), EvalError> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".into(),
        payload: json!({ "scope": scope, "key": key, "value": value }),
        action: None,
        timeout_ms: Some(DISPATCH_TIMEOUT_MS),
    })
    .await
    .map(|_| ())
    .map_err(|error| EvalError::State(format!("state::set {scope}/{key}: {error}")))
}

async fn state_list(iii: &IIIClient, scope: &str) -> Result<Value, EvalError> {
    iii.trigger(TriggerRequest {
        function_id: "state::list".into(),
        payload: json!({ "scope": scope }),
        action: None,
        timeout_ms: Some(DISPATCH_TIMEOUT_MS),
    })
    .await
    .map_err(|error| EvalError::State(format!("state::list {scope}: {error}")))
}

async fn state_delete(iii: &IIIClient, scope: &str, key: &str) -> Result<(), EvalError> {
    iii.trigger(TriggerRequest {
        function_id: "state::delete".into(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: Some(DISPATCH_TIMEOUT_MS),
    })
    .await
    .map(|_| ())
    .map_err(|error| EvalError::State(format!("state::delete {scope}/{key}: {error}")))
}

fn parse_list<T: serde::de::DeserializeOwned>(value: &Value) -> Vec<T> {
    let candidates: Vec<&Value> = match value {
        Value::Array(items) => items.iter().collect(),
        Value::Object(map) => {
            if let Some(Value::Array(items)) = map.get("values").or_else(|| map.get("items")) {
                items.iter().collect()
            } else {
                map.values().collect()
            }
        }
        _ => return Vec::new(),
    };
    candidates
        .into_iter()
        .filter_map(|value| serde_json::from_value(value.clone()).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use harness::prompt::SystemPromptStrategy;
    use harness::types::output::OutputContract;
    use harness::types::turn::FunctionPolicy;
    use serde_json::json;

    use super::*;
    use crate::contract::{
        ComparisonDimensionV1, EvalModelConfigV1, EvalVariantV1, EvaluatorSpecV1, VariantRoleV1,
    };
    use crate::limits::EvalLimitsV1;

    fn request(runs: u32) -> NormalizedEvalRequestV1 {
        NormalizedEvalRequestV1 {
            dimension: ComparisonDimensionV1::Prompt,
            model: EvalModelConfigV1 {
                model: "model".into(),
                provider: None,
                system_prompt_strategy: SystemPromptStrategy::Override,
                mode: None,
                thinking_level: None,
                provider_options: None,
            },
            control: EvalVariantV1 {
                label: None,
                prompt: "a".into(),
                system_prompt: Some("system".into()),
            },
            treatment: EvalVariantV1 {
                label: None,
                prompt: "b".into(),
                system_prompt: Some("system".into()),
            },
            evaluator: Some(EvaluatorSpecV1 {
                function_id: "judge".into(),
                arguments: json!({}),
            }),
            runs,
            execution_order: crate::contract::ExecutionOrderV1::default(),
            source_evaluation_id: None,
            limits: EvalLimitsV1::default(),
            functions: FunctionPolicy::default(),
            output: OutputContract::Text,
            metadata: None,
        }
    }

    #[test]
    fn run_plan_alternates_pair_order() {
        let runs = build_run_plan("eval_1", &request(3));
        let roles: Vec<_> = runs.iter().map(|run| run.role).collect();
        assert_eq!(
            roles,
            [
                VariantRoleV1::Control,
                VariantRoleV1::Treatment,
                VariantRoleV1::Treatment,
                VariantRoleV1::Control,
                VariantRoleV1::Control,
                VariantRoleV1::Treatment,
            ]
        );
        assert_eq!(runs[0].session_id, "eval_1_c_1");
        assert_eq!(runs[2].session_id, "eval_1_t_2");
        assert_eq!(runs[0].pair_position, 1);
        assert_eq!(runs[1].pair_position, 2);
        assert_eq!(runs[5].execution_position, 6);
    }

    #[test]
    fn reversed_run_plan_preserves_roles_and_inverts_each_pair() {
        let mut request = request(2);
        request.execution_order = crate::contract::ExecutionOrderV1::BalancedTreatmentFirst;
        let runs = build_run_plan("eval_2", &request);
        let roles: Vec<_> = runs.iter().map(|run| run.role).collect();
        assert_eq!(
            roles,
            [
                VariantRoleV1::Treatment,
                VariantRoleV1::Control,
                VariantRoleV1::Control,
                VariantRoleV1::Treatment,
            ]
        );
        assert_eq!(runs[0].run_id, "treatment-1");
        assert_eq!(runs[1].run_id, "control-1");
    }

    #[test]
    fn list_parser_accepts_all_state_shapes() {
        let index = json!({"evaluation_id": "e", "run_id": "r"});
        assert_eq!(parse_list::<SessionIndexV1>(&json!([index])).len(), 1);
        assert_eq!(
            parse_list::<SessionIndexV1>(&json!({"values": [index]})).len(),
            1
        );
        assert_eq!(
            parse_list::<SessionIndexV1>(&json!({"key": index})).len(),
            1
        );
    }

    #[test]
    fn summary_projects_progress_without_the_full_job() {
        let mut request = request(1);
        request.control.label = Some("baseline".into());
        request.treatment.label = Some("candidate".into());
        let mut runs = build_run_plan("eval_1", &request);
        runs[0].status = EvalRunStatusV1::Completed;
        runs[0].passed = Some(true);
        let job = EvalJobRecordV1 {
            schema_version: "1".into(),
            evaluation_id: "eval_1".into(),
            request,
            status: EvalStatusV1::Running,
            step: 1,
            next_index: 1,
            active_index: Some(1),
            active_waited_for_descendants: false,
            active_finalization_sent: false,
            runs,
            report: None,
            error: None,
            created_at: 10,
            updated_at: 20,
            completed_at: None,
        };

        let summary = job.summary_response();
        assert_eq!(summary.control_label.as_deref(), Some("baseline"));
        assert_eq!(summary.treatment_label.as_deref(), Some("candidate"));
        assert_eq!(summary.total_runs, 2);
        assert_eq!(summary.terminal_runs, 1);
        assert_eq!(summary.passed_runs, 1);
        assert_eq!(summary.failed_runs, 0);
        assert_eq!(summary.eligible, None);
    }

    #[test]
    fn completed_run_without_evaluation_is_neither_passed_nor_failed() {
        let request = request(1);
        let mut runs = build_run_plan("eval_1", &request);
        runs[0].status = EvalRunStatusV1::Completed;
        runs[0].output = Some(Value::Null);
        let job = EvalJobRecordV1 {
            schema_version: "1".into(),
            evaluation_id: "eval_1".into(),
            request,
            status: EvalStatusV1::Running,
            step: 1,
            next_index: 1,
            active_index: Some(1),
            active_waited_for_descendants: false,
            active_finalization_sent: false,
            runs,
            report: None,
            error: None,
            created_at: 10,
            updated_at: 20,
            completed_at: None,
        };

        let status = job.status_response();
        assert_eq!(status.terminal_runs, 1);
        assert_eq!(status.passed_runs, 0);
        assert_eq!(status.failed_runs, 0);
    }
}
