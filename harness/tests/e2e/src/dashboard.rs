use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use axum::body::Body;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{SecondsFormat, Utc};
use clap::Args;
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use url::Url;

use crate::context::E2eContext;
use crate::report::{E2eReport, E2eRunReport, E2eScenarioReport};
use crate::scenarios::ScenarioId;

const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_LOG_TAIL_BYTES: u64 = 256 * 1024;
const MAX_EXECUTIONS: usize = 100;
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

#[derive(RustEmbed)]
#[folder = "../../../.github/benchmark-site/"]
#[exclude = "*.test.cjs"]
#[exclude = "README.md"]
#[exclude = "sample-data.js"]
#[exclude = "sample-executions.js"]
struct DashboardAssets;

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

struct ControllerState {
    job: Option<RunMetadata>,
    child: Option<Child>,
}

struct Controller {
    runs_dir: PathBuf,
    executable: PathBuf,
    defaults: Defaults,
    state: Mutex<ControllerState>,
}

#[derive(Clone)]
struct AppState {
    controller: Arc<Controller>,
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

#[derive(Debug, Deserialize)]
struct CatalogQuery {
    url: Option<String>,
}

pub async fn serve(args: DashboardArgs) -> Result<()> {
    if !args.listen.ip().is_loopback() {
        bail!("dashboard --listen must use a loopback address; use SSH port forwarding for remote access");
    }
    validate_stack_url(&args.url)?;
    fs::create_dir_all(&args.runs_dir)
        .with_context(|| format!("create {}", args.runs_dir.display()))?;
    recover_interrupted_runs(&args.runs_dir)?;

    let controller = Arc::new(Controller {
        runs_dir: args.runs_dir,
        executable: env::current_exe().context("resolve harness-e2e executable")?,
        defaults: Defaults {
            url: args.url,
            model: env::var("HARNESS_E2E_MODEL").unwrap_or_default(),
            provider: env::var("HARNESS_E2E_PROVIDER").unwrap_or_default(),
            judge_model: env::var("HARNESS_E2E_JUDGE_MODEL").unwrap_or_default(),
            judge_provider: env::var("HARNESS_E2E_JUDGE_PROVIDER").unwrap_or_default(),
            runs: 1,
            technical_retries: 1,
        },
        state: Mutex::new(ControllerState {
            job: None,
            child: None,
        }),
    });
    let state = AppState { controller };
    let app = Router::new()
        .route("/api/local/run", get(run_snapshot).post(start_run))
        .route("/api/local/run/cancel", axum::routing::post(cancel_run))
        .route("/api/local/catalog", get(catalog))
        .route("/data.js", get(benchmark_data))
        .route("/executions.js", get(execution_manifest))
        .route("/runs/:id", get(execution_detail))
        .fallback(get(static_asset))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("bind dashboard on {}", args.listen))?;
    println!("dashboard: http://{}/index.html", args.listen);
    println!("press Ctrl+C to stop");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .context("serve local dashboard")?;
    Ok(())
}

async fn run_snapshot(State(state): State<AppState>) -> Result<Json<RunSnapshot>, ApiError> {
    state
        .controller
        .snapshot()
        .await
        .map(Json)
        .map_err(ApiError::internal)
}

async fn start_run(
    State(state): State<AppState>,
    Json(request): Json<RunRequest>,
) -> Result<(StatusCode, Json<RunSnapshot>), ApiError> {
    state.controller.start(request).await?;
    let snapshot = state
        .controller
        .snapshot()
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::ACCEPTED, Json(snapshot)))
}

async fn cancel_run(
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<RunSnapshot>), ApiError> {
    state.controller.cancel().await?;
    let snapshot = state
        .controller
        .snapshot()
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::ACCEPTED, Json(snapshot)))
}

async fn catalog(
    State(state): State<AppState>,
    Query(query): Query<CatalogQuery>,
) -> Result<Json<Value>, ApiError> {
    let url = query
        .url
        .unwrap_or_else(|| state.controller.defaults.url.clone());
    validate_stack_url(&url).map_err(|error| ApiError::bad_request(error.to_string()))?;
    let context = E2eContext::connect(&url)
        .await
        .map_err(ApiError::internal)?;
    if !context
        .function_exists("harness::send")
        .await
        .map_err(ApiError::internal)?
    {
        context.shutdown().await;
        return Err(ApiError::bad_request(
            "connected iii stack does not expose harness::send",
        ));
    }
    if !context
        .function_exists("router::models::list")
        .await
        .map_err(ApiError::internal)?
    {
        context.shutdown().await;
        return Err(ApiError::bad_request(
            "connected Harness stack does not expose router::models::list; start its llm-router",
        ));
    }
    let models = crate::catalog::list(&context, None)
        .await
        .map_err(ApiError::internal);
    context.shutdown().await;
    let models = models?;
    if models.is_empty() {
        return Err(ApiError::bad_request(
            "the running Harness has no registered models",
        ));
    }
    let scenarios: Vec<_> = ScenarioId::ALL.iter().map(|value| value.as_str()).collect();
    Ok(Json(
        json!({ "url": url, "models": models, "scenarios": scenarios }),
    ))
}

async fn benchmark_data() -> Response {
    javascript_response("window.BENCHMARK_DATA = {};\n".into())
}

async fn execution_manifest(State(state): State<AppState>) -> Result<Response, ApiError> {
    let executions =
        load_execution_summaries(&state.controller.runs_dir).map_err(ApiError::internal)?;
    let last_update = executions
        .first()
        .and_then(|value| value.get("completed_at"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    Ok(javascript_response(format!(
        "window.HARNESS_EXECUTIONS = {};\n",
        json!({
            "schema_version": 4,
            "mode": "local",
            "last_update": last_update,
            "repo_url": repository_url(),
            "retention": { "summaries": MAX_EXECUTIONS, "details": MAX_EXECUTIONS },
            "executions": executions,
        })
    )))
}

async fn execution_detail(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    let id = id
        .strip_suffix(".json")
        .ok_or_else(|| ApiError::bad_request("execution detail must end in .json"))?
        .to_string();
    validate_execution_id(&id).map_err(ApiError::bad_request)?;
    let metadata = read_metadata(&state.controller.runs_dir.join(&id))
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError {
            status: StatusCode::NOT_FOUND,
            message: "execution not found".into(),
        })?;
    let report = read_report(&state.controller.runs_dir.join(&id))
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError {
            status: StatusCode::NOT_FOUND,
            message: "execution report not found".into(),
        })?;
    execution_detail_value(&metadata, &report)
        .map(Json)
        .map_err(ApiError::internal)
}

async fn static_asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let Some(asset) = DashboardAssets::get(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let mut response = Response::new(Body::from(asset.data));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime.as_ref())
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn javascript_response(body: String) -> Response {
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/javascript; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

impl Controller {
    async fn snapshot(&self) -> Result<RunSnapshot> {
        let metadata = self.state.lock().await.job.clone();
        let job = metadata
            .map(|metadata| {
                let log = read_log_tail(&self.runs_dir.join(&metadata.id).join("run.log"))?;
                Ok::<_, anyhow::Error>(JobView { metadata, log })
            })
            .transpose()?;
        Ok(RunSnapshot {
            job,
            defaults: self.defaults.clone(),
        })
    }

    async fn start(self: &Arc<Self>, mut request: RunRequest) -> Result<(), ApiError> {
        validate_request(&mut request).map_err(ApiError::bad_request)?;
        let mut state = self.state.lock().await;
        if state.job.as_ref().is_some_and(|job| job.status.active()) {
            return Err(ApiError::conflict("an E2E execution is already running"));
        }

        let now = Utc::now();
        let id = format!(
            "local-{}-{}",
            now.format("%Y%m%dT%H%M%S"),
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        let run_dir = self.runs_dir.join(&id);
        let output_dir = run_dir.join("results");
        fs::create_dir_all(&output_dir).map_err(ApiError::internal)?;
        let log_path = run_dir.join("run.log");
        let stdout = File::create(&log_path).map_err(ApiError::internal)?;
        let stderr = stdout.try_clone().map_err(ApiError::internal)?;

        let mut command = build_run_command(&self.executable, &request, &output_dir);
        command.kill_on_drop(true);
        let child = command
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .map_err(ApiError::internal)?;
        let metadata = RunMetadata {
            schema_version: LOCAL_SCHEMA_VERSION,
            id: id.clone(),
            label: request.label.clone(),
            status: JobStatus::Running,
            started_at: now.to_rfc3339_opts(SecondsFormat::Millis, true),
            completed_at: String::new(),
            returncode: None,
            error: String::new(),
            request,
        };
        write_metadata(&run_dir, &metadata).map_err(ApiError::internal)?;
        state.job = Some(metadata);
        state.child = Some(child);
        drop(state);

        let controller = Arc::clone(self);
        tokio::spawn(async move { controller.monitor(id).await });
        Ok(())
    }

    async fn cancel(&self) -> Result<(), ApiError> {
        let mut state = self.state.lock().await;
        let Some(job) = state.job.as_ref() else {
            return Err(ApiError::conflict("no E2E execution is running"));
        };
        if job.status != JobStatus::Running {
            return Err(ApiError::conflict("no E2E execution is running"));
        }
        let id = job.id.clone();
        if let Some(child) = state.child.as_mut() {
            child.start_kill().map_err(ApiError::internal)?;
        }
        let job = state.job.as_mut().expect("job checked above");
        job.status = JobStatus::Cancelling;
        write_metadata(&self.runs_dir.join(id), job).map_err(ApiError::internal)?;
        Ok(())
    }

    async fn monitor(self: Arc<Self>, id: String) {
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let finished = {
                let mut state = self.state.lock().await;
                match state
                    .child
                    .as_mut()
                    .and_then(|child| child.try_wait().transpose())
                {
                    Some(Ok(status)) => {
                        state.child = None;
                        Some(Ok(status))
                    }
                    Some(Err(error)) => {
                        state.child = None;
                        Some(Err(error))
                    }
                    None => None,
                }
            };
            let Some(result) = finished else { continue };
            let mut state = self.state.lock().await;
            let Some(job) = state.job.as_mut().filter(|job| job.id == id) else {
                return;
            };
            let cancelling = job.status == JobStatus::Cancelling;
            job.completed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            match result {
                Ok(status) => {
                    job.returncode = status.code();
                    if cancelling {
                        job.status = JobStatus::Cancelled;
                    } else if read_report(&self.runs_dir.join(&id))
                        .ok()
                        .flatten()
                        .is_some()
                    {
                        job.status = JobStatus::Completed;
                    } else {
                        job.status = JobStatus::Failed;
                        job.error =
                            "E2E runner did not produce results.json; inspect the log".into();
                    }
                }
                Err(error) => {
                    job.status = JobStatus::Failed;
                    job.error = format!("cannot read E2E runner status: {error}");
                }
            }
            if let Err(error) = write_metadata(&self.runs_dir.join(&id), job) {
                tracing::error!(%error, %id, "write local E2E metadata");
            }
            return;
        }
    }
}

fn build_run_command(executable: &Path, request: &RunRequest, output_dir: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .arg("run")
        .arg("--url")
        .arg(&request.url)
        .arg("--model")
        .arg(&request.model)
        .arg("--provider")
        .arg(&request.provider)
        .arg("--output")
        .arg(output_dir)
        .arg("--runs")
        .arg(request.runs.to_string())
        .arg("--technical-retries")
        .arg(request.technical_retries.to_string());
    if !request.judge_model.is_empty() {
        command.arg("--judge-model").arg(&request.judge_model);
    }
    if !request.judge_provider.is_empty() {
        command.arg("--judge-provider").arg(&request.judge_provider);
    }
    for scenario in &request.scenarios {
        command.arg("--scenario").arg(scenario);
    }
    command
}

fn validate_request(request: &mut RunRequest) -> std::result::Result<(), String> {
    request.label = request.label.trim().to_string();
    request.url = request.url.trim().to_string();
    request.model = request.model.trim().to_string();
    request.provider = request.provider.trim().to_string();
    request.judge_model = request.judge_model.trim().to_string();
    request.judge_provider = request.judge_provider.trim().to_string();
    validate_stack_url(&request.url).map_err(|error| error.to_string())?;
    if request.label.len() > 120 || request.label.chars().any(char::is_control) {
        return Err("label is invalid".into());
    }
    for (name, value) in [("model", &request.model), ("provider", &request.provider)] {
        if value.is_empty() || value.len() > 200 || value.chars().any(char::is_control) {
            return Err(format!("{name} is invalid"));
        }
    }
    if !(1..=20).contains(&request.runs) {
        return Err("runs must be between 1 and 20".into());
    }
    if request.technical_retries > 3 {
        return Err("technical_retries must be between 0 and 3".into());
    }
    if request.scenarios.is_empty() || request.scenarios.len() > ScenarioId::ALL.len() {
        return Err("select at least one valid scenario".into());
    }
    let valid: BTreeMap<_, _> = ScenarioId::ALL
        .iter()
        .map(|value| (value.as_str(), ()))
        .collect();
    request.scenarios.sort();
    request.scenarios.dedup();
    if request
        .scenarios
        .iter()
        .any(|value| !valid.contains_key(value.as_str()))
    {
        return Err("request contains an unknown scenario".into());
    }
    Ok(())
}

fn validate_stack_url(value: &str) -> Result<()> {
    let parsed = Url::parse(value).context("url must be a ws:// or wss:// endpoint")?;
    if !matches!(parsed.scheme(), "ws" | "wss") || parsed.host_str().is_none() {
        bail!("url must be a ws:// or wss:// endpoint");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        bail!("url must not contain credentials");
    }
    Ok(())
}

fn write_metadata(run_dir: &Path, metadata: &RunMetadata) -> Result<()> {
    fs::create_dir_all(run_dir).with_context(|| format!("create {}", run_dir.display()))?;
    let target = run_dir.join("metadata.json");
    let temporary = run_dir.join("metadata.json.tmp");
    let mut bytes = serde_json::to_vec_pretty(metadata)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes).with_context(|| format!("write {}", temporary.display()))?;
    fs::rename(&temporary, &target).with_context(|| format!("replace {}", target.display()))?;
    Ok(())
}

fn read_metadata(run_dir: &Path) -> Result<Option<RunMetadata>> {
    let path = run_dir.join("metadata.json");
    if !path.is_file() {
        return Ok(None);
    }
    let value: RunMetadata = serde_json::from_slice(&fs::read(&path)?)
        .with_context(|| format!("decode {}", path.display()))?;
    if value.schema_version != LOCAL_SCHEMA_VERSION {
        bail!(
            "unsupported local run schema {} in {}; expected {}",
            value.schema_version,
            path.display(),
            LOCAL_SCHEMA_VERSION
        );
    }
    Ok(Some(value))
}

fn read_report(run_dir: &Path) -> Result<Option<E2eReport>> {
    let path = run_dir.join("results/results.json");
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(E2eReport::read_from(&path)?.0))
}

fn recover_interrupted_runs(runs_dir: &Path) -> Result<()> {
    for entry in fs::read_dir(runs_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(mut metadata) = read_metadata(&entry.path())? else {
            continue;
        };
        if metadata.status.active() {
            metadata.status = JobStatus::Failed;
            metadata.completed_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            metadata.error = "dashboard stopped before the runner completed".into();
            write_metadata(&entry.path(), &metadata)?;
        }
    }
    Ok(())
}

fn read_log_tail(path: &Path) -> Result<String> {
    if !path.is_file() {
        return Ok(String::new());
    }
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(MAX_LOG_TAIL_BYTES)))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn load_execution_summaries(runs_dir: &Path) -> Result<Vec<Value>> {
    let mut values = Vec::new();
    for entry in fs::read_dir(runs_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(metadata) = read_metadata(&entry.path())? else {
            continue;
        };
        let report = read_report(&entry.path())?;
        values.push(execution_summary(&metadata, report.as_ref())?);
    }
    values.sort_by(|left, right| {
        right
            .get("started_at")
            .and_then(Value::as_str)
            .cmp(&left.get("started_at").and_then(Value::as_str))
    });
    values.truncate(MAX_EXECUTIONS);
    Ok(values)
}

fn execution_summary(metadata: &RunMetadata, report: Option<&E2eReport>) -> Result<Value> {
    let generated_at = if metadata.completed_at.is_empty() {
        &metadata.started_at
    } else {
        &metadata.completed_at
    };
    let execution = execution_identity(metadata);
    let Some(report) = report else {
        let status = match metadata.status {
            JobStatus::Cancelled => "cancelled",
            JobStatus::Running | JobStatus::Cancelling => "running",
            JobStatus::Completed => "incomplete",
            JobStatus::Failed => "infra_failed",
        };
        return Ok(json!({
            "id": metadata.id,
            "label": metadata.label,
            "run_id": metadata.id,
            "attempt": 1,
            "workflow_name": "Harness E2E Local",
            "workflow_url": "",
            "event": "local",
            "actor": actor(),
            "started_at": metadata.started_at,
            "completed_at": metadata.completed_at,
            "conclusion": if metadata.status == JobStatus::Failed { "failure" } else { "" },
            "status": status,
            "availability": "unavailable",
            "detail_path": null,
            "generated_at": generated_at,
            "lane": "local",
            "execution": execution,
            "requested_runs": metadata.request.runs,
            "subjects": [],
            "scenario_metrics": [],
            "totals": {},
            "first_failure": if metadata.error.is_empty() { Value::Null } else { json!({"kind":"runner", "message": metadata.error}) },
        }));
    };

    let subject_id = slug(&format!(
        "{}-{}",
        report.subject.provider, report.subject.model
    ));
    let scenarios: Vec<_> = report.scenarios.iter().map(scenario_summary).collect();
    let hard_gate_failures: u32 = report
        .scenarios
        .iter()
        .map(|value| value.aggregate.hard_gate_failures)
        .sum();
    let technical_failures: u32 = report
        .scenarios
        .iter()
        .map(|value| value.aggregate.technical_failures)
        .sum();
    let retries: usize = report
        .scenarios
        .iter()
        .flat_map(|scenario| &scenario.runs)
        .map(|run| run.retry_attempts.len())
        .sum();
    let wall_time_seconds = report
        .scenarios
        .iter()
        .flat_map(|scenario| &scenario.runs)
        .map(|run| run.wall_time_ms as f64 / 1000.0)
        .sum::<f64>();
    let costs: Vec<_> = report
        .scenarios
        .iter()
        .map(|scenario| scenario.aggregate.cost.total_usd)
        .collect();
    let total_cost_usd = sum_complete(&costs);
    let scores: Vec<_> = report
        .scenarios
        .iter()
        .filter_map(|scenario| scenario.aggregate.median_score)
        .collect();
    let average_score = mean(&scores);
    let expected = report.scenarios.len();
    let passed = report
        .scenarios
        .iter()
        .filter(|scenario| scenario.passed)
        .count();
    let status = semantic_status(report.passed, hard_gate_failures, technical_failures);
    let totals = efficiency_totals(report);
    let subject = json!({
        "id": subject_id,
        "model": report.subject.model,
        "provider": report.subject.provider,
        "judge": report.judge,
        "engine_revision": report.engine_revision,
        "passed": report.passed,
        "expected_reports": expected,
        "received_reports": expected,
        "scenario_pass_rate": if expected == 0 { 0.0 } else { passed as f64 / expected as f64 },
        "report_coverage": 1.0,
        "hard_gate_failures": hard_gate_failures,
        "technical_failures": technical_failures,
        "infra_failures": 0,
        "retry_attempts": retries,
        "total_cost_usd": total_cost_usd,
        "wall_time_seconds": wall_time_seconds,
        "scenarios": scenarios,
    });
    Ok(json!({
        "id": metadata.id,
        "label": metadata.label,
        "run_id": metadata.id,
        "attempt": 1,
        "workflow_name": "Harness E2E Local",
        "workflow_url": "",
        "event": "local",
        "actor": actor(),
        "started_at": metadata.started_at,
        "completed_at": metadata.completed_at,
        "conclusion": if hard_gate_failures > 0 || technical_failures > 0 { "failure" } else { "success" },
        "status": status,
        "availability": "full",
        "detail_path": format!("runs/{}.json", metadata.id),
        "generated_at": generated_at,
        "lane": "local",
        "execution": execution,
        "release": { "tag":"", "worker":"", "version":"", "url":"", "registry_tag":"local" },
        "source": { "sha":"", "ref":"local", "repository":"" },
        "requested_runs": metadata.request.runs,
        "subjects": [subject],
        "scenario_metrics": scenario_metrics(&subject_id, report),
        "totals": {
            "expected_reports": expected,
            "received_reports": expected,
            "report_coverage": 100.0,
            "passed_scenarios": passed,
            "scenario_pass_rate": if expected == 0 { 0.0 } else { passed as f64 / expected as f64 * 100.0 },
            "average_score": average_score,
            "total_cost_usd": total_cost_usd,
            "wall_time_seconds": wall_time_seconds,
            "hard_gate_failures": hard_gate_failures,
            "technical_failures": technical_failures,
            "missing_reports": 0,
            "retries": retries,
            "total_tokens": totals.0,
            "function_calls": totals.1,
        },
        "workflow_duration_seconds": wall_time_seconds,
        "first_failure": first_failure(report),
    }))
}

fn execution_detail_value(metadata: &RunMetadata, report: &E2eReport) -> Result<Value> {
    let summary = execution_summary(metadata, Some(report))?;
    let subject_id = slug(&format!(
        "{}-{}",
        report.subject.provider, report.subject.model
    ));
    let base = serde_json::to_value(report)?;
    let reports: Vec<_> = report
        .scenarios
        .iter()
        .map(|scenario| {
            let mut value = base.clone();
            value["passed"] = json!(scenario.passed);
            value["scenarios"] = json!([scenario]);
            json!({
                "subject_id": subject_id,
                "scenario_id": scenario.scenario_id,
                "available": true,
                "report": value,
            })
        })
        .collect();
    let mut detail = summary;
    detail["reports"] = json!(reports);
    Ok(detail)
}

fn scenario_summary(scenario: &E2eScenarioReport) -> Value {
    let wall_time_seconds = scenario
        .runs
        .iter()
        .map(|run| run.wall_time_ms as f64 / 1000.0)
        .sum::<f64>();
    let retries: usize = scenario
        .runs
        .iter()
        .map(|run| run.retry_attempts.len())
        .sum();
    json!({
        "id": scenario.scenario_id,
        "status": semantic_status(scenario.passed, scenario.aggregate.hard_gate_failures, scenario.aggregate.technical_failures),
        "passed": scenario.passed,
        "threshold": scenario.threshold,
        "runs": scenario.aggregate.runs,
        "median_score": scenario.aggregate.median_score,
        "pass_rate": scenario.aggregate.pass_rate,
        "hard_gate_failures": scenario.aggregate.hard_gate_failures,
        "technical_failures": scenario.aggregate.technical_failures,
        "infra_failures": 0,
        "retries": retries,
        "total_cost_usd": scenario.aggregate.cost.total_usd,
        "wall_time_seconds": wall_time_seconds,
    })
}

fn semantic_status(passed: bool, hard_gates: u32, technical: u32) -> &'static str {
    if technical > 0 {
        "technical_failed"
    } else if hard_gates > 0 {
        "hard_gate_failed"
    } else if passed {
        "passed"
    } else {
        "quality_advisory"
    }
}

fn scenario_metrics(subject_id: &str, report: &E2eReport) -> Vec<Value> {
    report
        .scenarios
        .iter()
        .map(|scenario| {
            let metric = |run: &E2eRunReport, name: &str| -> Option<f64> {
                match name {
                    "tokens" => run.metrics.as_ref().and_then(|value| {
                        value
                            .totals
                            .input_tokens
                            .zip(value.totals.output_tokens)
                            .map(|(input, output)| (input + output) as f64)
                    }),
                    "duration_seconds" => Some(run.wall_time_ms as f64 / 1000.0),
                    "cost_usd" => run.cost.total_usd,
                    "function_calls" => run
                        .metrics
                        .as_ref()
                        .map(|value| value.totals.function_calls as f64),
                    "function_call_errors" => run
                        .metrics
                        .as_ref()
                        .map(|value| value.totals.function_call_errors as f64),
                    "sessions" => run
                        .metrics
                        .as_ref()
                        .map(|value| value.totals.sessions as f64),
                    "turns" => run.metrics.as_ref().map(|value| value.totals.turns as f64),
                    _ => None,
                }
            };
            let mut averages = serde_json::Map::new();
            let mut samples = serde_json::Map::new();
            for name in [
                "tokens",
                "duration_seconds",
                "cost_usd",
                "function_calls",
                "function_call_errors",
                "sessions",
                "turns",
            ] {
                let values: Vec<_> = scenario
                    .runs
                    .iter()
                    .filter_map(|run| metric(run, name))
                    .collect();
                averages.insert(name.into(), json!(mean(&values)));
                samples.insert(name.into(), json!(values.len()));
            }
            let contract = json!({
                "execution_policy": scenario.execution_policy,
                "scenario_id": scenario.scenario_id,
                "scenario_version": scenario.scenario_version,
                "threshold": scenario.threshold,
            });
            json!({
                "subject_id": subject_id,
                "scenario_id": scenario.scenario_id,
                "scenario_version": scenario.scenario_version,
                "contract_fingerprint": contract_fingerprint(&contract),
                "run_count": scenario.runs.len(),
                "averages": averages,
                "samples": samples,
            })
        })
        .collect()
}

fn contract_fingerprint(value: &Value) -> String {
    let bytes = serde_json::to_vec(value).expect("serialize scenario contract");
    let hash = bytes.into_iter().fold(2_166_136_261_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
    });
    format!("fnv1a32:{hash:08x}")
}

fn efficiency_totals(report: &E2eReport) -> (Option<f64>, Option<f64>) {
    let mut tokens = Vec::new();
    let mut calls = Vec::new();
    for run in report.scenarios.iter().flat_map(|scenario| &scenario.runs) {
        if let Some(metrics) = &run.metrics {
            if let Some((input, output)) = metrics
                .totals
                .input_tokens
                .zip(metrics.totals.output_tokens)
            {
                tokens.push((input + output) as f64);
            }
            calls.push(metrics.totals.function_calls as f64);
        }
    }
    (sum_available(&tokens), sum_available(&calls))
}

fn first_failure(report: &E2eReport) -> Value {
    for scenario in &report.scenarios {
        for run in &scenario.runs {
            if let Some(failure) = run.failures.first() {
                return json!({
                    "kind": "run_failure",
                    "scenario_id": scenario.scenario_id,
                    "phase": failure.phase,
                    "message": failure.message,
                });
            }
            if let Some(gate) = run.hard_gates.iter().find(|gate| !gate.passed) {
                return json!({
                    "kind": "hard_gate",
                    "scenario_id": scenario.scenario_id,
                    "message": format!("{}: {}", gate.id, gate.reason),
                });
            }
        }
    }
    Value::Null
}

fn execution_identity(metadata: &RunMetadata) -> Value {
    json!({
        "id": metadata.id,
        "run_id": metadata.id,
        "attempt": 1,
        "event": "local",
        "actor": actor(),
        "workflow_name": "Harness E2E Local",
        "workflow_url": "",
        "label": metadata.label,
        "started_at": metadata.started_at,
        "completed_at": metadata.completed_at,
        "conclusion": if metadata.status == JobStatus::Failed { "failure" } else { "success" },
        "head_sha": "",
        "head_branch": "local",
        "repository": "",
    })
}

fn validate_execution_id(value: &str) -> std::result::Result<(), String> {
    if value.starts_with("local-")
        && value.len() <= 80
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        Ok(())
    } else {
        Err("invalid execution id".into())
    }
}

fn sum_complete(values: &[Option<f64>]) -> Option<f64> {
    values
        .iter()
        .copied()
        .try_fold(0.0, |total, value| Some(total + value?))
}

fn sum_available(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum())
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn slug(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn actor() -> String {
    env::var("USER").unwrap_or_else(|_| "local".into())
}

fn repository_url() -> String {
    "https://github.com/iii-hq/workers".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{CostReport, E2eScenarioReport, ModelArtifact, RunStatus};
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
