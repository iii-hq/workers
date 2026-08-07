mod api;
mod assets;
mod controller;
mod presenter;
mod store;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Result;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use clap::Args;
use serde::{Deserialize, Serialize};
use serde_json::json;

const LOCAL_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Args)]
pub struct DashboardArgs {
    /// Loopback address used by the local dashboard.
    #[arg(long, default_value = "127.0.0.1:4173")]
    pub listen: SocketAddr,

    /// WebSocket URL of the running Harness stack.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    pub url: String,

    /// Directory that owns local run metadata, logs, and reports.
    #[arg(long, default_value = "target/harness-e2e-local-runs")]
    pub runs_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
struct Defaults {
    url: String,
    model: String,
    provider: String,
    judge_model: String,
    judge_provider: String,
    runs: u32,
    technical_retries: u8,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct RunRequest {
    #[serde(default)]
    label: String,
    url: String,
    model: String,
    provider: String,
    #[serde(default)]
    judge_model: String,
    #[serde(default)]
    judge_provider: String,
    scenarios: Vec<String>,
    runs: u32,
    technical_retries: u8,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum JobStatus {
    Running,
    Cancelling,
    Cancelled,
    Completed,
    Failed,
}

impl JobStatus {
    fn active(self) -> bool {
        matches!(self, Self::Running | Self::Cancelling)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct RunMetadata {
    schema_version: u32,
    id: String,
    label: String,
    status: JobStatus,
    started_at: String,
    completed_at: String,
    returncode: Option<i32>,
    error: String,
    request: RunRequest,
}

#[derive(Debug, Serialize)]
struct JobView {
    #[serde(flatten)]
    metadata: RunMetadata,
    log: String,
}

#[derive(Debug, Serialize)]
struct RunSnapshot {
    job: Option<JobView>,
    defaults: Defaults,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

pub async fn serve(args: DashboardArgs) -> Result<()> {
    api::serve(args).await
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;

    use super::controller::{build_run_command, validate_request};
    use super::presenter::{
        contract_fingerprint, execution_detail_value, execution_summary, load_execution_summaries,
    };
    use super::store::{read_metadata, write_metadata};
    use super::*;
    use crate::report::{
        CostReport, E2eReport, E2eRunReport, E2eScenarioReport, ModelArtifact, RunStatus,
    };
    use crate::scenarios::ExecutionPolicy;

    fn request() -> RunRequest {
        RunRequest {
            label: " baseline ".into(),
            url: "ws://127.0.0.1:49134".into(),
            model: "model".into(),
            provider: "provider".into(),
            judge_model: String::new(),
            judge_provider: String::new(),
            scenarios: vec!["direct_answer".into()],
            runs: 1,
            technical_retries: 1,
        }
    }

    fn report() -> E2eReport {
        let mut run = E2eRunReport::new("run".into(), "session".into(), "prompt".into());
        run.wall_time_ms = 1_500;
        run.score = Some(90);
        run.status = RunStatus::Passed;
        run.cost = CostReport {
            subject_usd: Some(0.1),
            judge_usd: Some(0.0),
            total_usd: Some(0.1),
        };
        E2eReport::new(
            ModelArtifact {
                model: "model".into(),
                provider: "provider".into(),
                context_window: 100,
                max_output_tokens: 10,
                supports_tools: Some(true),
                supports_vision: Some(false),
            },
            None,
            None,
            None,
            vec![E2eScenarioReport::aggregate(
                "direct_answer",
                1,
                80,
                ExecutionPolicy {
                    max_turns: 1,
                    max_output_tokens: Some(10),
                    max_total_tokens: 100,
                    stuck_timeout_seconds: 10,
                },
                vec![run],
            )],
        )
    }

    fn metadata() -> RunMetadata {
        RunMetadata {
            schema_version: LOCAL_SCHEMA_VERSION,
            id: "local-20260807T120000-abcdef12".into(),
            label: "baseline".into(),
            status: JobStatus::Completed,
            started_at: "2026-08-07T12:00:00Z".into(),
            completed_at: "2026-08-07T12:00:02Z".into(),
            returncode: Some(0),
            error: String::new(),
            request: request(),
        }
    }

    #[test]
    fn validates_and_normalizes_run_requests() {
        let mut value = request();
        validate_request(&mut value).unwrap();
        assert_eq!(value.label, "baseline");
        value.url = "https://example.com".into();
        assert!(validate_request(&mut value).is_err());
    }

    #[test]
    fn builds_self_invocation_without_cargo() {
        let command = build_run_command(
            Path::new("/tmp/harness-e2e"),
            &request(),
            Path::new("/tmp/results"),
        );
        let std = command.as_std();
        assert_eq!(std.get_program(), "/tmp/harness-e2e");
        let args: Vec<_> = std
            .get_args()
            .map(|value| value.to_string_lossy())
            .collect();
        assert_eq!(args[0], "run");
        assert!(args.contains(&"direct_answer".into()));
        assert!(!args.iter().any(|value| value == "cargo"));
    }

    #[test]
    fn local_summary_and_detail_use_the_static_dashboard_contract() {
        let report = report();
        let summary = execution_summary(&metadata(), Some(&report)).unwrap();
        assert_eq!(summary["status"], "passed");
        assert_eq!(
            summary["detail_path"],
            "runs/local-20260807T120000-abcdef12.json"
        );
        assert_eq!(
            summary["subjects"][0]["scenarios"][0]["id"],
            "direct_answer"
        );
        let detail = execution_detail_value(&metadata(), &report).unwrap();
        assert_eq!(
            detail["reports"][0]["report"]["scenarios"][0]["scenario_id"],
            "direct_answer"
        );
    }

    #[test]
    fn contract_fingerprint_matches_the_browser_implementation() {
        let value = json!({
            "execution_policy": {},
            "scenario_id": "direct_answer",
            "scenario_version": 1,
            "threshold": 50,
        });
        assert_eq!(contract_fingerprint(&value), "fnv1a32:607c4fd2");
    }

    #[test]
    fn local_store_accepts_only_native_metadata_owned_runs() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join("legacy-import");
        report().write_to(&legacy.join("results")).unwrap();
        assert!(load_execution_summaries(root.path()).unwrap().is_empty());

        let metadata = metadata();
        let native = root.path().join(&metadata.id);
        write_metadata(&native, &metadata).unwrap();
        report().write_to(&native.join("results")).unwrap();
        let summaries = load_execution_summaries(root.path()).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0]["id"], metadata.id);
    }

    #[test]
    fn local_store_rejects_unknown_schema_versions() {
        let root = tempfile::tempdir().unwrap();
        let run = root.path().join("local-20260807T120000-abcdef12");
        let mut metadata = metadata();
        metadata.schema_version = LOCAL_SCHEMA_VERSION + 1;
        write_metadata(&run, &metadata).unwrap();
        let error = read_metadata(&run).unwrap_err();
        assert!(error.to_string().contains("unsupported local run schema"));
    }
}
