use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use async_trait::async_trait;
use security_scan::{
    ActionEnqueueRequestV1, ActionRuntime, AnalysisConfigV1, AnalysisHandle, AnalysisPlan,
    CreateActionOutcome, CreateRunOutcome, EnqueueRequest, ExecutionRuntime,
    MaterializationRequest, MaterializedTargetV1, RepositoryConfigV1, RepositoryGitHubConfigV1,
    RunRecordV1, RunStatusV1, ScanModeV1, SecurityActionExecutor, SecurityActionKindV1,
    SecurityActionRecordV1, SecurityActionResultV1, SecurityActionStatusV1, SecurityAssessmentsV1,
    SecurityFindingV1, SecurityReportV1, SecurityRuntime, SecurityScanError, TurnCompletedEventV1,
    WorkerConfig,
};
use tokio::sync::Mutex;

struct FakeRuntime {
    gate_live: AtomicBool,
    run: Mutex<RunRecordV1>,
    action: Mutex<SecurityActionRecordV1>,
    plans: Mutex<Vec<AnalysisPlan>>,
    materialized: Mutex<Vec<String>>,
    cleaned: Mutex<Vec<String>>,
    enqueued: Mutex<Vec<ActionEnqueueRequestV1>>,
}

fn analysis_config() -> AnalysisConfigV1 {
    AnalysisConfigV1 {
        model: "security-review-model".into(),
        provider: None,
        max_turns: 4,
        max_output_tokens: 8_000,
        max_total_tokens: 50_000,
        max_cost_usd: Some(2.0),
    }
}

fn config() -> WorkerConfig {
    WorkerConfig {
        repositories: vec![RepositoryConfigV1 {
            id: "iii-hq/iii".into(),
            path: "/srv/repos/iii".into(),
            github: Some(RepositoryGitHubConfigV1 {
                full_name: "iii-hq/iii".into(),
            }),
            schedule: None,
        }],
        analysis: analysis_config(),
        archive: None,
    }
}

fn completed_run() -> RunRecordV1 {
    RunRecordV1 {
        schema_version: "1".into(),
        run_id: "sec_completed".into(),
        repository: "iii-hq/iii".into(),
        target_sha: "0123456789abcdef0123456789abcdef01234567".into(),
        resolved_from_head: false,
        mode: ScanModeV1::Suggest,
        model: None,
        provider: None,
        operation_nonce: "scan_nonce".into(),
        status: RunStatusV1::Completed,
        attempt: 1,
        step: 2,
        step_failures: 0,
        materialized: None,
        harness: None,
        report: Some(SecurityReportV1 {
            summary: "One finding".into(),
            assessments: SecurityAssessmentsV1::default(),
            findings: vec![SecurityFindingV1 {
                rule_id: "SEC-001".into(),
                severity: security_scan::SeverityV1::High,
                title: "Unsafe default".into(),
                description: "Details".into(),
                evidence: "Evidence".into(),
                location: None,
                remediation: "Fix it".into(),
                suggested_patch: Some("diff --git a/x b/x".into()),
            }],
        }),
        error: None,
        created_at: 1,
        updated_at: 2,
        completed_at: Some(2),
    }
}

fn queued_action(kind: SecurityActionKindV1) -> SecurityActionRecordV1 {
    SecurityActionRecordV1 {
        schema_version: "1".into(),
        action_id: "seca_action".into(),
        run_id: "sec_completed".into(),
        finding_index: 0,
        action: kind,
        repository: "iii-hq/iii".into(),
        target_sha: "0123456789abcdef0123456789abcdef01234567".into(),
        github_full_name: "iii-hq/iii".into(),
        operation_nonce: "action_nonce".into(),
        status: SecurityActionStatusV1::Queued,
        attempt: 1,
        step: 0,
        step_failures: 0,
        materialized: None,
        harness: None,
        result: None,
        error: None,
        created_at: 1,
        updated_at: 1,
        completed_at: None,
        cleanup_completed_at: None,
    }
}

fn runtime(action: SecurityActionRecordV1, gate_live: bool) -> Arc<FakeRuntime> {
    Arc::new(FakeRuntime {
        gate_live: AtomicBool::new(gate_live),
        run: Mutex::new(completed_run()),
        action: Mutex::new(action),
        plans: Mutex::new(Vec::new()),
        materialized: Mutex::new(Vec::new()),
        cleaned: Mutex::new(Vec::new()),
        enqueued: Mutex::new(Vec::new()),
    })
}

#[async_trait]
impl SecurityRuntime for FakeRuntime {
    fn require_ready(&self) -> Result<(), SecurityScanError> {
        Ok(())
    }

    async fn get_run(&self, run_id: &str) -> Result<Option<RunRecordV1>, SecurityScanError> {
        let run = self.run.lock().await.clone();
        Ok((run.run_id == run_id).then_some(run))
    }

    async fn create_run_if_absent(
        &self,
        _run: RunRecordV1,
    ) -> Result<CreateRunOutcome, SecurityScanError> {
        unreachable!()
    }

    async fn replace_run(
        &self,
        _expected: &RunRecordV1,
        _replacement: RunRecordV1,
    ) -> Result<bool, SecurityScanError> {
        unreachable!()
    }

    async fn delete_run_if_unchanged(&self, _run: &RunRecordV1) -> Result<(), SecurityScanError> {
        Ok(())
    }

    async fn enqueue_execute(&self, _request: EnqueueRequest) -> Result<(), SecurityScanError> {
        unreachable!()
    }

    async fn stop_analysis(
        &self,
        _harness: &security_scan::HarnessRunV1,
    ) -> Result<(), SecurityScanError> {
        unreachable!()
    }

    async fn ensure_analysis_chat_link(
        &self,
        _run: &RunRecordV1,
    ) -> Result<bool, SecurityScanError> {
        unreachable!()
    }

    async fn get_action(
        &self,
        action_id: &str,
    ) -> Result<Option<SecurityActionRecordV1>, SecurityScanError> {
        let action = self.action.lock().await.clone();
        Ok((action.action_id == action_id).then_some(action))
    }

    async fn list_actions(&self) -> Result<Vec<SecurityActionRecordV1>, SecurityScanError> {
        Ok(vec![self.action.lock().await.clone()])
    }

    async fn create_action_if_absent(
        &self,
        _action: SecurityActionRecordV1,
    ) -> Result<CreateActionOutcome, SecurityScanError> {
        unreachable!()
    }

    async fn replace_action(
        &self,
        expected: &SecurityActionRecordV1,
        replacement: SecurityActionRecordV1,
    ) -> Result<bool, SecurityScanError> {
        let mut action = self.action.lock().await;
        if &*action != expected {
            return Ok(false);
        }
        *action = replacement;
        Ok(true)
    }

    async fn delete_action_if_unchanged(
        &self,
        _action: &SecurityActionRecordV1,
    ) -> Result<(), SecurityScanError> {
        unreachable!()
    }

    async fn enqueue_action_execute(
        &self,
        request: ActionEnqueueRequestV1,
    ) -> Result<(), SecurityScanError> {
        self.enqueued.lock().await.push(request);
        Ok(())
    }

    async fn approval_gate_is_live(&self) -> Result<bool, SecurityScanError> {
        Ok(self.gate_live.load(Ordering::SeqCst))
    }
}

#[async_trait]
impl ExecutionRuntime for FakeRuntime {
    async fn get_run_by_session(
        &self,
        _session_id: &str,
    ) -> Result<Option<RunRecordV1>, SecurityScanError> {
        Ok(None)
    }

    async fn materialize_target(
        &self,
        _repository: &RepositoryConfigV1,
        _request: &MaterializationRequest,
    ) -> Result<MaterializedTargetV1, SecurityScanError> {
        unreachable!()
    }

    async fn start_analysis(
        &self,
        plan: AnalysisPlan,
    ) -> Result<AnalysisHandle, SecurityScanError> {
        self.plans.lock().await.push(plan);
        Ok(AnalysisHandle {
            session_id: "session_action".into(),
            turn_id: "turn_action".into(),
        })
    }

    async fn cleanup_target(&self, target: &MaterializedTargetV1) -> Result<(), SecurityScanError> {
        self.cleaned.lock().await.push(target.worktree_id.clone());
        Ok(())
    }

    async fn completed_analysis(
        &self,
        _run: &RunRecordV1,
    ) -> Result<Option<TurnCompletedEventV1>, SecurityScanError> {
        Ok(None)
    }
}

#[async_trait]
impl ActionRuntime for FakeRuntime {
    async fn materialize_action_target(
        &self,
        _repository: &RepositoryConfigV1,
        action: &SecurityActionRecordV1,
    ) -> Result<MaterializedTargetV1, SecurityScanError> {
        self.materialized
            .lock()
            .await
            .push(action.target_sha.clone());
        Ok(MaterializedTargetV1 {
            worktree_id: "wt_action".into(),
            path: "/private/tmp/wt_action".into(),
            base_sha: action.target_sha.clone(),
        })
    }

    async fn completed_action(
        &self,
        _action: &SecurityActionRecordV1,
    ) -> Result<Option<TurnCompletedEventV1>, SecurityScanError> {
        Ok(None)
    }

    async fn get_action_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<SecurityActionRecordV1>, SecurityScanError> {
        let action = self.action.lock().await.clone();
        let matches = action
            .harness
            .as_ref()
            .is_some_and(|harness| harness.session_id == session_id);
        Ok(matches.then_some(action))
    }
}

#[tokio::test]
async fn missing_approval_gate_fails_closed_without_starting_a_session() {
    let runtime = runtime(queued_action(SecurityActionKindV1::Issue), false);
    let executor = SecurityActionExecutor::new(runtime.clone(), config());
    let response = executor
        .execute(ActionEnqueueRequestV1::new(
            "seca_action".into(),
            "sec_completed".into(),
            1,
            0,
        ))
        .await
        .unwrap();
    assert!(!response.skipped);
    assert_eq!(response.status, SecurityActionStatusV1::Failed);
    let stored = runtime.action.lock().await.clone();
    assert_eq!(stored.status, SecurityActionStatusV1::Failed);
    let error = stored.error.expect("fail-closed error");
    assert_eq!(error.code, "approval_unavailable");
    assert!(error.retryable);
    assert!(runtime.plans.lock().await.is_empty());
}

#[tokio::test]
async fn existing_publication_is_not_republished() {
    let mut action = queued_action(SecurityActionKindV1::Issue);
    action.result = Some(SecurityActionResultV1 {
        url: "https://github.com/iii-hq/iii/issues/12".into(),
        kind: "issue".into(),
        branch: None,
        commit_sha: None,
        draft: None,
        validation: None,
    });
    let runtime = runtime(action, true);
    let executor = SecurityActionExecutor::new(runtime.clone(), config());
    let response = executor
        .execute(ActionEnqueueRequestV1::new(
            "seca_action".into(),
            "sec_completed".into(),
            1,
            0,
        ))
        .await
        .unwrap();
    assert!(response.skipped);
    assert_eq!(response.status, SecurityActionStatusV1::Completed);
    assert!(runtime.plans.lock().await.is_empty());
    let stored = runtime.action.lock().await.clone();
    assert_eq!(stored.status, SecurityActionStatusV1::Completed);
    assert_eq!(
        stored.result.unwrap().url,
        "https://github.com/iii-hq/iii/issues/12"
    );
}

#[tokio::test]
async fn recover_actions_reenqueues_in_flight_actions() {
    let runtime = runtime(queued_action(SecurityActionKindV1::Issue), true);
    let executor = SecurityActionExecutor::new(runtime.clone(), config());
    executor.recover_actions().await.unwrap();
    let enqueued = runtime.enqueued.lock().await.clone();
    assert_eq!(enqueued.len(), 1);
    assert_eq!(enqueued[0].action_id, "seca_action");
    assert_eq!(enqueued[0].step, 0);
}

#[tokio::test]
async fn recover_actions_cleans_terminal_worktrees() {
    let mut action = queued_action(SecurityActionKindV1::FixPr);
    action.status = SecurityActionStatusV1::Completed;
    action.completed_at = Some(3);
    action.materialized = Some(MaterializedTargetV1 {
        worktree_id: "wt_action".into(),
        path: "/private/tmp/wt_action".into(),
        base_sha: action.target_sha.clone(),
    });
    let runtime = runtime(action, true);
    let executor = SecurityActionExecutor::new(runtime.clone(), config());
    executor.recover_actions().await.unwrap();
    assert!(runtime.enqueued.lock().await.is_empty());
    assert_eq!(runtime.cleaned.lock().await.as_slice(), ["wt_action"]);
    assert!(runtime.action.lock().await.cleanup_completed_at.is_some());
    executor.recover_actions().await.unwrap();
    assert_eq!(
        runtime.cleaned.lock().await.as_slice(),
        ["wt_action"],
        "persisted cleanup completion must suppress repeated cleanup"
    );
}

#[tokio::test]
async fn fix_pr_materializes_an_exact_sha_checkout_then_starts_a_session() {
    let runtime = runtime(queued_action(SecurityActionKindV1::FixPr), true);
    let executor = SecurityActionExecutor::new(runtime.clone(), config());
    let response = executor
        .execute(ActionEnqueueRequestV1::new(
            "seca_action".into(),
            "sec_completed".into(),
            1,
            0,
        ))
        .await
        .unwrap();
    assert!(!response.skipped);
    assert_eq!(response.status, SecurityActionStatusV1::AwaitingApproval);
    assert_eq!(
        runtime.materialized.lock().await.as_slice(),
        ["0123456789abcdef0123456789abcdef01234567"]
    );
    let stored = runtime.action.lock().await.clone();
    assert_eq!(
        stored.materialized.as_ref().unwrap().base_sha,
        stored.target_sha
    );
    assert_eq!(stored.harness.unwrap().session_id, "session_action");
    let plans = runtime.plans.lock().await;
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].filesystem_root, "/private/tmp/wt_action");
    assert!(plans[0]
        .allowed_functions
        .iter()
        .any(|function| function == "github::pr::create"));
}
