use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use chrono::{SecondsFormat, Utc};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use url::Url;

use super::store::{read_report, recover_interrupted_runs, write_metadata};
use super::{
    ApiError, DashboardArgs, Defaults, JobStatus, JobView, RunMetadata, RunRequest, RunSnapshot,
    LOCAL_SCHEMA_VERSION,
};
use crate::scenarios::ScenarioId;

const MAX_LOG_TAIL_BYTES: u64 = 256 * 1024;

struct ControllerState {
    job: Option<RunMetadata>,
    child: Option<Child>,
}

pub(super) struct Controller {
    runs_dir: PathBuf,
    executable: PathBuf,
    defaults: Defaults,
    state: Mutex<ControllerState>,
}

impl Controller {
    pub(super) fn new(args: DashboardArgs) -> Result<Arc<Self>> {
        validate_stack_url(&args.url)?;
        fs::create_dir_all(&args.runs_dir)
            .with_context(|| format!("create {}", args.runs_dir.display()))?;
        recover_interrupted_runs(&args.runs_dir)?;
        Ok(Arc::new(Self {
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
        }))
    }

    pub(super) fn runs_dir(&self) -> &Path {
        &self.runs_dir
    }

    pub(super) fn default_url(&self) -> &str {
        &self.defaults.url
    }

    pub(super) async fn snapshot(&self) -> Result<RunSnapshot> {
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

    pub(super) async fn start(self: &Arc<Self>, mut request: RunRequest) -> Result<(), ApiError> {
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

    pub(super) async fn cancel(&self) -> Result<(), ApiError> {
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

pub(super) fn build_run_command(
    executable: &Path,
    request: &RunRequest,
    output_dir: &Path,
) -> Command {
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

pub(super) fn validate_request(request: &mut RunRequest) -> std::result::Result<(), String> {
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

pub(super) fn validate_stack_url(value: &str) -> Result<()> {
    let parsed = Url::parse(value).context("url must be a ws:// or wss:// endpoint")?;
    if !matches!(parsed.scheme(), "ws" | "wss") || parsed.host_str().is_none() {
        bail!("url must be a ws:// or wss:// endpoint");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        bail!("url must not contain credentials");
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
