use std::sync::Arc;

use async_trait::async_trait;
use security_scan::{
    action_id, build_fix_plan, build_issue_plan, sanitize_github_artifact_url,
    validate_action_request, ActionEnqueueRequestV1, AnalysisConfigV1, CreateActionOutcome,
    CreateRunOutcome, EnqueueRequest, RepositoryConfigV1, RepositoryGitHubConfigV1, RunRecordV1,
    RunStatusV1, ScanModeV1, SecurityActionKindV1, SecurityActionRecordV1, SecurityActionStatusV1,
    SecurityAssessmentsV1, SecurityFindingV1, SecurityReportV1, SecurityRuntime,
    SecurityScanActionRequestV1, SecurityScanError, SecurityScanService, SeverityV1, WorkerConfig,
    ACTION_DENIED_FUNCTIONS, FIX_ACTION_FUNCTIONS, ISSUE_ACTION_FUNCTIONS,
};
use tokio::sync::Mutex;

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

fn finding(patch: Option<&str>) -> SecurityFindingV1 {
    SecurityFindingV1 {
        rule_id: "SEC-001".into(),
        severity: SeverityV1::High,
        title: "Unsafe default".into(),
        description: "Details".into(),
        evidence: "Evidence".into(),
        location: None,
        remediation: "Fix it".into(),
        suggested_patch: patch.map(str::to_string),
    }
}

fn completed_run(mode: ScanModeV1, patch: Option<&str>) -> RunRecordV1 {
    RunRecordV1 {
        schema_version: "1".into(),
        run_id: "sec_completed".into(),
        repository: "iii-hq/iii".into(),
        target_sha: "0123456789abcdef0123456789abcdef01234567".into(),
        resolved_from_head: false,
        mode,
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
            findings: vec![finding(patch)],
        }),
        error: None,
        created_at: 1,
        updated_at: 2,
        completed_at: Some(2),
    }
}

fn queued_action() -> SecurityActionRecordV1 {
    SecurityActionRecordV1 {
        schema_version: "1".into(),
        action_id: "seca_issue".into(),
        run_id: "sec_completed".into(),
        finding_index: 0,
        action: SecurityActionKindV1::Issue,
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

#[test]
fn action_ids_are_deterministic_for_run_finding_and_kind() {
    let first = action_id("sec_completed", 0, SecurityActionKindV1::Issue);
    let second = action_id("sec_completed", 0, SecurityActionKindV1::Issue);
    let other_finding = action_id("sec_completed", 1, SecurityActionKindV1::Issue);
    let other_kind = action_id("sec_completed", 0, SecurityActionKindV1::FixPr);
    assert_eq!(first, second);
    assert_ne!(first, other_finding);
    assert_ne!(first, other_kind);
    assert!(first.starts_with("seca_"));
}

#[test]
fn issue_plan_allows_only_github_issue_create() {
    let plan = build_issue_plan(&queued_action(), &finding(None), &analysis_config());
    assert_eq!(
        plan.allowed_functions,
        ISSUE_ACTION_FUNCTIONS
            .iter()
            .map(|function| (*function).to_string())
            .collect::<Vec<_>>()
    );
    assert!(plan
        .allowed_functions
        .iter()
        .all(|function| function == "github::issue::create"));
    for denied in ACTION_DENIED_FUNCTIONS {
        assert!(plan
            .denied_functions
            .iter()
            .any(|function| function == denied));
    }
    assert!(!plan.unattended);
    assert!(plan.filesystem_root.is_empty());
    assert!(plan
        .system_prompt
        .contains("github::issue::create exactly once"));
}

#[test]
fn fix_plan_uses_exact_sha_worktree_and_draft_pr_policy() {
    let mut action = queued_action();
    action.action = SecurityActionKindV1::FixPr;
    let plan = build_fix_plan(
        &action,
        &finding(Some("diff --git a/x b/x")),
        "/private/tmp/wt_fix",
        &analysis_config(),
    );
    assert!(!plan.unattended);
    assert_eq!(plan.filesystem_root, "/private/tmp/wt_fix");
    assert_eq!(
        plan.allowed_functions,
        FIX_ACTION_FUNCTIONS
            .iter()
            .map(|function| (*function).to_string())
            .collect::<Vec<_>>()
    );
    assert!(plan
        .allowed_functions
        .iter()
        .any(|function| function == "github::pr::create"));
    assert!(plan
        .allowed_functions
        .iter()
        .any(|function| function == "coder::update-file"));
    assert!(!plan
        .allowed_functions
        .iter()
        .any(|function| function == "github::pr::merge"));
    assert!(plan
        .denied_functions
        .iter()
        .any(|function| function == "github::pr::merge"));
    assert!(!plan
        .allowed_functions
        .iter()
        .any(|function| function == "shell::exec" || function == "editor::git::commit"));
    assert!(plan
        .allowed_functions
        .iter()
        .any(|function| function == "security-scan::action-commit"));
    assert!(plan
        .message
        .contains("0123456789abcdef0123456789abcdef01234567"));
    assert!(plan.system_prompt.contains("draft=true"));
    assert!(plan.system_prompt.contains("Never merge"));
}

#[test]
fn github_artifact_urls_are_https_github_only() {
    assert_eq!(
        sanitize_github_artifact_url(
            "https://github.com/iii-hq/iii/issues/12",
            SecurityActionKindV1::Issue,
            "iii-hq/iii"
        )
        .unwrap(),
        "https://github.com/iii-hq/iii/issues/12"
    );
    assert_eq!(
        sanitize_github_artifact_url(
            "https://www.github.com/iii-hq/iii/pull/9",
            SecurityActionKindV1::FixPr,
            "iii-hq/iii"
        )
        .unwrap(),
        "https://github.com/iii-hq/iii/pull/9"
    );
    assert!(sanitize_github_artifact_url(
        "javascript:alert(1)",
        SecurityActionKindV1::Issue,
        "iii-hq/iii"
    )
    .is_err());
    assert!(sanitize_github_artifact_url(
        "https://evil.example/iii-hq/iii/issues/12",
        SecurityActionKindV1::Issue,
        "iii-hq/iii"
    )
    .is_err());
    assert!(sanitize_github_artifact_url(
        "https://user:token@github.com/iii-hq/iii/issues/12",
        SecurityActionKindV1::Issue,
        "iii-hq/iii"
    )
    .is_err());
    assert!(sanitize_github_artifact_url(
        "https://github.com/iii-hq/iii/pull/9",
        SecurityActionKindV1::Issue,
        "iii-hq/iii"
    )
    .is_err());
    assert!(sanitize_github_artifact_url(
        "https://github.com/other/repo/issues/12",
        SecurityActionKindV1::Issue,
        "iii-hq/iii"
    )
    .is_err());
}

#[test]
fn fix_pr_requires_suggest_mode_and_a_patch() {
    assert!(validate_action_request(
        &completed_run(ScanModeV1::Scan, Some("patch")),
        0,
        SecurityActionKindV1::FixPr
    )
    .is_err());
    assert!(validate_action_request(
        &completed_run(ScanModeV1::Suggest, None),
        0,
        SecurityActionKindV1::FixPr
    )
    .is_err());
    assert!(validate_action_request(
        &completed_run(ScanModeV1::Suggest, Some("patch")),
        0,
        SecurityActionKindV1::FixPr
    )
    .is_ok());
    assert!(validate_action_request(
        &completed_run(ScanModeV1::Scan, None),
        0,
        SecurityActionKindV1::Issue
    )
    .is_ok());
}

#[test]
fn fix_pr_results_must_be_drafts_with_sanitized_urls() {
    use security_scan::{result_from_output, ActionHarnessOutputV1};
    let output = ActionHarnessOutputV1 {
        url: "https://github.com/iii-hq/iii/pull/4".into(),
        title: None,
        branch: Some("fix/sec-001".into()),
        commit_sha: Some("0123456789abcdef0123456789abcdef01234567".into()),
        draft: Some(true),
        validation: Some("applied suggested patch".into()),
    };
    let result =
        result_from_output(SecurityActionKindV1::FixPr, "iii-hq/iii", output.clone()).unwrap();
    assert_eq!(result.url, "https://github.com/iii-hq/iii/pull/4");
    assert_eq!(result.draft, Some(true));

    let mut merged = output;
    merged.draft = Some(false);
    assert!(result_from_output(SecurityActionKindV1::FixPr, "iii-hq/iii", merged).is_err());
}

struct ActionFake {
    ready: bool,
    gate_live: bool,
    run: RunRecordV1,
    action: Mutex<Option<SecurityActionRecordV1>>,
    enqueued: Mutex<Vec<ActionEnqueueRequestV1>>,
}

fn service(fake: Arc<ActionFake>) -> SecurityScanService<ActionFake> {
    SecurityScanService::new(
        fake,
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
        },
    )
}

#[async_trait]
impl SecurityRuntime for ActionFake {
    fn require_ready(&self) -> Result<(), SecurityScanError> {
        if self.ready {
            Ok(())
        } else {
            Err(SecurityScanError::Dependency(
                "security-scan private state is not ready".into(),
            ))
        }
    }

    async fn get_run(&self, run_id: &str) -> Result<Option<RunRecordV1>, SecurityScanError> {
        Ok((self.run.run_id == run_id).then(|| self.run.clone()))
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
        Ok(false)
    }

    async fn delete_run_if_unchanged(&self, _run: &RunRecordV1) -> Result<(), SecurityScanError> {
        Ok(())
    }

    async fn enqueue_execute(&self, _request: EnqueueRequest) -> Result<(), SecurityScanError> {
        Ok(())
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
        Ok(false)
    }

    async fn get_action(
        &self,
        action_id: &str,
    ) -> Result<Option<SecurityActionRecordV1>, SecurityScanError> {
        Ok(self
            .action
            .lock()
            .await
            .clone()
            .filter(|action| action.action_id == action_id))
    }

    async fn create_action_if_absent(
        &self,
        action: SecurityActionRecordV1,
    ) -> Result<CreateActionOutcome, SecurityScanError> {
        let mut current = self.action.lock().await;
        if let Some(existing) = current.as_ref() {
            return Ok(CreateActionOutcome::Existing(Box::new(existing.clone())));
        }
        *current = Some(action);
        Ok(CreateActionOutcome::Created)
    }

    async fn list_actions(&self) -> Result<Vec<SecurityActionRecordV1>, SecurityScanError> {
        Ok(self.action.lock().await.clone().into_iter().collect())
    }

    async fn replace_action(
        &self,
        expected: &SecurityActionRecordV1,
        replacement: SecurityActionRecordV1,
    ) -> Result<bool, SecurityScanError> {
        let mut current = self.action.lock().await;
        if current.as_ref() != Some(expected) {
            return Ok(false);
        }
        *current = Some(replacement);
        Ok(true)
    }

    async fn delete_action_if_unchanged(
        &self,
        action: &SecurityActionRecordV1,
    ) -> Result<(), SecurityScanError> {
        let mut current = self.action.lock().await;
        if current.as_ref() == Some(action) {
            *current = None;
        }
        Ok(())
    }

    async fn enqueue_action_execute(
        &self,
        request: ActionEnqueueRequestV1,
    ) -> Result<(), SecurityScanError> {
        self.enqueued.lock().await.push(request);
        Ok(())
    }

    async fn approval_gate_is_live(&self) -> Result<bool, SecurityScanError> {
        Ok(self.gate_live)
    }
}

#[tokio::test]
async fn public_calls_fail_closed_until_private_state_is_ready() {
    let fake = Arc::new(ActionFake {
        ready: false,
        gate_live: true,
        run: completed_run(ScanModeV1::Scan, None),
        action: Mutex::new(None),
        enqueued: Mutex::new(Vec::new()),
    });
    let error = service(fake)
        .action(SecurityScanActionRequestV1::new(
            "sec_completed".into(),
            0,
            SecurityActionKindV1::Issue,
        ))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not ready"));
}

#[tokio::test]
async fn duplicate_action_requests_return_the_same_id() {
    let fake = Arc::new(ActionFake {
        ready: true,
        gate_live: true,
        run: completed_run(ScanModeV1::Scan, None),
        action: Mutex::new(None),
        enqueued: Mutex::new(Vec::new()),
    });
    let svc = service(fake.clone());
    let first = svc
        .action(SecurityScanActionRequestV1::new(
            "sec_completed".into(),
            0,
            SecurityActionKindV1::Issue,
        ))
        .await
        .unwrap();
    let second = svc
        .action(SecurityScanActionRequestV1::new(
            "sec_completed".into(),
            0,
            SecurityActionKindV1::Issue,
        ))
        .await
        .unwrap();
    assert!(!first.deduplicated);
    assert!(second.deduplicated);
    assert_eq!(first.action_id, second.action_id);
    assert_eq!(fake.enqueued.lock().await.len(), 1);
}

#[tokio::test]
async fn retry_does_not_orphan_an_uncleaned_action_worktree() {
    let run = completed_run(ScanModeV1::Suggest, Some("patch"));
    let mut action = queued_action();
    action.action = SecurityActionKindV1::FixPr;
    action.action_id = action_id(&run.run_id, 0, SecurityActionKindV1::FixPr);
    action.status = SecurityActionStatusV1::Failed;
    action.error = Some(security_scan::RunErrorV1 {
        code: "temporary_failure".into(),
        message: "retry later".into(),
        retryable: true,
    });
    action.materialized = Some(security_scan::MaterializedTargetV1 {
        worktree_id: "wt_preserved".into(),
        path: "/private/wt_preserved".into(),
        base_sha: action.target_sha.clone(),
    });
    let fake = Arc::new(ActionFake {
        ready: true,
        gate_live: true,
        run,
        action: Mutex::new(Some(action.clone())),
        enqueued: Mutex::new(Vec::new()),
    });

    let response = service(fake.clone())
        .action(SecurityScanActionRequestV1::new(
            "sec_completed".into(),
            0,
            SecurityActionKindV1::FixPr,
        ))
        .await
        .unwrap();

    assert!(response.deduplicated);
    assert!(fake.enqueued.lock().await.is_empty());
    let stored = fake.action.lock().await.clone().unwrap();
    assert_eq!(stored.attempt, 1);
    assert_eq!(stored.materialized.unwrap().worktree_id, "wt_preserved");
    assert!(stored.cleanup_completed_at.is_none());
}
