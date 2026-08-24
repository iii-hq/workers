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

#[async_trait]
pub trait SecurityRuntime: Send + Sync {
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
}
