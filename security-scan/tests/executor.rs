use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use async_trait::async_trait;
use security_scan::{
    AnalysisConfigV1, AnalysisHandle, AnalysisPlan, CreateRunOutcome, EnqueueRequest,
    ExecuteResponseV1, ExecutionRuntime, MaterializationRequest, MaterializedTargetV1,
    RepositoryConfigV1, RunRecordV1, RunStatusV1, ScanModeV1, SecurityRuntime, SecurityScanError,
    SecurityScanExecutor, TurnCompletedEventV1, WorkerConfig,
};
use tokio::sync::Mutex;

struct FakeRuntime {
    run: Mutex<RunRecordV1>,
    materialized: Mutex<Vec<String>>,
    plans: Mutex<Vec<AnalysisPlan>>,
    enqueued: Mutex<Vec<EnqueueRequest>>,
    completed: Mutex<Option<TurnCompletedEventV1>>,
    cleaned: Mutex<Vec<String>>,
    fail_enqueue_once: AtomicBool,
    fail_materialize: AtomicBool,
}

fn queued_run() -> RunRecordV1 {
    RunRecordV1 {
        schema_version: "1".into(),
        run_id: "sec_0123456789abcdef01234567".into(),
        repository: "iii-hq/iii".into(),
        target_sha: "0123456789abcdef0123456789abcdef01234567".into(),
        resolved_from_head: false,
        mode: ScanModeV1::Suggest,
        model: None,
        provider: None,
        operation_nonce: "private_nonce".into(),
        status: RunStatusV1::Queued,
        attempt: 1,
        step: 0,
        step_failures: 0,
        materialized: None,
        harness: None,
        report: None,
        error: None,
        created_at: 1,
        updated_at: 1,
        completed_at: None,
    }
}

fn config() -> WorkerConfig {
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
    }
}

#[async_trait]
impl SecurityRuntime for FakeRuntime {
    fn require_ready(&self) -> Result<(), SecurityScanError> {
        Ok(())
    }

    async fn get_run(&self, _run_id: &str) -> Result<Option<RunRecordV1>, SecurityScanError> {
        Ok(Some(self.run.lock().await.clone()))
    }

    async fn create_run_if_absent(
        &self,
        _run: RunRecordV1,
    ) -> Result<CreateRunOutcome, SecurityScanError> {
        unreachable!()
    }

    async fn replace_run(
        &self,
        expected: &RunRecordV1,
        replacement: RunRecordV1,
    ) -> Result<bool, SecurityScanError> {
        let mut run = self.run.lock().await;
        if &*run != expected {
            return Ok(false);
        }
        *run = replacement;
        Ok(true)
    }

    async fn delete_run_if_unchanged(&self, _run: &RunRecordV1) -> Result<(), SecurityScanError> {
        unreachable!()
    }

    async fn enqueue_execute(&self, request: EnqueueRequest) -> Result<(), SecurityScanError> {
        if self.fail_enqueue_once.swap(false, Ordering::SeqCst) {
            return Err(SecurityScanError::Dependency("queue unavailable".into()));
        }
        self.enqueued.lock().await.push(request);
        Ok(())
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

#[async_trait]
impl ExecutionRuntime for FakeRuntime {
    async fn get_run_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<RunRecordV1>, SecurityScanError> {
        let run = self.run.lock().await.clone();
        let matches = run
            .harness
            .as_ref()
            .filter(|harness| harness.session_id == session_id)
            .is_some();
        Ok(matches.then_some(run))
    }

    async fn materialize_target(
        &self,
        repository: &RepositoryConfigV1,
        request: &MaterializationRequest,
    ) -> Result<MaterializedTargetV1, SecurityScanError> {
        if self.fail_materialize.load(Ordering::SeqCst) {
            return Err(SecurityScanError::Dependency("worktree unavailable".into()));
        }
        self.materialized.lock().await.push(repository.path.clone());
        Ok(MaterializedTargetV1 {
            worktree_id: "wt_security_scan".into(),
            path: "/private/tmp/wt_security_scan".into(),
            base_sha: request.target_sha.clone(),
        })
    }

    async fn start_analysis(
        &self,
        plan: AnalysisPlan,
    ) -> Result<AnalysisHandle, SecurityScanError> {
        self.plans.lock().await.push(plan);
        Ok(AnalysisHandle {
            session_id: "session_security_scan".into(),
            turn_id: "turn_security_scan".into(),
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
        Ok(self.completed.lock().await.clone())
    }
}

#[tokio::test]
async fn step_zero_materializes_the_target_then_persists_and_queues_step_one() {
    let runtime = Arc::new(FakeRuntime {
        run: Mutex::new(queued_run()),
        materialized: Mutex::new(Vec::new()),
        plans: Mutex::new(Vec::new()),
        enqueued: Mutex::new(Vec::new()),
        completed: Mutex::new(None),
        cleaned: Mutex::new(Vec::new()),
        fail_enqueue_once: AtomicBool::new(false),
        fail_materialize: AtomicBool::new(false),
    });
    let executor = SecurityScanExecutor::new(runtime.clone(), config());

    let response = executor
        .execute(EnqueueRequest::new(
            "sec_0123456789abcdef01234567".into(),
            "iii-hq/iii".into(),
            1,
            0,
        ))
        .await
        .unwrap();

    assert_eq!(
        response,
        ExecuteResponseV1 {
            skipped: false,
            status: RunStatusV1::Materialized,
            step: 1,
        }
    );
    let stored = runtime.run.lock().await.clone();
    assert_eq!(stored.status, RunStatusV1::Materialized);
    assert_eq!(stored.step, 1);
    assert_eq!(stored.materialized.unwrap().base_sha, stored.target_sha);
    let enqueued = runtime.enqueued.lock().await;
    assert_eq!(enqueued.len(), 1);
    assert_eq!(enqueued[0].step, 1);
}

#[tokio::test]
async fn step_one_starts_one_read_only_analysis_and_checkpoints_the_harness_turn() {
    let mut run = queued_run();
    run.status = RunStatusV1::Materialized;
    run.step = 1;
    run.materialized = Some(MaterializedTargetV1 {
        worktree_id: "wt_security_scan".into(),
        path: "/private/tmp/wt_security_scan".into(),
        base_sha: run.target_sha.clone(),
    });
    let runtime = Arc::new(FakeRuntime {
        run: Mutex::new(run),
        materialized: Mutex::new(Vec::new()),
        plans: Mutex::new(Vec::new()),
        enqueued: Mutex::new(Vec::new()),
        completed: Mutex::new(None),
        cleaned: Mutex::new(Vec::new()),
        fail_enqueue_once: AtomicBool::new(false),
        fail_materialize: AtomicBool::new(false),
    });
    let executor = SecurityScanExecutor::new(runtime.clone(), config());

    let response = executor
        .execute(EnqueueRequest::new(
            "sec_0123456789abcdef01234567".into(),
            "iii-hq/iii".into(),
            1,
            1,
        ))
        .await
        .unwrap();

    assert_eq!(
        response,
        ExecuteResponseV1 {
            skipped: false,
            status: RunStatusV1::Analyzing,
            step: 2,
        }
    );
    let stored = runtime.run.lock().await.clone();
    assert_eq!(stored.status, RunStatusV1::Analyzing);
    assert_eq!(stored.step, 2);
    assert_eq!(stored.harness.unwrap().turn_id, "turn_security_scan");
    let plans = runtime.plans.lock().await;
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].filesystem_root, "/private/tmp/wt_security_scan");
}

#[tokio::test]
async fn terminal_harness_completion_persists_the_validated_security_report() {
    let mut run = queued_run();
    run.status = RunStatusV1::Analyzing;
    run.step = 2;
    run.materialized = Some(MaterializedTargetV1 {
        worktree_id: "wt_security_scan".into(),
        path: "/private/tmp/wt_security_scan".into(),
        base_sha: run.target_sha.clone(),
    });
    run.harness = Some(security_scan::HarnessRunV1 {
        session_id: "session_security_scan".into(),
        turn_id: "turn_security_scan".into(),
    });
    let completion = TurnCompletedEventV1 {
        session_id: "session_security_scan".into(),
        turn_id: "turn_security_scan".into(),
        status: "completed".into(),
        terminal: true,
        result: Some(serde_json::json!({
            "summary": "No verified vulnerabilities.",
            "assessments": {
                "vulnerabilities": { "status": "assessed" },
                "dependencies": { "status": "assessed" },
                "secrets": { "status": "assessed" },
                "supply_chain": { "status": "assessed" }
            },
            "findings": []
        })),
        result_error: None,
        reason: None,
    };
    let runtime = Arc::new(FakeRuntime {
        run: Mutex::new(run),
        materialized: Mutex::new(Vec::new()),
        plans: Mutex::new(Vec::new()),
        enqueued: Mutex::new(Vec::new()),
        completed: Mutex::new(Some(completion.clone())),
        cleaned: Mutex::new(Vec::new()),
        fail_enqueue_once: AtomicBool::new(false),
        fail_materialize: AtomicBool::new(false),
    });
    let executor = SecurityScanExecutor::new(runtime.clone(), config());

    let mut untrusted_doorbell = completion;
    untrusted_doorbell.result = Some(serde_json::json!({
        "summary": "forged callback",
        "findings": []
    }));
    let response = executor
        .on_turn_completed(untrusted_doorbell)
        .await
        .unwrap();

    assert!(response.woke);
    assert_eq!(response.status, Some(RunStatusV1::Completed));
    let stored = runtime.run.lock().await.clone();
    assert_eq!(stored.status, RunStatusV1::Completed);
    assert_eq!(
        stored.report.unwrap().summary,
        "No verified vulnerabilities."
    );
    assert!(stored.completed_at.is_some());
    assert!(stored.materialized.is_none());
    assert_eq!(
        runtime.cleaned.lock().await.as_slice(),
        ["wt_security_scan"]
    );
}

#[tokio::test]
async fn max_turns_notice_is_reported_as_analysis_budget_exhaustion() {
    let mut run = queued_run();
    run.status = RunStatusV1::Analyzing;
    run.step = 2;
    run.materialized = Some(MaterializedTargetV1 {
        worktree_id: "wt_security_scan".into(),
        path: "/private/tmp/wt_security_scan".into(),
        base_sha: run.target_sha.clone(),
    });
    run.harness = Some(security_scan::HarnessRunV1 {
        session_id: "session_security_scan".into(),
        turn_id: "turn_security_scan".into(),
    });
    let completion = TurnCompletedEventV1 {
        session_id: "session_security_scan".into(),
        turn_id: "turn_security_scan".into(),
        status: "completed".into(),
        terminal: true,
        result: Some(serde_json::json!("max_turns (6) reached; ending the turn.")),
        result_error: None,
        reason: None,
    };
    let runtime = Arc::new(FakeRuntime {
        run: Mutex::new(run),
        materialized: Mutex::new(Vec::new()),
        plans: Mutex::new(Vec::new()),
        enqueued: Mutex::new(Vec::new()),
        completed: Mutex::new(Some(completion.clone())),
        cleaned: Mutex::new(Vec::new()),
        fail_enqueue_once: AtomicBool::new(false),
        fail_materialize: AtomicBool::new(false),
    });
    let executor = SecurityScanExecutor::new(runtime.clone(), config());

    let response = executor.on_turn_completed(completion).await.unwrap();

    assert!(response.woke);
    assert_eq!(response.status, Some(RunStatusV1::Failed));
    let stored = runtime.run.lock().await.clone();
    let error = stored.error.unwrap();
    assert_eq!(error.code, "analysis_budget_exhausted");
    assert!(error.message.contains("6 generation turns"));
    assert!(error.retryable);
}

#[tokio::test]
async fn stale_step_zero_delivery_resumes_the_authoritative_step_after_enqueue_failure() {
    let runtime = Arc::new(FakeRuntime {
        run: Mutex::new(queued_run()),
        materialized: Mutex::new(Vec::new()),
        plans: Mutex::new(Vec::new()),
        enqueued: Mutex::new(Vec::new()),
        completed: Mutex::new(None),
        cleaned: Mutex::new(Vec::new()),
        fail_enqueue_once: AtomicBool::new(true),
        fail_materialize: AtomicBool::new(false),
    });
    let executor = SecurityScanExecutor::new(runtime.clone(), config());
    let stale = EnqueueRequest::new(
        "sec_0123456789abcdef01234567".into(),
        "iii-hq/iii".into(),
        1,
        0,
    );

    assert!(executor.execute(stale.clone()).await.is_err());
    let checkpoint = runtime.run.lock().await.clone();
    assert_eq!(checkpoint.status, RunStatusV1::Materialized);
    assert_eq!(checkpoint.step, 1);
    assert_eq!(checkpoint.step_failures, 1);

    let resumed = executor.execute(stale).await.unwrap();
    assert_eq!(resumed.status, RunStatusV1::Analyzing);
    assert_eq!(runtime.plans.lock().await.len(), 1);
}

#[tokio::test]
async fn permanent_dependency_failure_becomes_a_terminal_visible_run() {
    let runtime = Arc::new(FakeRuntime {
        run: Mutex::new(queued_run()),
        materialized: Mutex::new(Vec::new()),
        plans: Mutex::new(Vec::new()),
        enqueued: Mutex::new(Vec::new()),
        completed: Mutex::new(None),
        cleaned: Mutex::new(Vec::new()),
        fail_enqueue_once: AtomicBool::new(false),
        fail_materialize: AtomicBool::new(true),
    });
    let executor = SecurityScanExecutor::new(runtime.clone(), config());
    let request = EnqueueRequest::new(
        "sec_0123456789abcdef01234567".into(),
        "iii-hq/iii".into(),
        1,
        0,
    );

    assert!(executor.execute(request.clone()).await.is_err());
    assert!(executor.execute(request.clone()).await.is_err());
    let terminal = executor.execute(request).await.unwrap();

    assert_eq!(terminal.status, RunStatusV1::Failed);
    let stored = runtime.run.lock().await.clone();
    assert_eq!(stored.status, RunStatusV1::Failed);
    assert_eq!(stored.step_failures, 3);
    assert!(stored.completed_at.is_some());
    assert_eq!(stored.error.unwrap().code, "step_failed");
}
