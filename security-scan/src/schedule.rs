use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::runtime::FunctionRef;
use iii_sdk::trigger::Trigger;
use iii_sdk::{IIIClient, RegisterFunction};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::process::Command;

use crate::functions::ON_SCHEDULE_ID;
use crate::{
    IiiRuntime, RepositoryConfigV1, RepositoryScheduleV1, SecurityRuntime, SecurityScanError,
    SecurityScanRequestV1, SecurityScanScheduleEventV1, SecurityScanScheduleResponseV1,
    SecurityScanService, WorkerConfig,
};

const GIT_RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);
const CRON_PROBE_ATTEMPTS: u32 = 4;
const CRON_PROBE_BACKOFF: Duration = Duration::from_millis(250);

/// Keeps the static handler and every successful cron binding registered until
/// worker shutdown, while allowing missing bindings to recover later.
pub struct ScheduleHandles {
    _function: FunctionRef,
    iii: Arc<IIIClient>,
    config: Arc<WorkerConfig>,
    triggers: HashMap<String, Trigger>,
}

impl ScheduleHandles {
    pub fn bound_schedule_count(&self) -> usize {
        self.triggers.len()
    }

    pub async fn recover_bindings(&mut self) {
        let pending = pending_schedule_indices(&self.config, |repository| {
            self.triggers.contains_key(repository)
        });
        if pending.is_empty() {
            return;
        }

        if !wait_for_cron_owner(&self.iii).await {
            tracing::warn!(
                configured = pending.len(),
                "cron trigger owner is unavailable; scheduled security scans will retry while manual scanning remains active"
            );
            return;
        }

        for index in pending {
            let repository = &self.config.repositories[index];
            let Some(schedule) = repository.schedule.as_ref() else {
                continue;
            };
            match self.iii.register_trigger(RegisterTriggerInput {
                trigger_type: "cron".into(),
                function_id: ON_SCHEDULE_ID.into(),
                config: json!({ "expression": schedule.expression }),
                metadata: Some(json!({ "repository": repository.id })),
                namespace: None,
                trigger_namespace: None,
            }) {
                Ok(trigger) => {
                    tracing::info!(
                        repository = %repository.id,
                        expression = %schedule.expression,
                        "bound UTC security scan schedule"
                    );
                    self.triggers.insert(repository.id.clone(), trigger);
                }
                Err(error) => tracing::error!(
                    repository = %repository.id,
                    %error,
                    "could not bind security scan schedule; recovery will retry while manual scanning remains active"
                ),
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduleMetadataV1 {
    repository: String,
}

pub async fn register(
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

    let mut handles = ScheduleHandles {
        _function: function,
        iii: iii.clone(),
        config,
        triggers: HashMap::new(),
    };
    handles.recover_bindings().await;
    handles
}

fn pending_schedule_indices(config: &WorkerConfig, is_bound: impl Fn(&str) -> bool) -> Vec<usize> {
    config
        .repositories
        .iter()
        .enumerate()
        .filter_map(|(index, repository)| {
            (repository.schedule.is_some() && !is_bound(&repository.id)).then_some(index)
        })
        .collect()
}

async fn wait_for_cron_owner(iii: &IIIClient) -> bool {
    for attempt in 1..=CRON_PROBE_ATTEMPTS {
        match cron_owner_available(iii).await {
            Ok(true) => return true,
            Ok(false) if attempt < CRON_PROBE_ATTEMPTS => {}
            Ok(false) => return false,
            Err(error) if attempt < CRON_PROBE_ATTEMPTS => {
                tracing::warn!(attempt, %error, "could not confirm cron trigger owner; retrying")
            }
            Err(error) => {
                tracing::error!(%error, "could not confirm cron trigger owner");
                return false;
            }
        }
        tokio::time::sleep(CRON_PROBE_BACKOFF * attempt).await;
    }
    false
}

async fn cron_owner_available(iii: &IIIClient) -> Result<bool, SecurityScanError> {
    let response = iii
        .trigger(TriggerRequest {
            function_id: "engine::triggers::list".into(),
            payload: json!({ "include_internal": true }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .map_err(|error| {
            SecurityScanError::Dependency(format!(
                "engine::triggers::list failed while probing cron: {error}"
            ))
        })?;
    Ok(response
        .get("triggers")
        .and_then(Value::as_array)
        .is_some_and(|triggers| {
            triggers
                .iter()
                .any(|trigger| trigger.get("id").and_then(Value::as_str) == Some("cron"))
        }))
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

async fn resolve_target_sha(
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
    use std::collections::HashSet;
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
    fn late_cron_recovery_retries_only_unbound_schedules() {
        let config = config_with_schedule();
        let mut bound = HashSet::new();

        assert_eq!(
            pending_schedule_indices(&config, |repository| bound.contains(repository)),
            vec![0]
        );
        // An unavailable owner or failed registration leaves the repository
        // pending for the next recovery pass.
        assert_eq!(
            pending_schedule_indices(&config, |repository| bound.contains(repository)),
            vec![0]
        );

        bound.insert("iii-hq/iii".to_string());
        assert!(
            pending_schedule_indices(&config, |repository| bound.contains(repository)).is_empty()
        );
    }

    #[derive(Default)]
    struct MemoryRuntime {
        run: Mutex<Option<RunRecordV1>>,
        enqueued: Mutex<Vec<EnqueueRequest>>,
    }

    #[async_trait]
    impl SecurityRuntime for MemoryRuntime {
        async fn get_run(&self, run_id: &str) -> Result<Option<RunRecordV1>, SecurityScanError> {
            Ok(self
                .run
                .lock()
                .expect("run lock")
                .clone()
                .filter(|run| run.run_id == run_id))
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
