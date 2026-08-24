use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::SecurityScanError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScanModeV1 {
    Scan,
    Suggest,
}

impl ScanModeV1 {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::Suggest => "suggest",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecurityScanRequestV1 {
    pub repository: String,
    pub target_sha: String,
    pub mode: ScanModeV1,
    /// Metadata injected by the iii engine. It is accepted on the wire but is
    /// not part of the public function schema or the request identity.
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    _caller_worker_id: Option<String>,
}

impl SecurityScanRequestV1 {
    pub fn new(repository: String, target_sha: String, mode: ScanModeV1) -> Self {
        Self {
            repository,
            target_sha,
            mode,
            _caller_worker_id: None,
        }
    }

    pub(crate) fn normalize(mut self) -> Result<Self, SecurityScanError> {
        if self.target_sha.len() != 40
            || !self.target_sha.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(SecurityScanError::InvalidRequest(
                "target_sha must be an immutable 40-character Git commit SHA".into(),
            ));
        }
        self.target_sha.make_ascii_lowercase();
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStatusV1 {
    Queued,
    Materializing,
    Materialized,
    Dispatching,
    Analyzing,
    Completed,
    Failed,
    Cancelling,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaterializedTargetV1 {
    pub worktree_id: String,
    pub path: String,
    pub base_sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HarnessRunV1 {
    pub session_id: String,
    pub turn_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunErrorV1 {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunRecordV1 {
    pub schema_version: String,
    pub run_id: String,
    pub repository: String,
    pub target_sha: String,
    pub mode: ScanModeV1,
    /// Opaque private identity for dependency sessions. This field is not
    /// included in the public run projection.
    pub operation_nonce: String,
    pub status: RunStatusV1,
    pub attempt: u32,
    pub step: u64,
    #[serde(default)]
    pub step_failures: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materialized: Option<MaterializedTargetV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<HarnessRunV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<SecurityReportV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RunErrorV1>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecurityScanResponseV1 {
    pub run_id: String,
    pub status: RunStatusV1,
    pub deduplicated: bool,
}

/// Payload emitted by the iii cron trigger. Its values are observability data
/// only; the handler resolves all scan inputs from operator configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecurityScanScheduleEventV1 {
    pub trigger: String,
    pub job_id: String,
    pub scheduled_time: String,
    pub actual_time: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecurityScanScheduleResponseV1 {
    pub repository: String,
    pub target_sha: String,
    pub mode: ScanModeV1,
    pub run_id: String,
    pub status: RunStatusV1,
    pub deduplicated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecurityScanReadRequestV1 {
    pub run_id: String,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    _caller_worker_id: Option<String>,
}

impl SecurityScanReadRequestV1 {
    pub fn new(run_id: String) -> Self {
        Self {
            run_id,
            _caller_worker_id: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecurityScanListRequestV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<RunStatusV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    _caller_worker_id: Option<String>,
}

impl SecurityScanListRequestV1 {
    pub fn new(
        repository: Option<String>,
        status: Option<RunStatusV1>,
        limit: Option<u32>,
    ) -> Self {
        Self {
            repository,
            status,
            limit,
            _caller_worker_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublicRunV1 {
    pub schema_version: String,
    pub run_id: String,
    pub repository: String,
    pub target_sha: String,
    pub mode: ScanModeV1,
    pub status: RunStatusV1,
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<SecurityReportV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RunErrorV1>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
}

impl From<&RunRecordV1> for PublicRunV1 {
    fn from(run: &RunRecordV1) -> Self {
        Self {
            schema_version: run.schema_version.clone(),
            run_id: run.run_id.clone(),
            repository: run.repository.clone(),
            target_sha: run.target_sha.clone(),
            mode: run.mode,
            status: run.status,
            attempt: run.attempt,
            report: run.report.clone(),
            error: run.error.clone(),
            created_at: run.created_at,
            updated_at: run.updated_at,
            completed_at: run.completed_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublicRunSummaryV1 {
    pub run_id: String,
    pub repository: String,
    pub target_sha: String,
    pub mode: ScanModeV1,
    pub status: RunStatusV1,
    pub attempt: u32,
    pub finding_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RunErrorV1>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
}

impl From<&RunRecordV1> for PublicRunSummaryV1 {
    fn from(run: &RunRecordV1) -> Self {
        Self {
            run_id: run.run_id.clone(),
            repository: run.repository.clone(),
            target_sha: run.target_sha.clone(),
            mode: run.mode,
            status: run.status,
            attempt: run.attempt,
            finding_count: run
                .report
                .as_ref()
                .map(|report| u32::try_from(report.findings.len()).unwrap_or(u32::MAX))
                .unwrap_or(0),
            error: run.error.clone(),
            created_at: run.created_at,
            updated_at: run.updated_at,
            completed_at: run.completed_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecurityScanReadResponseV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<PublicRunV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecurityScanListResponseV1 {
    pub runs: Vec<PublicRunSummaryV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SeverityV1 {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationSourceV1 {
    Dependabot,
    CodeScanning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationLifecycleV1 {
    Open,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationScopeV1 {
    ExactCommit,
    RepositoryDefaultBranch,
    RepositorySnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationSourceStatusV1 {
    Complete,
    Partial,
    Unavailable,
    AuthenticationRequired,
    PermissionDenied,
    Disabled,
    NotConfigured,
    NotCollected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationHealthStatusV1 {
    Healthy,
    Warning,
    Error,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationSourceHealthV1 {
    pub status: ReconciliationHealthStatusV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationSourceSummaryV1 {
    pub source: ReconciliationSourceV1,
    pub status: ReconciliationSourceStatusV1,
    pub scope: ReconciliationScopeV1,
    /// Collection time in Unix milliseconds. Null means the source was not queried.
    pub collected_at: Option<i64>,
    /// Number of normalized records when collection returned usable data. Null
    /// is unavailable/not-collected and is deliberately distinct from zero.
    pub record_count: Option<u32>,
    pub health: ReconciliationSourceHealthV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationAlertV1 {
    pub source: ReconciliationSourceV1,
    pub number: u64,
    pub severity: SeverityV1,
    pub lifecycle: ReconciliationLifecycleV1,
    pub scope: ReconciliationScopeV1,
    pub title: String,
    pub description: String,
    /// Reconstructed public github.com URL. Dependency-provided URLs are never persisted.
    pub public_url: String,
    /// Exact source identifiers only, such as GHSA, CVE, or scanner rule IDs.
    #[serde(default)]
    pub structured_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HarnessReconciliationStatusV1 {
    Verified,
    NotAvailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HarnessReconciliationSummaryV1 {
    pub status: HarnessReconciliationStatusV1,
    /// Validated Harness report findings. This is never added to GitHub source counts.
    pub verified_count: Option<u32>,
    pub verified_at: Option<i64>,
    pub scope: ReconciliationScopeV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationMatchingStatusV1 {
    Available,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationMatchingV1 {
    pub status: ReconciliationMatchingStatusV1,
    /// Present only when exact structured identifiers produced matches.
    pub matched_records: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecurityScanReconciliationRequestV1 {
    pub run_id: String,
    #[serde(default)]
    pub refresh: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ReconciliationSourceV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<SeverityV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<ReconciliationLifecycleV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    _caller_worker_id: Option<String>,
}

impl SecurityScanReconciliationRequestV1 {
    pub fn new(run_id: String) -> Self {
        Self {
            run_id,
            ..Self::default()
        }
    }
}

/// Durable, sanitized reconciliation snapshot stored outside the Harness run record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationSnapshotV1 {
    pub schema_version: String,
    pub run_id: String,
    pub repository: String,
    pub target_sha: String,
    pub harness: HarnessReconciliationSummaryV1,
    pub github_repository: Option<String>,
    pub sources: Vec<ReconciliationSourceSummaryV1>,
    pub matching: ReconciliationMatchingV1,
    pub records: Vec<ReconciliationAlertV1>,
}

/// One source collection returned by the runtime before snapshot persistence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationSourceCollectionV1 {
    pub summary: ReconciliationSourceSummaryV1,
    pub records: Vec<ReconciliationAlertV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecurityScanReconciliationResponseV1 {
    pub schema_version: String,
    pub run_id: String,
    pub repository: String,
    pub target_sha: String,
    pub harness: HarnessReconciliationSummaryV1,
    pub github_repository: Option<String>,
    pub sources: Vec<ReconciliationSourceSummaryV1>,
    pub matching: ReconciliationMatchingV1,
    pub records: Vec<ReconciliationAlertV1>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentStatusV1 {
    Assessed,
    NotAssessed,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecurityAreaAssessmentV1 {
    pub status: AssessmentStatusV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecurityAssessmentsV1 {
    pub vulnerabilities: SecurityAreaAssessmentV1,
    pub dependencies: SecurityAreaAssessmentV1,
    pub secrets: SecurityAreaAssessmentV1,
    pub supply_chain: SecurityAreaAssessmentV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindingLocationV1 {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_start: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_end: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecurityFindingV1 {
    pub rule_id: String,
    pub severity: SeverityV1,
    pub title: String,
    pub description: String,
    pub evidence: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<FindingLocationV1>,
    pub remediation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_patch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecurityReportV1 {
    pub summary: String,
    pub assessments: SecurityAssessmentsV1,
    pub findings: Vec<SecurityFindingV1>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SecurityReportWireV1 {
    summary: String,
    /// Older persisted reports predate explicit coverage. They deserialize as
    /// unknown, while newly submitted reports are rejected unless every area
    /// carries an assessed or not_assessed status.
    #[serde(default)]
    assessments: SecurityAssessmentsV1,
    findings: Vec<SecurityFindingV1>,
}

impl<'de> Deserialize<'de> for SecurityReportV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = SecurityReportWireV1::deserialize(deserializer)?;
        Ok(Self {
            summary: wire.summary,
            assessments: wire.assessments,
            findings: wire.findings,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnqueueRequest {
    pub run_id: String,
    pub repository: String,
    pub attempt: u32,
    pub step: u64,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    _caller_worker_id: Option<String>,
}

impl EnqueueRequest {
    pub fn new(run_id: String, repository: String, attempt: u32, step: u64) -> Self {
        Self {
            run_id,
            repository,
            attempt,
            step,
            _caller_worker_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecuteResponseV1 {
    pub skipped: bool,
    pub status: RunStatusV1,
    pub step: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, JsonSchema)]
pub struct TurnCompletedEventV1 {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub turn_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub terminal: bool,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub result_error: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TurnCompletedResponseV1 {
    pub woke: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<RunStatusV1>,
}
