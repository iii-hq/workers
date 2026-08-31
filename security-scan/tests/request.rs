use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use async_trait::async_trait;
use security_scan::{
    AnalysisConfigV1, CreateRunOutcome, EnqueueRequest, PublicRunSummaryV1, RepositoryConfigV1,
    RunErrorV1, RunRecordV1, RunStatusV1, ScanModeV1, SecurityAssessmentsV1, SecurityFindingV1,
    SecurityReportV1, SecurityRuntime, SecurityScanError, SecurityScanListRequestV1,
    SecurityScanReadRequestV1, SecurityScanRequestV1, SecurityScanService, SeverityV1,
    WorkerConfig,
};
use tokio::sync::Mutex;

#[test]
fn typed_inputs_accept_engine_metadata_without_loosening_unknown_field_checks() {
    let request: SecurityScanRequestV1 = serde_json::from_value(serde_json::json!({
        "repository": "iii-hq/iii",
        "target_sha": "0123456789abcdef0123456789abcdef01234567",
        "mode": "scan",
        "_caller_worker_id": "console"
    }))
    .unwrap();
    assert_eq!(request.repository, "iii-hq/iii");

    let read: SecurityScanReadRequestV1 = serde_json::from_value(serde_json::json!({
        "run_id": "sec_x",
        "_caller_worker_id": "console"
    }))
    .unwrap();
    assert_eq!(read.run_id, "sec_x");

    let list: SecurityScanListRequestV1 = serde_json::from_value(serde_json::json!({
        "repository": "iii-hq/iii",
        "status": "analyzing",
        "limit": 25,
        "_caller_worker_id": "console"
    }))
    .unwrap();
    assert_eq!(list.limit, Some(25));

    let execute: EnqueueRequest = serde_json::from_value(serde_json::json!({
        "run_id": "sec_x",
        "repository": "iii-hq/iii",
        "attempt": 1,
        "step": 0,
        "_caller_worker_id": "queue"
    }))
    .unwrap();
    assert_eq!(execute.step, 0);

    assert!(
        serde_json::from_value::<SecurityScanRequestV1>(serde_json::json!({
            "repository": "iii-hq/iii",
            "target_sha": "0123456789abcdef0123456789abcdef01234567",
            "mode": "scan",
            "unexpected": true
        }))
        .is_err()
    );

    let schema = serde_json::to_value(schemars::schema_for!(SecurityScanRequestV1)).unwrap();
    assert!(schema["properties"].get("_caller_worker_id").is_none());
    let schema = serde_json::to_value(schemars::schema_for!(SecurityScanListRequestV1)).unwrap();
    assert!(schema["properties"].get("_caller_worker_id").is_none());
}

#[derive(Default)]
struct FakeRuntime {
    run: Mutex<Option<RunRecordV1>>,
    listed_runs: Mutex<Vec<RunRecordV1>>,
    enqueued: Mutex<Vec<EnqueueRequest>>,
    fail_enqueue_once: AtomicBool,
}

fn service(runtime: Arc<FakeRuntime>) -> SecurityScanService<FakeRuntime> {
    SecurityScanService::new(
        runtime,
        WorkerConfig {
            repositories: vec![RepositoryConfigV1 {
                id: "iii-hq/iii".into(),
                path: "/srv/repos/iii".into(),
                github: None,
                schedule: None,
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
        },
    )
}

#[async_trait]
impl SecurityRuntime for FakeRuntime {
    fn require_ready(&self) -> Result<(), SecurityScanError> {
        Ok(())
    }

    async fn get_run(&self, _run_id: &str) -> Result<Option<RunRecordV1>, SecurityScanError> {
        Ok(self.run.lock().await.clone())
    }

    async fn list_run_summaries(&self) -> Result<Vec<PublicRunSummaryV1>, SecurityScanError> {
        let listed = self.listed_runs.lock().await.clone();
        if listed.is_empty() {
            Ok(self
                .run
                .lock()
                .await
                .as_ref()
                .map(PublicRunSummaryV1::from)
                .into_iter()
                .collect())
        } else {
            Ok(listed.iter().map(PublicRunSummaryV1::from).collect())
        }
    }

    async fn create_run_if_absent(
        &self,
        run: RunRecordV1,
    ) -> Result<CreateRunOutcome, SecurityScanError> {
        let mut stored = self.run.lock().await;
        if let Some(existing) = stored.clone() {
            return Ok(CreateRunOutcome::Existing(Box::new(existing)));
        }
        *stored = Some(run);
        Ok(CreateRunOutcome::Created)
    }

    async fn replace_run(
        &self,
        expected: &RunRecordV1,
        replacement: RunRecordV1,
    ) -> Result<bool, SecurityScanError> {
        let mut stored = self.run.lock().await;
        if stored.as_ref() != Some(expected) {
            return Ok(false);
        }
        *stored = Some(replacement);
        Ok(true)
    }

    async fn delete_run_if_unchanged(&self, run: &RunRecordV1) -> Result<(), SecurityScanError> {
        let mut stored = self.run.lock().await;
        if stored.as_ref() == Some(run) {
            *stored = None;
        }
        Ok(())
    }

    async fn enqueue_execute(&self, request: EnqueueRequest) -> Result<(), SecurityScanError> {
        if self.fail_enqueue_once.swap(false, Ordering::SeqCst) {
            return Err(SecurityScanError::Dependency("queue unavailable".into()));
        }
        self.enqueued.lock().await.push(request);
        Ok(())
    }

    async fn resolve_target_ref(
        &self,
        _repository: &RepositoryConfigV1,
        target_ref: &str,
    ) -> Result<String, SecurityScanError> {
        if target_ref != "HEAD" {
            return Err(SecurityScanError::Dependency(format!(
                "unexpected target ref {target_ref}"
            )));
        }
        Ok("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into())
    }

    async fn stop_analysis(
        &self,
        _harness: &security_scan::HarnessRunV1,
    ) -> Result<(), SecurityScanError> {
        Ok(())
    }

    async fn ensure_analysis_chat_link(
        &self,
        _run: &RunRecordV1,
    ) -> Result<bool, SecurityScanError> {
        unreachable!()
    }

    async fn get_action(
        &self,
        _action_id: &str,
    ) -> Result<Option<security_scan::SecurityActionRecordV1>, SecurityScanError> {
        unreachable!()
    }

    async fn list_actions(
        &self,
    ) -> Result<Vec<security_scan::SecurityActionRecordV1>, SecurityScanError> {
        unreachable!()
    }

    async fn create_action_if_absent(
        &self,
        _action: security_scan::SecurityActionRecordV1,
    ) -> Result<security_scan::CreateActionOutcome, SecurityScanError> {
        unreachable!()
    }

    async fn replace_action(
        &self,
        _expected: &security_scan::SecurityActionRecordV1,
        _replacement: security_scan::SecurityActionRecordV1,
    ) -> Result<bool, SecurityScanError> {
        unreachable!()
    }

    async fn delete_action_if_unchanged(
        &self,
        _action: &security_scan::SecurityActionRecordV1,
    ) -> Result<(), SecurityScanError> {
        unreachable!()
    }

    async fn enqueue_action_execute(
        &self,
        _request: security_scan::ActionEnqueueRequestV1,
    ) -> Result<(), SecurityScanError> {
        unreachable!()
    }

    async fn approval_gate_is_live(&self) -> Result<bool, SecurityScanError> {
        unreachable!()
    }
}

#[tokio::test]
async fn duplicate_manual_request_returns_the_same_run_and_enqueues_once() {
    let runtime = Arc::new(FakeRuntime::default());
    let service = service(runtime.clone());
    let request = SecurityScanRequestV1::new(
        "iii-hq/iii".into(),
        "0123456789abcdef0123456789abcdef01234567".into(),
        ScanModeV1::Suggest,
    );

    let first = service.request(request.clone()).await.unwrap();
    let second = service.request(request).await.unwrap();

    assert_eq!(first.run_id, second.run_id);
    assert!(!first.deduplicated);
    assert!(second.deduplicated);
    let enqueued = runtime.enqueued.lock().await;
    assert_eq!(enqueued.len(), 1);
    assert_eq!(enqueued[0].attempt, 1);
    assert_eq!(enqueued[0].step, 0);
}

#[tokio::test]
async fn request_stores_the_composer_model_and_does_not_inherit_the_operator_provider() {
    let runtime = Arc::new(FakeRuntime::default());
    let service = SecurityScanService::new(
        runtime.clone(),
        WorkerConfig {
            repositories: vec![RepositoryConfigV1 {
                id: "iii-hq/iii".into(),
                path: "/srv/repos/iii".into(),
                github: None,
                schedule: None,
            }],
            analysis: AnalysisConfigV1 {
                model: "codex/gpt-5.6-terra".into(),
                provider: Some("openai-codex".into()),
                max_turns: 4,
                max_output_tokens: 8_000,
                max_total_tokens: 50_000,
                max_cost_usd: Some(2.0),
            },
            archive: None,
        },
    );
    let mut request = SecurityScanRequestV1::new(
        "iii-hq/iii".into(),
        "0123456789abcdef0123456789abcdef01234567".into(),
        ScanModeV1::Scan,
    );
    request.model = Some("deepseek::deepseek-v4-flash".into());

    let response = service.request(request).await.unwrap();
    let stored = runtime.run.lock().await.clone().unwrap();
    assert!(!response.deduplicated);
    assert_eq!(stored.model.as_deref(), Some("deepseek::deepseek-v4-flash"));
    assert_eq!(stored.provider, None);
}

#[tokio::test]
async fn request_rejects_an_empty_model() {
    let runtime = Arc::new(FakeRuntime::default());
    let service = service(runtime.clone());
    let mut request = SecurityScanRequestV1::new(
        "iii-hq/iii".into(),
        "0123456789abcdef0123456789abcdef01234567".into(),
        ScanModeV1::Scan,
    );
    request.model = Some("   ".into());
    let error = service.request(request).await.unwrap_err();
    assert!(matches!(error, SecurityScanError::InvalidRequest(_)));
    assert!(runtime.run.lock().await.is_none());
}

#[tokio::test]
async fn request_without_a_model_uses_operator_analysis_routing() {
    let runtime = Arc::new(FakeRuntime::default());
    let service = SecurityScanService::new(
        runtime.clone(),
        WorkerConfig {
            repositories: vec![RepositoryConfigV1 {
                id: "iii-hq/iii".into(),
                path: "/srv/repos/iii".into(),
                github: None,
                schedule: None,
            }],
            analysis: AnalysisConfigV1 {
                model: "codex/gpt-5.6-terra".into(),
                provider: Some("openai-codex".into()),
                max_turns: 4,
                max_output_tokens: 8_000,
                max_total_tokens: 50_000,
                max_cost_usd: Some(2.0),
            },
            archive: None,
        },
    );
    let response = service
        .request(SecurityScanRequestV1::new(
            "iii-hq/iii".into(),
            "0123456789abcdef0123456789abcdef01234567".into(),
            ScanModeV1::Scan,
        ))
        .await
        .unwrap();
    let stored = runtime.run.lock().await.clone().unwrap();
    assert!(!response.deduplicated);
    assert_eq!(stored.model.as_deref(), Some("codex/gpt-5.6-terra"));
    assert_eq!(stored.provider.as_deref(), Some("openai-codex"));
}

#[tokio::test]
async fn distinct_composer_models_queue_distinct_runs() {
    let first_runtime = Arc::new(FakeRuntime::default());
    let second_runtime = Arc::new(FakeRuntime::default());
    let first = service(first_runtime);
    let second = service(second_runtime);
    let mut deepseek = SecurityScanRequestV1::new(
        "iii-hq/iii".into(),
        "0123456789abcdef0123456789abcdef01234567".into(),
        ScanModeV1::Scan,
    );
    deepseek.model = Some("deepseek::deepseek-v4-flash".into());
    let mut terra = deepseek.clone();
    terra.model = Some("codex/gpt-5.6-terra".into());

    let first_id = first.request(deepseek).await.unwrap().run_id;
    let second_id = second.request(terra).await.unwrap().run_id;
    assert_ne!(first_id, second_id);
}

#[tokio::test]
async fn request_rejects_a_symbolic_ref_instead_of_persisting_a_mutable_target() {
    let runtime = Arc::new(FakeRuntime::default());
    let service = service(runtime.clone());

    let error = service
        .request(SecurityScanRequestV1::new(
            "iii-hq/iii".into(),
            "main".into(),
            ScanModeV1::Scan,
        ))
        .await
        .unwrap_err();

    assert!(matches!(error, SecurityScanError::InvalidRequest(_)));
    assert!(runtime.run.lock().await.is_none());
    assert!(runtime.enqueued.lock().await.is_empty());
}

#[tokio::test]
async fn omitted_sha_analyzes_the_entire_repository_at_head() {
    let runtime = Arc::new(FakeRuntime::default());
    let service = service(runtime.clone());
    let request: SecurityScanRequestV1 = serde_json::from_value(serde_json::json!({
        "repository": "iii-hq/iii",
        "mode": "scan"
    }))
    .unwrap();

    let response = service.request(request).await.unwrap();
    let stored = runtime.run.lock().await.clone().unwrap();
    assert!(!response.deduplicated);
    assert_eq!(stored.target_sha, "b".repeat(40));
    assert!(stored.resolved_from_head);

    let public = service
        .read(SecurityScanReadRequestV1::new(response.run_id))
        .await
        .unwrap()
        .run
        .unwrap();
    assert!(public.resolved_from_head);
    let encoded = serde_json::to_value(&public).unwrap();
    assert_eq!(encoded["resolved_from_head"], true);
    assert!(encoded.get("harness").is_none());
    assert!(encoded.get("materialized").is_none());
    assert!(encoded.get("operation_nonce").is_none());
}

#[tokio::test]
async fn request_rejects_a_repository_that_is_not_operator_configured() {
    let runtime = Arc::new(FakeRuntime::default());
    let service = service(runtime.clone());

    let error = service
        .request(SecurityScanRequestV1::new(
            "attacker/untrusted".into(),
            "0123456789abcdef0123456789abcdef01234567".into(),
            ScanModeV1::Scan,
        ))
        .await
        .unwrap_err();

    assert!(matches!(error, SecurityScanError::InvalidRequest(_)));
    assert!(runtime.run.lock().await.is_none());
    assert!(runtime.enqueued.lock().await.is_empty());
}

#[tokio::test]
async fn enqueue_failure_keeps_a_durable_outbox_checkpoint_for_recovery() {
    let runtime = Arc::new(FakeRuntime::default());
    runtime.fail_enqueue_once.store(true, Ordering::SeqCst);
    let service = service(runtime.clone());
    let request = SecurityScanRequestV1::new(
        "iii-hq/iii".into(),
        "0123456789abcdef0123456789abcdef01234567".into(),
        ScanModeV1::Scan,
    );

    let error = service.request(request.clone()).await.unwrap_err();
    assert!(matches!(error, SecurityScanError::Dependency(_)));
    let stored = runtime
        .run
        .lock()
        .await
        .clone()
        .expect("durable queued run");
    assert_eq!(stored.status, RunStatusV1::Queued);

    let recovered = service.request(request).await.unwrap();
    assert!(recovered.deduplicated);
    assert!(runtime.enqueued.lock().await.is_empty());
}

#[tokio::test]
async fn read_returns_a_sanitized_public_run_without_internal_paths_or_session_ids() {
    let runtime = Arc::new(FakeRuntime::default());
    let service = service(runtime);
    let requested = service
        .request(SecurityScanRequestV1::new(
            "iii-hq/iii".into(),
            "0123456789abcdef0123456789abcdef01234567".into(),
            ScanModeV1::Suggest,
        ))
        .await
        .unwrap();

    let response = service
        .read(SecurityScanReadRequestV1::new(requested.run_id))
        .await
        .unwrap();

    let run = response.run.unwrap();
    assert_eq!(run.repository, "iii-hq/iii");
    assert_eq!(run.status, security_scan::RunStatusV1::Queued);
    let encoded = serde_json::to_value(run).unwrap();
    assert!(encoded.get("materialized").is_none());
    assert!(encoded.get("harness").is_none());
    assert!(encoded.get("operation_nonce").is_none());
}

#[tokio::test]
async fn cancel_marks_an_in_flight_run_cancelling_and_enqueues_cleanup() {
    let runtime = Arc::new(FakeRuntime::default());
    let service = service(runtime.clone());
    let requested = service
        .request(SecurityScanRequestV1::new(
            "iii-hq/iii".into(),
            "0123456789abcdef0123456789abcdef01234567".into(),
            ScanModeV1::Scan,
        ))
        .await
        .unwrap();

    let cancelled = service
        .cancel(security_scan::SecurityScanCancelRequestV1::new(
            requested.run_id.clone(),
        ))
        .await
        .unwrap();
    assert!(!cancelled.deduplicated);
    assert_eq!(cancelled.status, RunStatusV1::Cancelling);
    let stored = runtime.run.lock().await.clone().unwrap();
    assert_eq!(stored.status, RunStatusV1::Cancelling);
    assert_eq!(runtime.enqueued.lock().await.len(), 2);
    let encoded = serde_json::to_value(&cancelled).unwrap();
    assert!(encoded.get("harness").is_none());
    assert!(encoded.get("session_id").is_none());
    assert!(encoded.get("operation_nonce").is_none());
}

#[tokio::test]
async fn cancel_of_an_already_terminal_run_is_idempotent() {
    let runtime = Arc::new(FakeRuntime::default());
    let service = service(runtime.clone());
    let requested = service
        .request(SecurityScanRequestV1::new(
            "iii-hq/iii".into(),
            "0123456789abcdef0123456789abcdef01234567".into(),
            ScanModeV1::Scan,
        ))
        .await
        .unwrap();
    let mut stored = runtime.run.lock().await.clone().unwrap();
    stored.status = RunStatusV1::Completed;
    *runtime.run.lock().await = Some(stored);

    let cancelled = service
        .cancel(security_scan::SecurityScanCancelRequestV1::new(
            requested.run_id,
        ))
        .await
        .unwrap();
    assert!(cancelled.deduplicated);
    assert_eq!(cancelled.status, RunStatusV1::Completed);
}

fn listed_run(
    run_id: &str,
    repository: &str,
    status: RunStatusV1,
    updated_at: i64,
    finding_count: usize,
) -> RunRecordV1 {
    let findings = (0..finding_count)
        .map(|index| SecurityFindingV1 {
            rule_id: format!("SEC-{index}"),
            severity: SeverityV1::High,
            title: "Finding".into(),
            description: "Description".into(),
            evidence: "Evidence".into(),
            location: None,
            remediation: "Remediation".into(),
            suggested_patch: None,
        })
        .collect();
    RunRecordV1 {
        schema_version: "1".into(),
        run_id: run_id.into(),
        repository: repository.into(),
        target_sha: "a".repeat(40),
        resolved_from_head: false,
        mode: ScanModeV1::Scan,
        model: None,
        provider: None,
        operation_nonce: format!("private_{run_id}"),
        status,
        attempt: 1,
        step: 2,
        step_failures: 0,
        materialized: Some(security_scan::MaterializedTargetV1 {
            worktree_id: format!("wt_{run_id}"),
            path: format!("/private/{run_id}"),
            base_sha: "a".repeat(40),
        }),
        harness: Some(security_scan::HarnessRunV1 {
            session_id: format!("session_{run_id}"),
            turn_id: format!("turn_{run_id}"),
        }),
        report: Some(SecurityReportV1 {
            summary: "Summary".into(),
            assessments: SecurityAssessmentsV1::default(),
            findings,
        }),
        error: None,
        created_at: updated_at.saturating_sub(10),
        updated_at,
        completed_at: (status == RunStatusV1::Completed).then_some(updated_at),
    }
}

#[tokio::test]
async fn list_sorts_filters_limits_and_returns_only_sanitized_summaries() {
    let runtime = Arc::new(FakeRuntime::default());
    *runtime.listed_runs.lock().await = vec![
        listed_run("sec_old", "iii-hq/iii", RunStatusV1::Completed, 100, 2),
        listed_run("sec_z", "iii-hq/iii", RunStatusV1::Analyzing, 200, 0),
        listed_run("sec_a", "iii-hq/iii", RunStatusV1::Analyzing, 200, 0),
        listed_run("sec_other", "other/repo", RunStatusV1::Analyzing, 300, 0),
    ];
    let service = service(runtime);

    let all = service
        .list(SecurityScanListRequestV1::default())
        .await
        .unwrap();
    assert_eq!(
        all.runs
            .iter()
            .map(|run| run.run_id.as_str())
            .collect::<Vec<_>>(),
        ["sec_other", "sec_a", "sec_z", "sec_old"]
    );

    let filtered = service
        .list(SecurityScanListRequestV1::new(
            Some(" iii-hq/iii ".into()),
            Some(RunStatusV1::Analyzing),
            Some(1),
        ))
        .await
        .unwrap();
    assert_eq!(filtered.runs.len(), 1);
    assert_eq!(filtered.runs[0].run_id, "sec_a");

    let completed = service
        .list(SecurityScanListRequestV1::new(
            None,
            Some(RunStatusV1::Completed),
            Some(10),
        ))
        .await
        .unwrap();
    assert_eq!(completed.runs[0].finding_count, 2);
    let encoded = serde_json::to_value(&completed.runs[0]).unwrap();
    for private in [
        "operation_nonce",
        "materialized",
        "harness",
        "report",
        "step",
    ] {
        assert!(encoded.get(private).is_none(), "leaked {private}");
    }
}

#[tokio::test]
async fn list_defaults_to_fifty_and_rejects_invalid_limits_or_filters() {
    let runtime = Arc::new(FakeRuntime::default());
    *runtime.listed_runs.lock().await = (0..51)
        .map(|index| {
            listed_run(
                &format!("sec_{index:02}"),
                "iii-hq/iii",
                RunStatusV1::Queued,
                index,
                0,
            )
        })
        .collect();
    let service = service(runtime);

    assert_eq!(
        service
            .list(SecurityScanListRequestV1::default())
            .await
            .unwrap()
            .runs
            .len(),
        50
    );
    for request in [
        SecurityScanListRequestV1::new(None, None, Some(0)),
        SecurityScanListRequestV1::new(None, None, Some(201)),
        SecurityScanListRequestV1::new(Some("   ".into()), None, Some(1)),
    ] {
        assert!(matches!(
            service.list(request).await.unwrap_err(),
            SecurityScanError::InvalidRequest(_)
        ));
    }
}

#[tokio::test]
async fn repeating_a_retryable_failed_request_atomically_starts_a_new_attempt() {
    let runtime = Arc::new(FakeRuntime::default());
    let service = service(runtime.clone());
    let request = SecurityScanRequestV1::new(
        "iii-hq/iii".into(),
        "0123456789abcdef0123456789abcdef01234567".into(),
        ScanModeV1::Suggest,
    );
    let first = service.request(request.clone()).await.unwrap();
    {
        let mut stored = runtime.run.lock().await;
        let run = stored.as_mut().unwrap();
        run.status = RunStatusV1::Failed;
        run.error = Some(RunErrorV1 {
            code: "analysis_failed".into(),
            message: "temporary dependency failure".into(),
            retryable: true,
        });
        run.completed_at = Some(run.updated_at);
    }
    runtime.enqueued.lock().await.clear();

    let retry = service.request(request).await.unwrap();

    assert_eq!(retry.run_id, first.run_id);
    assert_eq!(retry.status, RunStatusV1::Queued);
    assert!(!retry.deduplicated);
    let stored = runtime.run.lock().await.clone().unwrap();
    assert_eq!(stored.attempt, 2);
    assert_eq!(stored.step, 0);
    assert!(stored.error.is_none());
    let enqueued = runtime.enqueued.lock().await;
    assert_eq!(enqueued.len(), 1);
    assert_eq!(enqueued[0].attempt, 2);
}
