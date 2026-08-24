use std::{collections::HashSet, sync::Arc};

use async_trait::async_trait;
use security_scan::{
    AnalysisConfigV1, AssessmentStatusV1, CreateRunOutcome, EnqueueRequest,
    HarnessReconciliationStatusV1, HarnessReconciliationSummaryV1, ReconciliationAlertV1,
    ReconciliationHealthStatusV1, ReconciliationLifecycleV1, ReconciliationMatchingStatusV1,
    ReconciliationMatchingV1, ReconciliationScopeV1, ReconciliationSnapshotV1,
    ReconciliationSourceCollectionV1, ReconciliationSourceHealthV1, ReconciliationSourceStatusV1,
    ReconciliationSourceSummaryV1, ReconciliationSourceV1, RepositoryConfigV1,
    RepositoryGitHubConfigV1, RunRecordV1, RunStatusV1, ScanModeV1, SecurityAssessmentsV1,
    SecurityFindingV1, SecurityReportV1, SecurityRuntime, SecurityScanError,
    SecurityScanReadRequestV1, SecurityScanReconciliationRequestV1,
    SecurityScanReconciliationResponseV1, SecurityScanService, SeverityV1, WorkerConfig,
};
use serde_json::Value;
use tokio::sync::Mutex;

struct FakeRuntime {
    run: RunRecordV1,
    snapshot: Mutex<Option<ReconciliationSnapshotV1>>,
    collections: Vec<ReconciliationSourceCollectionV1>,
    failing_sources: HashSet<ReconciliationSourceV1>,
    collected_sources: Mutex<Vec<ReconciliationSourceV1>>,
}

fn runtime(
    run: RunRecordV1,
    snapshot: Option<ReconciliationSnapshotV1>,
    collections: Vec<ReconciliationSourceCollectionV1>,
    failing_sources: impl IntoIterator<Item = ReconciliationSourceV1>,
) -> Arc<FakeRuntime> {
    Arc::new(FakeRuntime {
        run,
        snapshot: Mutex::new(snapshot),
        collections,
        failing_sources: failing_sources.into_iter().collect(),
        collected_sources: Mutex::new(Vec::new()),
    })
}

#[async_trait]
impl SecurityRuntime for FakeRuntime {
    async fn get_run(&self, run_id: &str) -> Result<Option<RunRecordV1>, SecurityScanError> {
        Ok((self.run.run_id == run_id).then(|| self.run.clone()))
    }

    async fn get_reconciliation_snapshot(
        &self,
        run_id: &str,
    ) -> Result<Option<ReconciliationSnapshotV1>, SecurityScanError> {
        Ok(self
            .snapshot
            .lock()
            .await
            .clone()
            .filter(|snapshot| snapshot.run_id == run_id))
    }

    async fn save_reconciliation_snapshot(
        &self,
        snapshot: ReconciliationSnapshotV1,
    ) -> Result<(), SecurityScanError> {
        *self.snapshot.lock().await = Some(snapshot);
        Ok(())
    }

    async fn collect_reconciliation_source(
        &self,
        source: ReconciliationSourceV1,
        github_full_name: &str,
        target_sha: &str,
        _collected_at: i64,
    ) -> Result<ReconciliationSourceCollectionV1, SecurityScanError> {
        assert_eq!(github_full_name, "iii-hq/iii");
        assert_eq!(target_sha, self.run.target_sha);
        self.collected_sources.lock().await.push(source);
        if self.failing_sources.contains(&source) {
            return Err(SecurityScanError::Dependency(format!(
                "{source:?} unavailable"
            )));
        }
        self.collections
            .iter()
            .find(|collection| collection.summary.source == source)
            .cloned()
            .ok_or_else(|| SecurityScanError::Dependency(format!("missing {source:?} fixture")))
    }

    async fn create_run_if_absent(
        &self,
        _run: RunRecordV1,
    ) -> Result<CreateRunOutcome, SecurityScanError> {
        unreachable!("reconciliation does not create runs")
    }

    async fn replace_run(
        &self,
        _expected: &RunRecordV1,
        _replacement: RunRecordV1,
    ) -> Result<bool, SecurityScanError> {
        unreachable!("reconciliation does not replace runs")
    }

    async fn delete_run_if_unchanged(&self, _run: &RunRecordV1) -> Result<(), SecurityScanError> {
        unreachable!("reconciliation does not delete runs")
    }

    async fn enqueue_execute(&self, _request: EnqueueRequest) -> Result<(), SecurityScanError> {
        unreachable!("reconciliation does not enqueue runs")
    }
}

fn config(github: bool) -> WorkerConfig {
    WorkerConfig {
        repositories: vec![RepositoryConfigV1 {
            id: "iii-hq/iii".into(),
            path: "/srv/repos/iii".into(),
            github: github.then(|| RepositoryGitHubConfigV1 {
                full_name: "iii-hq/iii".into(),
            }),
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
    }
}

fn completed_run(finding_count: usize) -> RunRecordV1 {
    let findings = (0..finding_count)
        .map(|index| SecurityFindingV1 {
            rule_id: if index == 0 {
                "GHSA-model-authored".into()
            } else {
                format!("HARNESS-{index}")
            },
            severity: SeverityV1::High,
            title: format!("Harness finding {index}"),
            description: "Validated by Harness".into(),
            evidence: "Exact-commit evidence".into(),
            location: None,
            remediation: "Apply a bounded fix".into(),
            suggested_patch: None,
        })
        .collect();
    RunRecordV1 {
        schema_version: "1".into(),
        run_id: "sec_reconciliation".into(),
        repository: "iii-hq/iii".into(),
        target_sha: "0123456789abcdef0123456789abcdef01234567".into(),
        mode: ScanModeV1::Scan,
        operation_nonce: "private_state_nonce".into(),
        status: RunStatusV1::Completed,
        attempt: 1,
        step: 2,
        step_failures: 0,
        materialized: None,
        harness: None,
        report: Some(SecurityReportV1 {
            summary: "Harness report".into(),
            assessments: SecurityAssessmentsV1::default(),
            findings,
        }),
        error: None,
        created_at: 100,
        updated_at: 200,
        completed_at: Some(200),
    }
}

fn source_scope(source: ReconciliationSourceV1) -> ReconciliationScopeV1 {
    match source {
        ReconciliationSourceV1::Dependabot => ReconciliationScopeV1::RepositoryDefaultBranch,
        ReconciliationSourceV1::CodeScanning => ReconciliationScopeV1::RepositorySnapshot,
    }
}

fn summary(
    source: ReconciliationSourceV1,
    status: ReconciliationSourceStatusV1,
    record_count: Option<u32>,
) -> ReconciliationSourceSummaryV1 {
    ReconciliationSourceSummaryV1 {
        source,
        status,
        scope: source_scope(source),
        collected_at: (!matches!(
            status,
            ReconciliationSourceStatusV1::NotCollected
                | ReconciliationSourceStatusV1::NotConfigured
        ))
        .then_some(300),
        record_count,
        health: ReconciliationSourceHealthV1 {
            status: if status == ReconciliationSourceStatusV1::Complete {
                ReconciliationHealthStatusV1::Healthy
            } else {
                ReconciliationHealthStatusV1::Warning
            },
            tool: None,
            commit_sha: None,
            observed_at: None,
        },
    }
}

fn alert(
    source: ReconciliationSourceV1,
    number: u64,
    severity: SeverityV1,
) -> ReconciliationAlertV1 {
    ReconciliationAlertV1 {
        source,
        number,
        severity,
        lifecycle: ReconciliationLifecycleV1::Open,
        scope: source_scope(source),
        title: format!("{source:?} alert {number}"),
        description: "Normalized GitHub alert".into(),
        public_url: format!("https://github.com/iii-hq/iii/security/alert/{number}"),
        structured_ids: Vec::new(),
        path: None,
        start_line: None,
        end_line: None,
        observed_at: None,
    }
}

fn collection(
    source: ReconciliationSourceV1,
    status: ReconciliationSourceStatusV1,
    record_count: Option<u32>,
    records: Vec<ReconciliationAlertV1>,
) -> ReconciliationSourceCollectionV1 {
    ReconciliationSourceCollectionV1 {
        summary: summary(source, status, record_count),
        records,
    }
}

fn persisted_snapshot(
    run: &RunRecordV1,
    records: Vec<ReconciliationAlertV1>,
) -> ReconciliationSnapshotV1 {
    let count = |source| {
        u32::try_from(
            records
                .iter()
                .filter(|record| record.source == source)
                .count(),
        )
        .unwrap()
    };
    ReconciliationSnapshotV1 {
        schema_version: "1".into(),
        run_id: run.run_id.clone(),
        repository: run.repository.clone(),
        target_sha: run.target_sha.clone(),
        harness: HarnessReconciliationSummaryV1 {
            status: HarnessReconciliationStatusV1::Verified,
            verified_count: Some(
                u32::try_from(run.report.as_ref().unwrap().findings.len()).unwrap(),
            ),
            verified_at: run.completed_at,
            scope: ReconciliationScopeV1::ExactCommit,
        },
        github_repository: Some("iii-hq/iii".into()),
        sources: [
            ReconciliationSourceV1::Dependabot,
            ReconciliationSourceV1::CodeScanning,
        ]
        .into_iter()
        .map(|source| {
            summary(
                source,
                ReconciliationSourceStatusV1::Complete,
                Some(count(source)),
            )
        })
        .collect(),
        matching: ReconciliationMatchingV1 {
            status: ReconciliationMatchingStatusV1::Unavailable,
            matched_records: None,
        },
        records,
    }
}

fn source_summary(
    response: &SecurityScanReconciliationResponseV1,
    source: ReconciliationSourceV1,
) -> &ReconciliationSourceSummaryV1 {
    response
        .sources
        .iter()
        .find(|summary| summary.source == source)
        .expect("source summary")
}

#[tokio::test]
async fn harness_and_github_counts_remain_non_additive_and_alerts_dedupe_by_source_number() {
    let run = completed_run(3);
    let mut dependabot = (1..=111)
        .map(|number| alert(ReconciliationSourceV1::Dependabot, number, SeverityV1::High))
        .collect::<Vec<_>>();
    dependabot.push(alert(
        ReconciliationSourceV1::Dependabot,
        1,
        SeverityV1::Critical,
    ));
    let mut code_scanning = (1..=110)
        .map(|number| {
            alert(
                ReconciliationSourceV1::CodeScanning,
                number,
                SeverityV1::Medium,
            )
        })
        .collect::<Vec<_>>();
    code_scanning[0].structured_ids = vec!["GHSA-model-authored".into()];

    let mut code_summary = summary(
        ReconciliationSourceV1::CodeScanning,
        ReconciliationSourceStatusV1::Complete,
        Some(110),
    );
    code_summary.health.commit_sha = Some("f".repeat(40));
    let runtime = runtime(
        run,
        None,
        vec![
            collection(
                ReconciliationSourceV1::Dependabot,
                ReconciliationSourceStatusV1::Complete,
                Some(112),
                dependabot,
            ),
            ReconciliationSourceCollectionV1 {
                summary: code_summary,
                records: code_scanning,
            },
        ],
        [],
    );
    let service = SecurityScanService::new(runtime.clone(), config(true));
    let mut request = SecurityScanReconciliationRequestV1::new("sec_reconciliation".into());
    request.refresh = true;
    request.limit = Some(200);

    let first = service.reconciliation(request).await.unwrap();

    assert_eq!(
        first.harness.status,
        HarnessReconciliationStatusV1::Verified
    );
    assert_eq!(first.harness.verified_count, Some(3));
    assert_eq!(first.harness.scope, ReconciliationScopeV1::ExactCommit);
    assert_eq!(first.records.len(), 200);
    assert_eq!(first.next_cursor.as_deref(), Some("v1:200"));
    assert_eq!(
        first
            .sources
            .iter()
            .map(|source| source.record_count.unwrap())
            .sum::<u32>(),
        221
    );
    assert_eq!(
        first.matching.status,
        ReconciliationMatchingStatusV1::Unavailable
    );
    assert_eq!(first.matching.matched_records, None);
    let code = source_summary(&first, ReconciliationSourceV1::CodeScanning);
    assert_eq!(code.scope, ReconciliationScopeV1::RepositorySnapshot);
    assert_eq!(
        code.health.commit_sha.as_deref(),
        Some("ffffffffffffffffffffffffffffffffffffffff")
    );
    assert_ne!(
        code.health.commit_sha.as_deref(),
        Some(first.target_sha.as_str())
    );
    assert!(first
        .records
        .iter()
        .filter(|record| record.source == ReconciliationSourceV1::CodeScanning)
        .all(|record| record.scope == ReconciliationScopeV1::RepositorySnapshot));

    let mut next = SecurityScanReconciliationRequestV1::new("sec_reconciliation".into());
    next.cursor = first.next_cursor.clone();
    next.limit = Some(200);
    let second = service.reconciliation(next).await.unwrap();
    assert_eq!(second.records.len(), 21);
    assert!(second.next_cursor.is_none());

    let unique = first
        .records
        .iter()
        .chain(&second.records)
        .map(|record| (record.source, record.number))
        .collect::<HashSet<_>>();
    assert_eq!(unique.len(), 221);
    let cached = runtime.snapshot.lock().await.clone().unwrap();
    assert_eq!(cached.records.len(), 221);
    assert_eq!(cached.harness.verified_count, Some(3));
    let collected = runtime.collected_sources.lock().await.clone();
    assert_eq!(collected.len(), 2);
    assert_eq!(
        collected.into_iter().collect::<HashSet<_>>(),
        HashSet::from([
            ReconciliationSourceV1::Dependabot,
            ReconciliationSourceV1::CodeScanning,
        ])
    );
    let encoded = serde_json::to_value(&cached).unwrap();
    assert!(encoded.get("total_count").is_none());
    assert!(encoded.get("unique_count").is_none());
    assert_no_internal_keys(&encoded);

    let default_page = service
        .reconciliation(SecurityScanReconciliationRequestV1::new(
            "sec_reconciliation".into(),
        ))
        .await
        .unwrap();
    assert_eq!(default_page.records.len(), 50);
    assert_eq!(default_page.next_cursor.as_deref(), Some("v1:50"));
    assert_eq!(runtime.collected_sources.lock().await.len(), 2);
}

#[tokio::test]
async fn complete_zero_is_distinct_from_every_non_complete_source_state() {
    let run = completed_run(0);

    let not_collected =
        SecurityScanService::new(runtime(run.clone(), None, Vec::new(), []), config(true))
            .reconciliation(SecurityScanReconciliationRequestV1::new(run.run_id.clone()))
            .await
            .unwrap();
    assert!(not_collected.sources.iter().all(|source| {
        source.status == ReconciliationSourceStatusV1::NotCollected
            && source.record_count.is_none()
            && source.collected_at.is_none()
    }));

    let not_configured =
        SecurityScanService::new(runtime(run.clone(), None, Vec::new(), []), config(false))
            .reconciliation(SecurityScanReconciliationRequestV1::new(run.run_id.clone()))
            .await
            .unwrap();
    assert!(not_configured.sources.iter().all(|source| {
        source.status == ReconciliationSourceStatusV1::NotConfigured
            && source.record_count.is_none()
    }));

    let cases = [
        (
            ReconciliationSourceStatusV1::Complete,
            Some(0),
            ReconciliationSourceStatusV1::AuthenticationRequired,
            None,
        ),
        (
            ReconciliationSourceStatusV1::PermissionDenied,
            None,
            ReconciliationSourceStatusV1::Disabled,
            None,
        ),
    ];
    for (dependabot_status, dependabot_count, code_status, code_count) in cases {
        let current = runtime(
            run.clone(),
            None,
            vec![
                collection(
                    ReconciliationSourceV1::Dependabot,
                    dependabot_status,
                    dependabot_count,
                    Vec::new(),
                ),
                collection(
                    ReconciliationSourceV1::CodeScanning,
                    code_status,
                    code_count,
                    Vec::new(),
                ),
            ],
            [],
        );
        let service = SecurityScanService::new(current, config(true));
        let mut request = SecurityScanReconciliationRequestV1::new(run.run_id.clone());
        request.refresh = true;
        let response = service.reconciliation(request).await.unwrap();
        assert_eq!(
            source_summary(&response, ReconciliationSourceV1::Dependabot).status,
            dependabot_status
        );
        assert_eq!(
            source_summary(&response, ReconciliationSourceV1::Dependabot).record_count,
            dependabot_count
        );
        assert_eq!(
            source_summary(&response, ReconciliationSourceV1::CodeScanning).status,
            code_status
        );
        assert_eq!(
            source_summary(&response, ReconciliationSourceV1::CodeScanning).record_count,
            code_count
        );
    }

    let partial = vec![alert(
        ReconciliationSourceV1::Dependabot,
        7,
        SeverityV1::Low,
    )];
    let current = runtime(
        run.clone(),
        None,
        vec![collection(
            ReconciliationSourceV1::Dependabot,
            ReconciliationSourceStatusV1::Partial,
            Some(1),
            partial,
        )],
        [ReconciliationSourceV1::CodeScanning],
    );
    let service = SecurityScanService::new(current, config(true));
    let mut request = SecurityScanReconciliationRequestV1::new(run.run_id);
    request.refresh = true;
    let response = service.reconciliation(request).await.unwrap();
    let partial = source_summary(&response, ReconciliationSourceV1::Dependabot);
    assert_eq!(partial.status, ReconciliationSourceStatusV1::Partial);
    assert_eq!(partial.record_count, Some(1));
    let unavailable = source_summary(&response, ReconciliationSourceV1::CodeScanning);
    assert_eq!(
        unavailable.status,
        ReconciliationSourceStatusV1::Unavailable
    );
    assert_eq!(unavailable.record_count, None);
}

#[tokio::test]
async fn filters_apply_before_cursor_and_cursor_and_limit_bounds_are_enforced() {
    let run = completed_run(0);
    let records = vec![
        alert(ReconciliationSourceV1::Dependabot, 2, SeverityV1::Low),
        alert(ReconciliationSourceV1::CodeScanning, 2, SeverityV1::Low),
        alert(ReconciliationSourceV1::Dependabot, 1, SeverityV1::High),
        alert(ReconciliationSourceV1::CodeScanning, 1, SeverityV1::High),
    ];
    let snapshot = persisted_snapshot(&run, records);
    let service = SecurityScanService::new(
        runtime(run.clone(), Some(snapshot), Vec::new(), []),
        config(true),
    );

    let mut first = SecurityScanReconciliationRequestV1::new(run.run_id.clone());
    first.source = Some(ReconciliationSourceV1::CodeScanning);
    first.lifecycle = Some(ReconciliationLifecycleV1::Open);
    first.limit = Some(1);
    let first = service.reconciliation(first).await.unwrap();
    assert_eq!(first.records[0].number, 1);
    assert_eq!(first.next_cursor.as_deref(), Some("v1:1"));

    let mut second = SecurityScanReconciliationRequestV1::new(run.run_id.clone());
    second.source = Some(ReconciliationSourceV1::CodeScanning);
    second.lifecycle = Some(ReconciliationLifecycleV1::Open);
    second.limit = Some(1);
    second.cursor = first.next_cursor;
    let second = service.reconciliation(second).await.unwrap();
    assert_eq!(second.records[0].number, 2);
    assert!(second.next_cursor.is_none());

    for limit in [0, 201] {
        let mut request = SecurityScanReconciliationRequestV1::new(run.run_id.clone());
        request.limit = Some(limit);
        assert!(matches!(
            service.reconciliation(request).await.unwrap_err(),
            SecurityScanError::InvalidRequest(_)
        ));
    }
    for cursor in [
        "",
        "v2:0",
        "v1:",
        "v1:-1",
        "v1:1x",
        "v1:999999999999999999999999999999999999999999",
    ] {
        let mut request = SecurityScanReconciliationRequestV1::new(run.run_id.clone());
        request.cursor = Some(cursor.into());
        assert!(matches!(
            service.reconciliation(request).await.unwrap_err(),
            SecurityScanError::InvalidRequest(_)
        ));
    }

    let mut beyond_filtered = SecurityScanReconciliationRequestV1::new(run.run_id);
    beyond_filtered.source = Some(ReconciliationSourceV1::CodeScanning);
    beyond_filtered.severity = Some(SeverityV1::Critical);
    beyond_filtered.cursor = Some("v1:1".into());
    assert!(matches!(
        service.reconciliation(beyond_filtered).await.unwrap_err(),
        SecurityScanError::InvalidRequest(_)
    ));
}

#[tokio::test]
async fn legacy_runs_without_assessments_remain_readable() {
    let run: RunRecordV1 = serde_json::from_value(serde_json::json!({
        "schema_version": "1",
        "run_id": "sec_reconciliation",
        "repository": "iii-hq/iii",
        "target_sha": "0123456789abcdef0123456789abcdef01234567",
        "mode": "scan",
        "operation_nonce": "legacy_private_nonce",
        "status": "completed",
        "attempt": 1,
        "step": 2,
        "report": {
            "summary": "Legacy report",
            "findings": [{
                "rule_id": "LEGACY-1",
                "severity": "high",
                "title": "Legacy finding",
                "description": "Persisted before coverage tracking",
                "evidence": "Legacy evidence",
                "remediation": "Legacy remediation"
            }]
        },
        "created_at": 100,
        "updated_at": 200,
        "completed_at": 200
    }))
    .unwrap();
    let service = SecurityScanService::new(runtime(run, None, Vec::new(), []), config(false));

    let response = service
        .read(SecurityScanReadRequestV1::new("sec_reconciliation".into()))
        .await
        .unwrap();
    let report = response.run.unwrap().report.unwrap();
    assert_eq!(report.findings.len(), 1);
    assert_eq!(
        report.assessments.vulnerabilities.status,
        AssessmentStatusV1::Unknown
    );
    assert_eq!(
        report.assessments.dependencies.status,
        AssessmentStatusV1::Unknown
    );
    assert_eq!(
        report.assessments.secrets.status,
        AssessmentStatusV1::Unknown
    );
    assert_eq!(
        report.assessments.supply_chain.status,
        AssessmentStatusV1::Unknown
    );
}

#[test]
fn reconciliation_wire_hides_engine_metadata_tokens_state_and_raw_payloads() {
    let request: SecurityScanReconciliationRequestV1 = serde_json::from_value(serde_json::json!({
        "run_id": "sec_reconciliation",
        "refresh": true,
        "limit": 25,
        "_caller_worker_id": "console"
    }))
    .unwrap();
    let encoded = serde_json::to_value(&request).unwrap();
    assert!(encoded.get("_caller_worker_id").is_none());
    let schema =
        serde_json::to_value(schemars::schema_for!(SecurityScanReconciliationRequestV1)).unwrap();
    assert!(schema["properties"].get("_caller_worker_id").is_none());

    let run = completed_run(1);
    let snapshot = persisted_snapshot(
        &run,
        vec![alert(
            ReconciliationSourceV1::Dependabot,
            1,
            SeverityV1::High,
        )],
    );
    assert_no_internal_keys(&serde_json::to_value(snapshot).unwrap());
}

fn assert_no_internal_keys(value: &Value) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                assert!(
                    !matches!(
                        key.as_str(),
                        "token"
                            | "tokens"
                            | "access_token"
                            | "authorization"
                            | "state"
                            | "state_key"
                            | "raw"
                            | "raw_payload"
                            | "payload"
                            | "operation_nonce"
                            | "session_id"
                            | "turn_id"
                    ),
                    "serialized reconciliation leaked private field {key}"
                );
                assert_no_internal_keys(value);
            }
        }
        Value::Array(values) => values.iter().for_each(assert_no_internal_keys),
        _ => {}
    }
}
