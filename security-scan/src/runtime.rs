use async_trait::async_trait;

use crate::{
    EnqueueRequest, PublicRunSummaryV1, ReconciliationSnapshotV1, ReconciliationSourceCollectionV1,
    ReconciliationSourceV1, RunRecordV1, SecurityScanError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateRunOutcome {
    Created,
    Existing(Box<RunRecordV1>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateActionOutcome {
    Created,
    Existing(Box<crate::SecurityActionRecordV1>),
}

#[async_trait]
pub trait SecurityRuntime: Send + Sync {
    fn require_ready(&self) -> Result<(), SecurityScanError> {
        Ok(())
    }

    async fn get_run(&self, run_id: &str) -> Result<Option<RunRecordV1>, SecurityScanError>;

    async fn list_run_summaries(&self) -> Result<Vec<PublicRunSummaryV1>, SecurityScanError> {
        Err(SecurityScanError::Dependency(
            "runtime does not support listing security scan runs".into(),
        ))
    }

    async fn get_reconciliation_snapshot(
        &self,
        _run_id: &str,
    ) -> Result<Option<ReconciliationSnapshotV1>, SecurityScanError> {
        Ok(None)
    }

    async fn save_reconciliation_snapshot(
        &self,
        _snapshot: ReconciliationSnapshotV1,
    ) -> Result<(), SecurityScanError> {
        Ok(())
    }

    async fn collect_reconciliation_source(
        &self,
        source: ReconciliationSourceV1,
        _github_full_name: &str,
        _target_sha: &str,
        _collected_at: i64,
    ) -> Result<ReconciliationSourceCollectionV1, SecurityScanError> {
        Err(SecurityScanError::Dependency(format!(
            "runtime does not support {source:?} reconciliation"
        )))
    }

    async fn create_run_if_absent(
        &self,
        run: RunRecordV1,
    ) -> Result<CreateRunOutcome, SecurityScanError>;

    async fn replace_run(
        &self,
        expected: &RunRecordV1,
        replacement: RunRecordV1,
    ) -> Result<bool, SecurityScanError>;

    async fn delete_run_if_unchanged(&self, run: &RunRecordV1) -> Result<(), SecurityScanError>;

    async fn enqueue_execute(&self, request: EnqueueRequest) -> Result<(), SecurityScanError>;

    async fn stop_analysis(&self, harness: &crate::HarnessRunV1) -> Result<(), SecurityScanError>;

    async fn ensure_analysis_chat_link(&self, run: &RunRecordV1)
        -> Result<bool, SecurityScanError>;

    async fn resolve_target_ref(
        &self,
        repository: &crate::RepositoryConfigV1,
        target_ref: &str,
    ) -> Result<String, SecurityScanError> {
        crate::schedule::resolve_target_sha(repository, target_ref).await
    }

    async fn get_action(
        &self,
        action_id: &str,
    ) -> Result<Option<crate::SecurityActionRecordV1>, SecurityScanError>;

    async fn list_actions(&self) -> Result<Vec<crate::SecurityActionRecordV1>, SecurityScanError>;

    async fn create_action_if_absent(
        &self,
        action: crate::SecurityActionRecordV1,
    ) -> Result<crate::CreateActionOutcome, SecurityScanError>;

    async fn replace_action(
        &self,
        expected: &crate::SecurityActionRecordV1,
        replacement: crate::SecurityActionRecordV1,
    ) -> Result<bool, SecurityScanError>;

    async fn delete_action_if_unchanged(
        &self,
        action: &crate::SecurityActionRecordV1,
    ) -> Result<(), SecurityScanError>;

    async fn enqueue_action_execute(
        &self,
        request: crate::ActionEnqueueRequestV1,
    ) -> Result<(), SecurityScanError>;

    async fn approval_gate_is_live(&self) -> Result<bool, SecurityScanError>;
}
