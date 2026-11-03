use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::runtime::FunctionRef;
use iii_sdk::{
    Deliver, IIIClient, ObserverHandle, ObserverOptions, ObserverState, RegisterFunction, Source,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::process::Command;

use crate::functions::ON_SCHEDULE_ID;
use crate::{
    IiiRuntime, RepositoryConfigV1, RepositoryScheduleV1, SecurityRuntime, SecurityScanError,
    SecurityScanRequestV1, SecurityScanScheduleEventV1, SecurityScanScheduleResponseV1,
    SecurityScanService, WorkerConfig,
};

/// The cron worker's tick function, observed with args `{ expression }`.
pub const ON_TICK_FN: &str = "cron::on-tick";

const GIT_RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);

/// Keeps the static handler and one `cron::on-tick` observer per scheduled
/// repository until worker shutdown. An observer stays `pending` while the
/// cron worker is not running and becomes active once it registers its
/// function, so start order does not matter.
pub struct ScheduleHandles {
    _function: FunctionRef,
    observers: HashMap<String, ObserverHandle>,
}

impl ScheduleHandles {
    pub fn schedule_count(&self) -> usize {
        self.observers.len()
    }

    /// The observer of one configured repository's schedule.
    pub fn observer(&self, repository: &str) -> Option<&ObserverHandle> {
        self.observers.get(repository)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduleMetadataV1 {
    repository: String,
}

pub fn register(
    iii: &Arc<IIIClient>,
    service: Arc<SecurityScanService<IiiRuntime>>,
    config: Arc<WorkerConfig>,
) -> ScheduleHandles {
    let service_for_handler = service.clone();
    let config_for_handler = config.clone();
    let function = iii.register_function(
        ON_SCHEDULE_ID,
        RegisterFunction::new_async(
            move |_event: SecurityScanScheduleEventV1, metadata: Option<Value>| {
                let service = service_for_handler.clone();
                let config = config_for_handler.clone();
                async move {
                    handle_schedule(&service, &config, metadata)
                        .await
                        .map_err(Into::into)
                }
            },
        )
        .description(crate::functions::ON_SCHEDULE_DESC)
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    let mut observers = HashMap::new();
    for repository in &config.repositories {
        let Some(schedule) = repository.schedule.as_ref() else {
            continue;
        };
        let handle = observe_schedule(iii, repository, schedule);
        let (id, expression) = (repository.id.clone(), schedule.expression.clone());
        let _ = handle.on_state_change(move |state, reason| match state {
            ObserverState::Active => tracing::info!(
                repository = %id,
                %expression,
                "observing UTC security scan schedule"
            ),
            ObserverState::Error => tracing::error!(
                repository = %id,
                %expression,
                reason = ?reason,
                "cron refused the security scan schedule; manual scanning remains active"
            ),
            _ => {}
        });
        observers.insert(repository.id.clone(), handle);
    }

    ScheduleHandles {
        _function: function,
        observers,
    }
}

/// One `cron::on-tick` observer for a repository's schedule: the cron
/// expression is the arg, the repository id rides as observer metadata (the
/// target's metadata sidecar), and the target takes the tick as output.
fn observe_schedule(
    iii: &IIIClient,
    repository: &RepositoryConfigV1,
    schedule: &RepositoryScheduleV1,
) -> ObserverHandle {
    iii.observe_with(
        schedule_source(schedule),
        ON_SCHEDULE_ID,
        ObserverOptions::default().metadata(schedule_metadata(repository)),
    )
}

fn schedule_source(schedule: &RepositoryScheduleV1) -> Source {
    Source::function(ON_TICK_FN)
        .args(json!({ "expression": schedule.expression }))
        .deliver(Deliver::Output)
}

fn schedule_metadata(repository: &RepositoryConfigV1) -> Value {
    json!({ "repository": repository.id })
}

async fn handle_schedule(
    service: &SecurityScanService<IiiRuntime>,
    config: &WorkerConfig,
    metadata: Option<Value>,
) -> Result<SecurityScanScheduleResponseV1, SecurityScanError> {
    let (repository, schedule) = schedule_from_metadata(config, metadata)?;
    let target_sha = resolve_target_sha(repository, &schedule.target_ref).await?;
    request_resolved_schedule(service, &repository.id, schedule, target_sha).await
}

fn schedule_from_metadata(
    config: &WorkerConfig,
    metadata: Option<Value>,
) -> Result<(&RepositoryConfigV1, &RepositoryScheduleV1), SecurityScanError> {
    let metadata: ScheduleMetadataV1 = serde_json::from_value(metadata.ok_or_else(|| {
        SecurityScanError::InvalidRequest("schedule invocation metadata is missing".into())
    })?)
    .map_err(|error| {
        SecurityScanError::InvalidRequest(format!(
            "schedule invocation metadata is invalid: {error}"
        ))
    })?;
    let repository = config.repository(&metadata.repository).ok_or_else(|| {
        SecurityScanError::InvalidRequest(format!(
            "scheduled repository {} is not configured",
            metadata.repository
        ))
    })?;
    let schedule = repository.schedule.as_ref().ok_or_else(|| {
        SecurityScanError::InvalidRequest(format!(
            "repository {} has no configured schedule",
            repository.id
        ))
    })?;
    Ok((repository, schedule))
}

pub(crate) async fn resolve_target_sha(
    repository: &RepositoryConfigV1,
    target_ref: &str,
) -> Result<String, SecurityScanError> {
    let mut command = Command::new("git");
    command
        .current_dir(&repository.path)
        .args(["rev-parse", "--verify", "--quiet", "--end-of-options"])
        .arg(format!("{target_ref}^{{commit}}"))
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_NAMESPACE")
        .env_remove("GIT_CEILING_DIRECTORIES")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let output = tokio::time::timeout(GIT_RESOLVE_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            SecurityScanError::Dependency(format!(
                "resolving target_ref for repository {} timed out",
                repository.id
            ))
        })?
        .map_err(|error| {
            SecurityScanError::Dependency(format!(
                "could not run git rev-parse for repository {}: {error}",
                repository.id
            ))
        })?;
    if !output.status.success() {
        return Err(SecurityScanError::Dependency(format!(
            "target_ref {target_ref} did not resolve to a commit for repository {}",
            repository.id
        )));
    }
    parse_resolved_sha(&output.stdout)
}

fn parse_resolved_sha(stdout: &[u8]) -> Result<String, SecurityScanError> {
    let output = std::str::from_utf8(stdout).map_err(|_| {
        SecurityScanError::Dependency("git rev-parse returned non-UTF-8 output".into())
    })?;
    let sha = output
        .strip_suffix("\r\n")
        .or_else(|| output.strip_suffix('\n'))
        .unwrap_or(output);
    if sha.len() != 40
        || !sha
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(SecurityScanError::Dependency(
            "git rev-parse did not return exactly one lowercase 40-character commit SHA".into(),
        ));
    }
    Ok(sha.to_string())
}

async fn request_resolved_schedule<R: SecurityRuntime>(
    service: &SecurityScanService<R>,
    repository: &str,
    schedule: &RepositoryScheduleV1,
    target_sha: String,
) -> Result<SecurityScanScheduleResponseV1, SecurityScanError> {
    let response = service
        .request(SecurityScanRequestV1::new(
            repository.to_string(),
            target_sha.clone(),
            schedule.mode,
        ))
        .await?;
    Ok(SecurityScanScheduleResponseV1 {
        repository: repository.to_string(),
        target_sha,
        mode: schedule.mode,
        run_id: response.run_id,
        status: response.status,
        deduplicated: response.deduplicated,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;
    use crate::{AnalysisConfigV1, CreateRunOutcome, EnqueueRequest, RunRecordV1, ScanModeV1};

    fn config_with_schedule() -> WorkerConfig {
        WorkerConfig {
            repositories: vec![RepositoryConfigV1 {
                id: "iii-hq/iii".into(),
                path: "/srv/repos/iii".into(),
                github: None,
                schedule: Some(RepositoryScheduleV1 {
                    expression: "0 0 3 * * *".into(),
                    target_ref: "refs/heads/main".into(),
                    mode: ScanModeV1::Scan,
                }),
            }],
            analysis: AnalysisConfigV1 {
                model: "security-review-model".into(),
                provider: None,
                max_turns: 4,
                max_output_tokens: 8_000,
                max_total_tokens: 50_000,
                max_cost_usd: Some(2.0),
            },
            archive: None,
        }
    }

    #[test]
    fn parses_exact_lowercase_full_sha_only() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            parse_resolved_sha(format!("{sha}\n").as_bytes()).unwrap(),
            sha
        );
        assert_eq!(
            parse_resolved_sha(format!("{sha}\r\n").as_bytes()).unwrap(),
            sha
        );
        assert!(parse_resolved_sha(sha.to_ascii_uppercase().as_bytes()).is_err());
        assert!(parse_resolved_sha(b"0123456789abcdef").is_err());
        assert!(parse_resolved_sha(format!("{sha}\n{sha}\n").as_bytes()).is_err());
        assert!(parse_resolved_sha(format!("{sha}\n\n").as_bytes()).is_err());
        assert!(parse_resolved_sha(&[0xff; 40]).is_err());
    }

    #[test]
    fn metadata_is_only_a_repository_lookup_key() {
        let config = config_with_schedule();
        let (repository, schedule) =
            schedule_from_metadata(&config, Some(json!({ "repository": "iii-hq/iii" }))).unwrap();
        assert_eq!(repository.path, "/srv/repos/iii");
        assert_eq!(schedule.target_ref, "refs/heads/main");

        assert!(schedule_from_metadata(&config, None).is_err());
        assert!(schedule_from_metadata(&config, Some(json!({ "repository": "unknown" }))).is_err());
        assert!(schedule_from_metadata(
            &config,
            Some(json!({
                "repository": "iii-hq/iii",
                "target_ref": "refs/heads/attacker"
            }))
        )
        .is_err());
    }

    #[test]
    fn a_schedule_observes_cron_with_its_expression_and_repository_metadata() {
        let config = config_with_schedule();
        let repository = &config.repositories[0];
        let schedule = repository.schedule.as_ref().unwrap();

        assert_eq!(
            schedule_source(schedule),
            Source::function("cron::on-tick")
                .args(json!({ "expression": "0 0 3 * * *" }))
                .deliver(Deliver::Output)
        );
        assert_eq!(
            schedule_metadata(repository),
            json!({ "repository": "iii-hq/iii" })
        );
    }

    #[tokio::test]
    async fn every_scheduled_repository_gets_one_pending_observer_without_cron() {
        // `IIIClient::new` builds local state only: observers stay pending
        // until a connection and the cron worker's function exist.
        let iii = Arc::new(IIIClient::new("ws://127.0.0.1:1"));
        let mut config = config_with_schedule();
        config.repositories.push(RepositoryConfigV1 {
            id: "iii-hq/manual".into(),
            path: "/srv/repos/manual".into(),
            github: None,
            schedule: None,
        });
        let config = Arc::new(config);
        let service = Arc::new(SecurityScanService::new(
            Arc::new(IiiRuntime::new(iii.clone())),
            config.as_ref().clone(),
        ));

        let handles = register(&iii, service, config);

        assert_eq!(handles.schedule_count(), 1);
        let observer = handles
            .observer("iii-hq/iii")
            .expect("scheduled repository");
        assert_eq!(observer.state(), ObserverState::Pending);
        assert!(handles.observer("iii-hq/manual").is_none());
    }

    #[derive(Default)]
    struct MemoryRuntime {
        run: Mutex<Option<RunRecordV1>>,
        enqueued: Mutex<Vec<EnqueueRequest>>,
    }

    #[async_trait]
    impl SecurityRuntime for MemoryRuntime {
        fn require_ready(&self) -> Result<(), SecurityScanError> {
            Ok(())
        }

        async fn get_run(&self, run_id: &str) -> Result<Option<RunRecordV1>, SecurityScanError> {
            Ok(self
                .run
                .lock()
                .expect("run lock")
                .clone()
                .filter(|run| run.run_id == run_id))
        }

        async fn list_run_summaries(
            &self,
        ) -> Result<Vec<crate::PublicRunSummaryV1>, SecurityScanError> {
            Err(unused_capability())
        }

        async fn get_reconciliation_snapshot(
            &self,
            _run_id: &str,
        ) -> Result<Option<crate::ReconciliationSnapshotV1>, SecurityScanError> {
            Err(unused_capability())
        }

        async fn save_reconciliation_snapshot(
            &self,
            _snapshot: crate::ReconciliationSnapshotV1,
        ) -> Result<(), SecurityScanError> {
            Err(unused_capability())
        }

        async fn collect_reconciliation_source(
            &self,
            _source: crate::ReconciliationSourceV1,
            _github_full_name: &str,
            _target_sha: &str,
            _collected_at: i64,
        ) -> Result<crate::ReconciliationSourceCollectionV1, SecurityScanError> {
            Err(unused_capability())
        }

        async fn create_run_if_absent(
            &self,
            run: RunRecordV1,
        ) -> Result<CreateRunOutcome, SecurityScanError> {
            let mut current = self.run.lock().expect("run lock");
            if let Some(existing) = current.as_ref() {
                return Ok(CreateRunOutcome::Existing(Box::new(existing.clone())));
            }
            *current = Some(run);
            Ok(CreateRunOutcome::Created)
        }

        async fn replace_run(
            &self,
            _expected: &RunRecordV1,
            _replacement: RunRecordV1,
        ) -> Result<bool, SecurityScanError> {
            Ok(false)
        }

        async fn delete_run_if_unchanged(
            &self,
            _run: &RunRecordV1,
        ) -> Result<(), SecurityScanError> {
            Ok(())
        }

        async fn enqueue_execute(&self, request: EnqueueRequest) -> Result<(), SecurityScanError> {
            self.enqueued.lock().expect("enqueue lock").push(request);
            Ok(())
        }

        async fn stop_analysis(
            &self,
            _harness: &crate::HarnessRunV1,
        ) -> Result<(), SecurityScanError> {
            Err(unused_capability())
        }

        async fn ensure_analysis_chat_link(
            &self,
            _run: &RunRecordV1,
        ) -> Result<bool, SecurityScanError> {
            Err(unused_capability())
        }

        async fn get_action(
            &self,
            _action_id: &str,
        ) -> Result<Option<crate::SecurityActionRecordV1>, SecurityScanError> {
            Err(unused_capability())
        }

        async fn list_actions(
            &self,
        ) -> Result<Vec<crate::SecurityActionRecordV1>, SecurityScanError> {
            Err(unused_capability())
        }

        async fn create_action_if_absent(
            &self,
            _action: crate::SecurityActionRecordV1,
        ) -> Result<crate::CreateActionOutcome, SecurityScanError> {
            Err(unused_capability())
        }

        async fn replace_action(
            &self,
            _expected: &crate::SecurityActionRecordV1,
            _replacement: crate::SecurityActionRecordV1,
        ) -> Result<bool, SecurityScanError> {
            Err(unused_capability())
        }

        async fn delete_action_if_unchanged(
            &self,
            _action: &crate::SecurityActionRecordV1,
        ) -> Result<(), SecurityScanError> {
            Err(unused_capability())
        }

        async fn enqueue_action_execute(
            &self,
            _request: crate::ActionEnqueueRequestV1,
        ) -> Result<(), SecurityScanError> {
            Err(unused_capability())
        }

        async fn approval_gate_is_live(&self) -> Result<bool, SecurityScanError> {
            Err(unused_capability())
        }
    }

    fn unused_capability() -> SecurityScanError {
        SecurityScanError::Dependency("unused schedule test runtime capability".into())
    }

    #[tokio::test]
    async fn scheduled_requests_use_the_service_dedupe_path() {
        let config = config_with_schedule();
        let runtime = Arc::new(MemoryRuntime::default());
        let service = SecurityScanService::new(runtime.clone(), config.clone());
        let (_, schedule) =
            schedule_from_metadata(&config, Some(json!({ "repository": "iii-hq/iii" }))).unwrap();
        let sha = "0123456789abcdef0123456789abcdef01234567".to_string();

        let first = request_resolved_schedule(&service, "iii-hq/iii", schedule, sha.clone())
            .await
            .unwrap();
        let duplicate = request_resolved_schedule(&service, "iii-hq/iii", schedule, sha)
            .await
            .unwrap();

        assert!(!first.deduplicated);
        assert!(duplicate.deduplicated);
        assert_eq!(first.run_id, duplicate.run_id);
        assert_eq!(runtime.enqueued.lock().expect("enqueue lock").len(), 1);
    }
}
