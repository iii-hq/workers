use std::{collections::HashSet, sync::Arc};

use crate::{
    ids, CreateRunOutcome, EnqueueRequest, HarnessReconciliationStatusV1,
    HarnessReconciliationSummaryV1, ReconciliationHealthStatusV1, ReconciliationMatchingStatusV1,
    ReconciliationMatchingV1, ReconciliationScopeV1, ReconciliationSnapshotV1,
    ReconciliationSourceCollectionV1, ReconciliationSourceHealthV1, ReconciliationSourceStatusV1,
    ReconciliationSourceSummaryV1, ReconciliationSourceV1, RunRecordV1, RunStatusV1,
    SecurityRuntime, SecurityScanError, SecurityScanListRequestV1, SecurityScanListResponseV1,
    SecurityScanReadRequestV1, SecurityScanReadResponseV1, SecurityScanReconciliationRequestV1,
    SecurityScanReconciliationResponseV1, SecurityScanRequestV1, SecurityScanResponseV1,
    WorkerConfig,
};

const DEFAULT_LIST_LIMIT: u32 = 50;
const MAX_LIST_LIMIT: u32 = 200;
const DEFAULT_RECONCILIATION_LIMIT: u32 = 50;
const MAX_RECONCILIATION_LIMIT: u32 = 200;

pub struct SecurityScanService<R> {
    runtime: Arc<R>,
    config: WorkerConfig,
}

impl<R> SecurityScanService<R>
where
    R: SecurityRuntime,
{
    pub fn new(runtime: Arc<R>, config: WorkerConfig) -> Self {
        Self { runtime, config }
    }

    pub fn configured_repository_count(&self) -> usize {
        self.config.repositories.len()
    }

    pub async fn request(
        &self,
        request: SecurityScanRequestV1,
    ) -> Result<SecurityScanResponseV1, SecurityScanError> {
        self.runtime.require_ready()?;
        let mut request = request.normalize()?;
        let Some(repository) = self.config.repository(&request.repository).cloned() else {
            return Err(SecurityScanError::InvalidRequest(format!(
                "repository {} is not configured",
                request.repository
            )));
        };
        let resolved_from_head = request.target_sha.is_empty();
        if resolved_from_head {
            request.target_sha = self.runtime.resolve_target_ref(&repository, "HEAD").await?;
        }
        let model = request
            .model
            .clone()
            .unwrap_or_else(|| self.config.analysis.model.clone());
        let provider = if request.model.is_some() {
            request.provider.clone()
        } else {
            request
                .provider
                .clone()
                .or_else(|| self.config.analysis.provider.clone())
        };
        let now = ids::now_ms();
        let run = RunRecordV1 {
            schema_version: "1".into(),
            run_id: ids::run_id(&request, &model),
            repository: request.repository,
            target_sha: request.target_sha,
            resolved_from_head,
            mode: request.mode,
            model: Some(model),
            provider,
            operation_nonce: ids::operation_nonce(),
            status: RunStatusV1::Queued,
            attempt: 1,
            step: 0,
            step_failures: 0,
            materialized: None,
            harness: None,
            report: None,
            error: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
        };

        match self.runtime.create_run_if_absent(run.clone()).await? {
            CreateRunOutcome::Created => {
                self.enqueue(&run).await?;
                Ok(scan_response(run, false))
            }
            CreateRunOutcome::Existing(existing)
                if existing.status == RunStatusV1::Failed
                    && existing.error.as_ref().is_some_and(|error| error.retryable)
                    && existing.materialized.is_none() =>
            {
                let mut retried = (*existing).clone();
                retried.status = RunStatusV1::Queued;
                retried.operation_nonce = ids::operation_nonce();
                retried.attempt = retried.attempt.checked_add(1).ok_or_else(|| {
                    SecurityScanError::Dependency("security scan attempt overflow".into())
                })?;
                retried.step = 0;
                retried.step_failures = 0;
                retried.harness = None;
                retried.report = None;
                retried.error = None;
                retried.completed_at = None;
                retried.updated_at = now;
                if !self.runtime.replace_run(&existing, retried.clone()).await? {
                    let current =
                        self.runtime
                            .get_run(&retried.run_id)
                            .await?
                            .ok_or_else(|| {
                                SecurityScanError::Dependency(format!(
                                    "run {} disappeared during retry",
                                    retried.run_id
                                ))
                            })?;
                    return Ok(scan_response(current, true));
                }
                self.enqueue(&retried).await?;
                Ok(scan_response(retried, false))
            }
            CreateRunOutcome::Existing(existing) => Ok(scan_response(*existing, true)),
        }
    }

    async fn enqueue(&self, run: &RunRecordV1) -> Result<(), SecurityScanError> {
        self.runtime
            .enqueue_execute(EnqueueRequest::new(
                run.run_id.clone(),
                run.repository.clone(),
                run.attempt,
                run.step,
            ))
            .await
    }

    pub async fn cancel(
        &self,
        request: crate::SecurityScanCancelRequestV1,
    ) -> Result<crate::SecurityScanCancelResponseV1, SecurityScanError> {
        self.runtime.require_ready()?;
        if request.run_id.trim().is_empty() {
            return Err(SecurityScanError::InvalidRequest(
                "run_id cannot be empty".into(),
            ));
        }
        let Some(run) = self.runtime.get_run(&request.run_id).await? else {
            return Err(SecurityScanError::InvalidRequest(format!(
                "unknown run {}",
                request.run_id
            )));
        };
        if matches!(
            run.status,
            RunStatusV1::Completed | RunStatusV1::Failed | RunStatusV1::Cancelled
        ) {
            return Ok(crate::SecurityScanCancelResponseV1 {
                run_id: run.run_id,
                status: run.status,
                deduplicated: true,
            });
        }
        if run.status == RunStatusV1::Cancelling {
            if let Some(harness) = run.harness.as_ref() {
                self.runtime.stop_analysis(harness).await?;
            }
            return Ok(crate::SecurityScanCancelResponseV1 {
                run_id: run.run_id,
                status: run.status,
                deduplicated: true,
            });
        }
        let mut cancelling = run.clone();
        cancelling.status = RunStatusV1::Cancelling;
        cancelling.updated_at = ids::now_ms();
        if !self.runtime.replace_run(&run, cancelling.clone()).await? {
            let current = self
                .runtime
                .get_run(&request.run_id)
                .await?
                .ok_or_else(|| {
                    SecurityScanError::Dependency(format!(
                        "run {} disappeared during cancel",
                        request.run_id
                    ))
                })?;
            return Ok(crate::SecurityScanCancelResponseV1 {
                run_id: current.run_id,
                status: current.status,
                deduplicated: true,
            });
        }
        if let Some(harness) = cancelling.harness.as_ref() {
            self.runtime.stop_analysis(harness).await?;
        }
        self.enqueue(&cancelling).await?;
        Ok(crate::SecurityScanCancelResponseV1 {
            run_id: cancelling.run_id,
            status: cancelling.status,
            deduplicated: false,
        })
    }

    pub async fn read(
        &self,
        request: SecurityScanReadRequestV1,
    ) -> Result<SecurityScanReadResponseV1, SecurityScanError> {
        self.runtime.require_ready()?;
        if request.run_id.trim().is_empty() {
            return Err(SecurityScanError::InvalidRequest(
                "run_id cannot be empty".into(),
            ));
        }
        Ok(SecurityScanReadResponseV1 {
            run: self
                .runtime
                .get_run(&request.run_id)
                .await?
                .as_ref()
                .map(Into::into),
        })
    }

    pub async fn analysis_chat(
        &self,
        request: crate::SecurityScanAnalysisChatRequestV1,
    ) -> Result<crate::SecurityScanAnalysisChatResponseV1, SecurityScanError> {
        self.runtime.require_ready()?;
        if request.run_id.trim().is_empty() || request.run_id.trim() != request.run_id {
            return Err(SecurityScanError::InvalidRequest(
                "run_id must be non-empty and trimmed".into(),
            ));
        }
        let run = self
            .runtime
            .get_run(&request.run_id)
            .await?
            .ok_or_else(|| SecurityScanError::InvalidRequest("run_id was not found".into()))?;
        Ok(crate::SecurityScanAnalysisChatResponseV1 {
            available: self.runtime.ensure_analysis_chat_link(&run).await?,
        })
    }

    pub async fn list(
        &self,
        request: SecurityScanListRequestV1,
    ) -> Result<SecurityScanListResponseV1, SecurityScanError> {
        self.runtime.require_ready()?;
        let limit = request.limit.unwrap_or(DEFAULT_LIST_LIMIT);
        if !(1..=MAX_LIST_LIMIT).contains(&limit) {
            return Err(SecurityScanError::InvalidRequest(format!(
                "limit must be between 1 and {MAX_LIST_LIMIT}"
            )));
        }
        let repository = request
            .repository
            .as_deref()
            .map(str::trim)
            .filter(|repository| !repository.is_empty());
        if request.repository.is_some() && repository.is_none() {
            return Err(SecurityScanError::InvalidRequest(
                "repository cannot be empty when set".into(),
            ));
        }

        let mut runs = self.runtime.list_run_summaries().await?;
        runs.retain(|run| {
            repository.is_none_or(|repository| run.repository == repository)
                && request.status.is_none_or(|status| run.status == status)
        });
        runs.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.run_id.cmp(&right.run_id))
        });

        Ok(SecurityScanListResponseV1 {
            runs: runs.into_iter().take(limit as usize).collect(),
        })
    }

    pub async fn reconciliation(
        &self,
        request: SecurityScanReconciliationRequestV1,
    ) -> Result<SecurityScanReconciliationResponseV1, SecurityScanError> {
        self.runtime.require_ready()?;
        if request.run_id.trim().is_empty() || request.run_id.trim() != request.run_id {
            return Err(SecurityScanError::InvalidRequest(
                "run_id must be non-empty and trimmed".into(),
            ));
        }
        let limit = request.limit.unwrap_or(DEFAULT_RECONCILIATION_LIMIT);
        if !(1..=MAX_RECONCILIATION_LIMIT).contains(&limit) {
            return Err(SecurityScanError::InvalidRequest(format!(
                "limit must be between 1 and {MAX_RECONCILIATION_LIMIT}"
            )));
        }
        let offset = parse_cursor(request.cursor.as_deref())?;
        let run = self
            .runtime
            .get_run(&request.run_id)
            .await?
            .ok_or_else(|| SecurityScanError::InvalidRequest("run_id was not found".into()))?;

        let snapshot = if request.refresh {
            self.refresh_reconciliation(&run).await?
        } else if let Some(snapshot) = self
            .runtime
            .get_reconciliation_snapshot(&run.run_id)
            .await?
        {
            validate_snapshot_identity(&snapshot, &run)?;
            snapshot
        } else {
            self.uncollected_snapshot(&run)
        };

        let mut records = snapshot
            .records
            .iter()
            .filter(|record| request.source.is_none_or(|source| record.source == source))
            .filter(|record| {
                request
                    .severity
                    .is_none_or(|severity| record.severity == severity)
            })
            .filter(|record| {
                request
                    .lifecycle
                    .is_none_or(|lifecycle| record.lifecycle == lifecycle)
            })
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by_key(|record| (source_rank(record.source), record.number));
        if offset > records.len() {
            return Err(SecurityScanError::InvalidRequest(
                "cursor is beyond the filtered reconciliation result".into(),
            ));
        }
        let end = offset.saturating_add(limit as usize).min(records.len());
        let next_cursor = (end < records.len()).then(|| format!("v1:{end}"));
        let records = records[offset..end].to_vec();

        Ok(SecurityScanReconciliationResponseV1 {
            schema_version: snapshot.schema_version,
            run_id: snapshot.run_id,
            repository: snapshot.repository,
            target_sha: snapshot.target_sha,
            harness: snapshot.harness,
            github_repository: snapshot.github_repository,
            sources: snapshot.sources,
            matching: snapshot.matching,
            records,
            next_cursor,
        })
    }

    pub async fn action(
        &self,
        request: crate::SecurityScanActionRequestV1,
    ) -> Result<crate::SecurityScanActionResponseV1, SecurityScanError> {
        self.runtime.require_ready()?;
        if request.run_id.trim().is_empty() {
            return Err(SecurityScanError::InvalidRequest(
                "run_id cannot be empty".into(),
            ));
        }
        let run = self
            .runtime
            .get_run(&request.run_id)
            .await?
            .ok_or_else(|| {
                SecurityScanError::InvalidRequest(format!("unknown run {}", request.run_id))
            })?;
        crate::action_executor::validate_action_request(
            &run,
            request.finding_index,
            request.action,
        )?;
        let github_full_name = self
            .config
            .repository(&run.repository)
            .and_then(|repository| repository.github.as_ref())
            .map(|github| github.full_name.clone())
            .ok_or_else(|| {
                SecurityScanError::InvalidRequest(
                    "repository has no operator-verified GitHub mapping".into(),
                )
            })?;
        let now = ids::now_ms();
        let action = crate::SecurityActionRecordV1 {
            schema_version: "1".into(),
            action_id: ids::action_id(&run.run_id, request.finding_index, request.action),
            run_id: run.run_id.clone(),
            finding_index: request.finding_index,
            action: request.action,
            repository: run.repository.clone(),
            target_sha: run.target_sha.clone(),
            github_full_name,
            operation_nonce: ids::operation_nonce(),
            status: crate::SecurityActionStatusV1::Queued,
            attempt: 1,
            step: 0,
            step_failures: 0,
            materialized: None,
            harness: None,
            result: None,
            error: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
            cleanup_completed_at: None,
        };
        match self.runtime.create_action_if_absent(action.clone()).await? {
            crate::CreateActionOutcome::Created => {
                self.enqueue_action(&action).await?;
                Ok(action_response(&action, false))
            }
            crate::CreateActionOutcome::Existing(existing)
                if existing.status == crate::SecurityActionStatusV1::Failed
                    && existing.error.as_ref().is_some_and(|error| error.retryable)
                    && existing.result.is_none()
                    && (existing.materialized.is_none()
                        || existing.cleanup_completed_at.is_some()) =>
            {
                let mut retried = (*existing).clone();
                retried.status = crate::SecurityActionStatusV1::Queued;
                retried.operation_nonce = ids::operation_nonce();
                retried.attempt = retried.attempt.checked_add(1).ok_or_else(|| {
                    SecurityScanError::Dependency("security scan action attempt overflow".into())
                })?;
                retried.step = 0;
                retried.step_failures = 0;
                retried.harness = None;
                retried.materialized = None;
                retried.error = None;
                retried.completed_at = None;
                retried.cleanup_completed_at = None;
                retried.updated_at = now;
                if !self
                    .runtime
                    .replace_action(&existing, retried.clone())
                    .await?
                {
                    let current = self
                        .runtime
                        .get_action(&retried.action_id)
                        .await?
                        .ok_or_else(|| {
                            SecurityScanError::Dependency(format!(
                                "action {} disappeared during retry",
                                retried.action_id
                            ))
                        })?;
                    return Ok(action_response(&current, true));
                }
                self.enqueue_action(&retried).await?;
                Ok(action_response(&retried, false))
            }
            crate::CreateActionOutcome::Existing(existing) => Ok(action_response(&existing, true)),
        }
    }

    pub async fn action_read(
        &self,
        request: crate::SecurityScanActionReadRequestV1,
    ) -> Result<crate::SecurityScanActionReadResponseV1, SecurityScanError> {
        self.runtime.require_ready()?;
        if request.action_id.trim().is_empty() {
            return Err(SecurityScanError::InvalidRequest(
                "action_id cannot be empty".into(),
            ));
        }
        Ok(crate::SecurityScanActionReadResponseV1 {
            action: self
                .runtime
                .get_action(&request.action_id)
                .await?
                .as_ref()
                .map(Into::into),
        })
    }

    async fn enqueue_action(
        &self,
        action: &crate::SecurityActionRecordV1,
    ) -> Result<(), SecurityScanError> {
        self.runtime
            .enqueue_action_execute(crate::ActionEnqueueRequestV1::new(
                action.action_id.clone(),
                action.run_id.clone(),
                action.attempt,
                action.step,
            ))
            .await
    }

    async fn refresh_reconciliation(
        &self,
        run: &RunRecordV1,
    ) -> Result<ReconciliationSnapshotV1, SecurityScanError> {
        let Some(github_full_name) = self
            .config
            .repository(&run.repository)
            .and_then(|repository| repository.github.as_ref())
            .map(|github| github.full_name.clone())
        else {
            let snapshot = snapshot_with_status(run, ReconciliationSourceStatusV1::NotConfigured);
            self.runtime
                .save_reconciliation_snapshot(snapshot.clone())
                .await?;
            return Ok(snapshot);
        };

        let collected_at = ids::now_ms();
        let (dependabot, code_scanning) = tokio::join!(
            self.runtime.collect_reconciliation_source(
                ReconciliationSourceV1::Dependabot,
                &github_full_name,
                &run.target_sha,
                collected_at,
            ),
            self.runtime.collect_reconciliation_source(
                ReconciliationSourceV1::CodeScanning,
                &github_full_name,
                &run.target_sha,
                collected_at,
            ),
        );
        let mut collections = vec![
            dependabot.unwrap_or_else(|error| {
                tracing::warn!(run_id = %run.run_id, %error, "Dependabot reconciliation unavailable");
                unavailable_collection(ReconciliationSourceV1::Dependabot, collected_at)
            }),
            code_scanning.unwrap_or_else(|error| {
                tracing::warn!(run_id = %run.run_id, %error, "code-scanning reconciliation unavailable");
                unavailable_collection(ReconciliationSourceV1::CodeScanning, collected_at)
            }),
        ];

        let mut seen = HashSet::new();
        let mut records = Vec::new();
        for collection in &mut collections {
            collection.records.retain(|record| {
                record.source == collection.summary.source
                    && seen.insert((record.source, record.number))
            });
            if collection.summary.record_count.is_some() {
                collection.summary.record_count = Some(count_u32(collection.records.len()));
            }
            records.append(&mut collection.records);
        }
        records.sort_by_key(|record| (source_rank(record.source), record.number));
        // Harness rule_id is model-authored, not a typed external identifier.
        // V1 therefore never claims cross-source identity, even when strings match.
        let matching = unavailable_matching();
        let snapshot = ReconciliationSnapshotV1 {
            schema_version: "1".into(),
            run_id: run.run_id.clone(),
            repository: run.repository.clone(),
            target_sha: run.target_sha.clone(),
            harness: harness_summary(run),
            github_repository: Some(github_full_name),
            sources: collections
                .into_iter()
                .map(|collection| collection.summary)
                .collect(),
            matching,
            records,
        };
        self.runtime
            .save_reconciliation_snapshot(snapshot.clone())
            .await?;
        Ok(snapshot)
    }

    fn uncollected_snapshot(&self, run: &RunRecordV1) -> ReconciliationSnapshotV1 {
        let status = if self
            .config
            .repository(&run.repository)
            .and_then(|repository| repository.github.as_ref())
            .is_some()
        {
            ReconciliationSourceStatusV1::NotCollected
        } else {
            ReconciliationSourceStatusV1::NotConfigured
        };
        snapshot_with_status(run, status)
    }
}

fn scan_response(run: RunRecordV1, deduplicated: bool) -> SecurityScanResponseV1 {
    SecurityScanResponseV1 {
        run_id: run.run_id,
        status: run.status,
        deduplicated,
    }
}

fn parse_cursor(cursor: Option<&str>) -> Result<usize, SecurityScanError> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    let offset = cursor.strip_prefix("v1:").ok_or_else(|| {
        SecurityScanError::InvalidRequest("cursor must be an opaque reconciliation cursor".into())
    })?;
    if offset.is_empty() || !offset.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(SecurityScanError::InvalidRequest(
            "cursor must be an opaque reconciliation cursor".into(),
        ));
    }
    offset.parse::<usize>().map_err(|_| {
        SecurityScanError::InvalidRequest("cursor offset exceeds platform bounds".into())
    })
}

fn snapshot_with_status(
    run: &RunRecordV1,
    status: ReconciliationSourceStatusV1,
) -> ReconciliationSnapshotV1 {
    ReconciliationSnapshotV1 {
        schema_version: "1".into(),
        run_id: run.run_id.clone(),
        repository: run.repository.clone(),
        target_sha: run.target_sha.clone(),
        harness: harness_summary(run),
        github_repository: None,
        sources: [
            ReconciliationSourceV1::Dependabot,
            ReconciliationSourceV1::CodeScanning,
        ]
        .into_iter()
        .map(|source| ReconciliationSourceSummaryV1 {
            source,
            status,
            scope: source_scope(source),
            collected_at: None,
            record_count: None,
            health: unknown_health(),
        })
        .collect(),
        matching: unavailable_matching(),
        records: Vec::new(),
    }
}

fn unavailable_collection(
    source: ReconciliationSourceV1,
    collected_at: i64,
) -> ReconciliationSourceCollectionV1 {
    ReconciliationSourceCollectionV1 {
        summary: ReconciliationSourceSummaryV1 {
            source,
            status: ReconciliationSourceStatusV1::Unavailable,
            scope: source_scope(source),
            collected_at: Some(collected_at),
            record_count: None,
            health: unknown_health(),
        },
        records: Vec::new(),
    }
}

fn harness_summary(run: &RunRecordV1) -> HarnessReconciliationSummaryV1 {
    let verified_count = run
        .report
        .as_ref()
        .map(|report| count_u32(report.findings.len()));
    HarnessReconciliationSummaryV1 {
        status: if verified_count.is_some() {
            HarnessReconciliationStatusV1::Verified
        } else {
            HarnessReconciliationStatusV1::NotAvailable
        },
        verified_count,
        verified_at: verified_count.and(run.completed_at),
        scope: ReconciliationScopeV1::ExactCommit,
    }
}

fn unavailable_matching() -> ReconciliationMatchingV1 {
    ReconciliationMatchingV1 {
        status: ReconciliationMatchingStatusV1::Unavailable,
        matched_records: None,
    }
}

fn validate_snapshot_identity(
    snapshot: &ReconciliationSnapshotV1,
    run: &RunRecordV1,
) -> Result<(), SecurityScanError> {
    if snapshot.run_id != run.run_id
        || snapshot.repository != run.repository
        || snapshot.target_sha != run.target_sha
    {
        return Err(SecurityScanError::Dependency(format!(
            "reconciliation snapshot identity does not match run {}",
            run.run_id
        )));
    }
    Ok(())
}

fn source_scope(source: ReconciliationSourceV1) -> ReconciliationScopeV1 {
    match source {
        ReconciliationSourceV1::Dependabot => ReconciliationScopeV1::RepositoryDefaultBranch,
        ReconciliationSourceV1::CodeScanning => ReconciliationScopeV1::RepositorySnapshot,
    }
}

fn source_rank(source: ReconciliationSourceV1) -> u8 {
    match source {
        ReconciliationSourceV1::Dependabot => 0,
        ReconciliationSourceV1::CodeScanning => 1,
    }
}

fn unknown_health() -> ReconciliationSourceHealthV1 {
    ReconciliationSourceHealthV1 {
        status: ReconciliationHealthStatusV1::Unknown,
        tool: None,
        commit_sha: None,
        observed_at: None,
    }
}

fn count_u32(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn action_response(
    action: &crate::SecurityActionRecordV1,
    deduplicated: bool,
) -> crate::SecurityScanActionResponseV1 {
    crate::SecurityScanActionResponseV1 {
        action_id: action.action_id.clone(),
        run_id: action.run_id.clone(),
        finding_index: action.finding_index,
        action: action.action,
        status: action.status,
        deduplicated,
    }
}
