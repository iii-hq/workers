use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use async_trait::async_trait;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, TriggerAction};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};

mod wire;

use wire::*;

use crate::{
    AnalysisHandle, AnalysisPlan, CreateRunOutcome, EnqueueRequest, ExecutionRuntime,
    MaterializedTargetV1, PublicRunSummaryV1, ReconciliationAlertV1, ReconciliationHealthStatusV1,
    ReconciliationLifecycleV1, ReconciliationScopeV1, ReconciliationSnapshotV1,
    ReconciliationSourceCollectionV1, ReconciliationSourceHealthV1, ReconciliationSourceStatusV1,
    ReconciliationSourceSummaryV1, ReconciliationSourceV1, RepositoryConfigV1, RunRecordV1,
    RunStatusV1, SecurityRuntime, SecurityScanError, SeverityV1,
};

pub const RUN_SCOPE: &str = "security_scan_runs";
pub const RUN_INDEX_SCOPE: &str = "security_scan_run_index";
pub const RECONCILIATION_SCOPE: &str = "security_scan_reconciliation";
pub const RUN_QUEUE: &str = "security-scan-run";
const STATE_PREFIX: &str = "security-scan";
const STATE_GET_ID: &str = "security-scan::state::get";
const STATE_LIST_ID: &str = "security-scan::state::list";
const STATE_CAS_ID: &str = "security-scan::state::compare-and-set";
const CLAIM_NAMESPACE_ID: &str = "state::claim-namespace";
const EXECUTE_ID: &str = "security-scan::execute";
const GITHUB_API_ID: &str = "github::api";
const GITHUB_ALERT_LIMIT: usize = 500;
const RUN_STREAM_NAME: &str = "security-scan:runs";
const RUN_STREAM_GROUP: &str = "all";
const RUN_UPDATED_EVENT_TYPE: &str = "security-scan:updated";
const RECONCILIATION_UPDATED_EVENT_TYPE: &str = "security-scan:reconciliation-updated";
const RPC_TIMEOUT_MS: u64 = 30_000;
const EVENT_TIMEOUT_MS: u64 = 5_000;
const INDEX_REPAIR_ATTEMPTS: u32 = 8;
const BOOT_ATTEMPTS: u32 = 20;
const BOOT_RETRY_MS: u64 = 250;

#[derive(Clone)]
pub struct IiiRuntime {
    iii: Arc<IIIClient>,
    pending_index_repairs: Arc<Mutex<HashSet<String>>>,
    run_index_backfill_pending: Arc<AtomicBool>,
}

impl IiiRuntime {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self {
            iii,
            pending_index_repairs: Arc::new(Mutex::new(HashSet::new())),
            run_index_backfill_pending: Arc::new(AtomicBool::new(true)),
        }
    }

    pub async fn claim_private_state(&self) -> Result<(), SecurityScanError> {
        self.retry_boot_call(CLAIM_NAMESPACE_ID, || {
            self.call(
                CLAIM_NAMESPACE_ID,
                json!({
                    "functions_prefix": STATE_PREFIX,
                    "scopes": [RUN_SCOPE, RUN_INDEX_SCOPE, RECONCILIATION_SCOPE],
                }),
                None,
                Some(5_000),
            )
        })
        .await
        .map(|_| ())
    }

    pub async fn ensure_queue(&self) -> Result<(), SecurityScanError> {
        let definition = queue_definition();
        self.retry_boot_call("queue::define", || {
            self.call("queue::define", definition.clone(), None, Some(5_000))
        })
        .await
        .map(|_| ())
    }

    async fn list_full_runs(&self) -> Result<Vec<RunRecordV1>, SecurityScanError> {
        let value = self
            .call_private(STATE_LIST_ID, json!({ "scope": RUN_SCOPE }))
            .await?;
        parse_state_list(&value, "private run")
    }

    async fn list_index_records(&self) -> Result<Vec<RunIndexRecordV1>, SecurityScanError> {
        let value = self
            .call_private(STATE_LIST_ID, json!({ "scope": RUN_INDEX_SCOPE }))
            .await?;
        parse_state_list(&value, "private run index")
    }

    /// Migration scan retried until one complete list/parse succeeds.
    /// Steady-state listing and recovery never enumerate full records or
    /// deserialize their reports after that success.
    pub async fn backfill_run_index(&self) -> Result<usize, SecurityScanError> {
        let result = async {
            let runs = self.list_full_runs().await?;
            let mut repaired = 0;
            for run in runs {
                match self.repair_run_index_record(&run.run_id).await {
                    Ok(changed) => repaired += usize::from(changed),
                    Err(error) => {
                        self.queue_index_repair(&run.run_id);
                        tracing::warn!(
                            run_id = %run.run_id,
                            %error,
                            "security scan history backfill deferred"
                        );
                    }
                }
            }
            Ok(repaired)
        }
        .await;
        mark_backfill_complete(&self.run_index_backfill_pending, &result);
        result
    }

    /// Retries a failed boot-time migration without turning the full record
    /// scope into a steady-state polling source.
    pub async fn retry_run_index_backfill(&self) -> Result<Option<usize>, SecurityScanError> {
        if !self.run_index_backfill_pending.load(Ordering::Acquire) {
            return Ok(None);
        }
        self.backfill_run_index().await.map(Some)
    }

    pub async fn repair_pending_run_index(&self) -> usize {
        let pending = self
            .pending_index_repairs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let mut repaired = 0;
        for run_id in pending {
            match self.repair_run_index_record(&run_id).await {
                Ok(_) => {
                    self.clear_index_repair(&run_id);
                    repaired += 1;
                }
                Err(error) => {
                    tracing::warn!(
                        %run_id,
                        %error,
                        "security scan history projection repair failed"
                    );
                }
            }
        }
        repaired
    }

    pub async fn list_reconciliation_runs(&self) -> Result<Vec<RunRecordV1>, SecurityScanError> {
        let candidates = self
            .list_index_records()
            .await?
            .into_iter()
            .filter(needs_full_reconciliation);
        let mut runs = Vec::new();
        for candidate in candidates {
            if let Some(run) = self.get_run(&candidate.summary.run_id).await? {
                if run.status == RunStatusV1::Analyzing
                    || (is_terminal(run.status) && run.materialized.is_some())
                {
                    runs.push(run);
                }
            }
        }
        Ok(runs)
    }

    pub async fn recover_queueable_runs(&self) -> Result<usize, SecurityScanError> {
        let mut recovered = 0;
        let candidates = self
            .list_index_records()
            .await?
            .into_iter()
            .filter(|record| is_queueable(record.summary.status));
        for candidate in candidates {
            if let Some(run) = self.get_run(&candidate.summary.run_id).await? {
                if !is_queueable(run.status) {
                    continue;
                }
                self.enqueue_execute(EnqueueRequest::new(
                    run.run_id,
                    run.repository,
                    run.attempt,
                    run.step,
                ))
                .await?;
                recovered += 1;
            }
        }
        Ok(recovered)
    }

    async fn retry_boot_call<F, Fut>(
        &self,
        dependency: &str,
        mut call: F,
    ) -> Result<Value, SecurityScanError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<Value, SecurityScanError>>,
    {
        let mut last_error = None;
        for attempt in 1..=BOOT_ATTEMPTS {
            match call().await {
                Ok(value) => return Ok(value),
                Err(error) => {
                    last_error = Some(error);
                    if attempt < BOOT_ATTEMPTS {
                        tokio::time::sleep(Duration::from_millis(BOOT_RETRY_MS)).await;
                    }
                }
            }
        }
        Err(SecurityScanError::Dependency(format!(
            "{dependency} failed after {BOOT_ATTEMPTS} attempts: {}",
            last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "unknown error".into())
        )))
    }

    async fn call_private(
        &self,
        function_id: &str,
        payload: Value,
    ) -> Result<Value, SecurityScanError> {
        match self
            .call(function_id, payload.clone(), None, Some(RPC_TIMEOUT_MS))
            .await
        {
            Err(error) if accessor_is_missing(&error) => {
                self.claim_private_state().await?;
                self.call(function_id, payload, None, Some(RPC_TIMEOUT_MS))
                    .await
            }
            result => result,
        }
    }

    async fn call(
        &self,
        function_id: &str,
        payload: Value,
        action: Option<TriggerAction>,
        timeout_ms: Option<u64>,
    ) -> Result<Value, SecurityScanError> {
        self.iii
            .trigger(TriggerRequest {
                function_id: function_id.into(),
                payload,
                action,
                timeout_ms,
            })
            .await
            .map_err(|error| {
                SecurityScanError::Dependency(format!("{function_id} failed: {error}"))
            })
    }

    async fn call_typed<Req, Resp>(
        &self,
        function_id: &str,
        request: &Req,
    ) -> Result<Resp, SecurityScanError>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
    {
        let payload = serialize(request, "typed dependency request")?;
        let response = self
            .call(function_id, payload, None, Some(RPC_TIMEOUT_MS))
            .await?;
        serde_json::from_value(response).map_err(|error| dependency_parse(function_id, error))
    }

    async fn compare_and_set_in_scope(
        &self,
        scope: &str,
        key: &str,
        expected: Option<Value>,
        value: Value,
    ) -> Result<CasOutcome, SecurityScanError> {
        let mut payload = json!({
            "scope": scope,
            "key": key,
            "value": value,
        });
        if let Some(expected) = expected {
            payload["expected"] = expected;
        }
        let response = self.call_private(STATE_CAS_ID, payload).await?;
        let swapped = response
            .get("swapped")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                SecurityScanError::Dependency(format!(
                    "{STATE_CAS_ID} returned no boolean `swapped` field"
                ))
            })?;
        Ok(if swapped {
            CasOutcome::Swapped
        } else {
            CasOutcome::Current(response.get("current").cloned().unwrap_or(Value::Null))
        })
    }

    async fn compare_and_set(
        &self,
        key: &str,
        expected: Option<Value>,
        value: Value,
    ) -> Result<CasOutcome, SecurityScanError> {
        self.compare_and_set_in_scope(RUN_SCOPE, key, expected, value)
            .await
    }

    async fn repair_run_index_record(&self, run_id: &str) -> Result<bool, SecurityScanError> {
        let mut changed = false;
        for _ in 0..INDEX_REPAIR_ATTEMPTS {
            let source = self.get_run(run_id).await?;
            let desired = source.as_ref().map(RunIndexRecordV1::from);
            let current = self
                .call_private(
                    STATE_GET_ID,
                    json!({ "scope": RUN_INDEX_SCOPE, "key": run_id }),
                )
                .await?;
            let current_projection = if current.is_null() {
                None
            } else {
                serde_json::from_value::<RunIndexRecordV1>(current.clone()).ok()
            };

            if current_projection.as_ref() != desired.as_ref() {
                let expected = (!current.is_null()).then_some(current);
                let value = desired
                    .as_ref()
                    .map(|record| serialize(record, "run index record"))
                    .transpose()?
                    .unwrap_or(Value::Null);
                if !matches!(
                    self.compare_and_set_in_scope(RUN_INDEX_SCOPE, run_id, expected, value)
                        .await?,
                    CasOutcome::Swapped
                ) {
                    continue;
                }
                changed = true;
            }

            // A second authoritative read closes the race where another CAS
            // advances the run while this projection write is in flight.
            if self.get_run(run_id).await? == source {
                return Ok(changed);
            }
        }
        Err(SecurityScanError::Dependency(format!(
            "run {run_id} changed repeatedly while repairing its history projection"
        )))
    }

    async fn sync_run_index_best_effort(&self, run_id: &str) {
        match self.repair_run_index_record(run_id).await {
            Ok(_) => self.clear_index_repair(run_id),
            Err(error) => {
                self.queue_index_repair(run_id);
                tracing::warn!(
                    %run_id,
                    %error,
                    "security scan history projection update deferred"
                );
            }
        }
    }

    fn queue_index_repair(&self, run_id: &str) {
        self.pending_index_repairs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(run_id.to_owned());
    }

    fn clear_index_repair(&self, run_id: &str) {
        self.pending_index_repairs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(run_id);
    }

    fn emit_run_update(&self, run: &RunRecordV1) {
        let runtime = self.clone();
        let payload = run_update_payload(run);
        let run_id = run.run_id.clone();
        tokio::spawn(async move {
            if let Err(error) = runtime
                .call("stream::send", payload, None, Some(EVENT_TIMEOUT_MS))
                .await
            {
                tracing::warn!(
                    %run_id,
                    %error,
                    "security scan live-update doorbell failed"
                );
            }
        });
    }

    fn emit_reconciliation_update(&self, run_id: &str) {
        let runtime = self.clone();
        let payload = reconciliation_update_payload(run_id);
        let run_id = run_id.to_owned();
        tokio::spawn(async move {
            if let Err(error) = runtime
                .call("stream::send", payload, None, Some(EVENT_TIMEOUT_MS))
                .await
            {
                tracing::warn!(
                    %run_id,
                    %error,
                    "security scan reconciliation live-update doorbell failed"
                );
            }
        });
    }
}

#[async_trait]
impl SecurityRuntime for IiiRuntime {
    async fn get_run(&self, run_id: &str) -> Result<Option<RunRecordV1>, SecurityScanError> {
        let value = self
            .call_private(STATE_GET_ID, json!({ "scope": RUN_SCOPE, "key": run_id }))
            .await?;
        parse_optional_run(value, run_id)
    }

    async fn list_run_summaries(&self) -> Result<Vec<PublicRunSummaryV1>, SecurityScanError> {
        Ok(self
            .list_index_records()
            .await?
            .into_iter()
            .map(|record| record.summary)
            .collect())
    }

    async fn get_reconciliation_snapshot(
        &self,
        run_id: &str,
    ) -> Result<Option<ReconciliationSnapshotV1>, SecurityScanError> {
        let value = self
            .call_private(
                STATE_GET_ID,
                json!({ "scope": RECONCILIATION_SCOPE, "key": run_id }),
            )
            .await?;
        if value.is_null() {
            return Ok(None);
        }
        serde_json::from_value(value).map(Some).map_err(|error| {
            SecurityScanError::Dependency(format!(
                "could not parse reconciliation snapshot {run_id}: {error}"
            ))
        })
    }

    async fn save_reconciliation_snapshot(
        &self,
        snapshot: ReconciliationSnapshotV1,
    ) -> Result<(), SecurityScanError> {
        let replacement = serialize(&snapshot, "reconciliation snapshot")?;
        for _ in 0..INDEX_REPAIR_ATTEMPTS {
            let current = self
                .call_private(
                    STATE_GET_ID,
                    json!({ "scope": RECONCILIATION_SCOPE, "key": snapshot.run_id }),
                )
                .await?;
            if !current.is_null() {
                let current_snapshot: ReconciliationSnapshotV1 =
                    serde_json::from_value(current.clone()).map_err(|error| {
                        SecurityScanError::Dependency(format!(
                            "could not parse current reconciliation snapshot {}: {error}",
                            snapshot.run_id
                        ))
                    })?;
                if snapshot_is_newer(&current_snapshot, &snapshot) {
                    return Ok(());
                }
            }
            let expected = (!current.is_null()).then_some(current);
            if matches!(
                self.compare_and_set_in_scope(
                    RECONCILIATION_SCOPE,
                    &snapshot.run_id,
                    expected,
                    replacement.clone(),
                )
                .await?,
                CasOutcome::Swapped
            ) {
                self.emit_reconciliation_update(&snapshot.run_id);
                return Ok(());
            }
        }
        Err(SecurityScanError::Dependency(format!(
            "reconciliation snapshot {} changed repeatedly while saving",
            snapshot.run_id
        )))
    }

    async fn collect_reconciliation_source(
        &self,
        source: ReconciliationSourceV1,
        github_full_name: &str,
        target_sha: &str,
        collected_at: i64,
    ) -> Result<ReconciliationSourceCollectionV1, SecurityScanError> {
        match source {
            ReconciliationSourceV1::Dependabot => {
                let request = dependabot_api_request(github_full_name)?;
                let response = dependabot_api_response(
                    github_full_name,
                    self.call_typed::<_, GithubApiResponseWire>(GITHUB_API_ID, &request)
                        .await,
                );
                normalize_dependabot_response(github_full_name, collected_at, response)
            }
            ReconciliationSourceV1::CodeScanning => {
                let alerts_request = code_scanning_alerts_api_request(github_full_name)?;
                let analysis_request = code_scanning_analysis_api_request(github_full_name)?;
                let (alerts, analysis) = tokio::join!(
                    self.call_typed::<_, GithubApiResponseWire>(GITHUB_API_ID, &alerts_request),
                    self.call_typed::<_, GithubApiResponseWire>(GITHUB_API_ID, &analysis_request),
                );
                let response = code_scanning_api_response(github_full_name, alerts, analysis);
                normalize_code_scanning_response(
                    github_full_name,
                    target_sha,
                    collected_at,
                    response,
                )
            }
        }
    }

    async fn create_run_if_absent(
        &self,
        run: RunRecordV1,
    ) -> Result<CreateRunOutcome, SecurityScanError> {
        let value = serialize(&run, "run record")?;
        match self.compare_and_set(&run.run_id, None, value).await? {
            CasOutcome::Swapped => {
                self.sync_run_index_best_effort(&run.run_id).await;
                self.emit_run_update(&run);
                Ok(CreateRunOutcome::Created)
            }
            CasOutcome::Current(current) => {
                let existing = parse_run(current, &run.run_id)?;
                if existing.run_id != run.run_id
                    || existing.repository != run.repository
                    || existing.target_sha != run.target_sha
                    || existing.mode != run.mode
                    || existing.schema_version != run.schema_version
                {
                    return Err(SecurityScanError::Dependency(format!(
                        "state collision or corruption for run {}",
                        run.run_id
                    )));
                }
                self.sync_run_index_best_effort(&existing.run_id).await;
                Ok(CreateRunOutcome::Existing(Box::new(existing)))
            }
        }
    }

    async fn replace_run(
        &self,
        expected: &RunRecordV1,
        replacement: RunRecordV1,
    ) -> Result<bool, SecurityScanError> {
        if expected.run_id != replacement.run_id
            || expected.repository != replacement.repository
            || expected.target_sha != replacement.target_sha
            || expected.mode != replacement.mode
        {
            return Err(SecurityScanError::Dependency(
                "run replacement changed immutable identity fields".into(),
            ));
        }
        let expected_value = serialize(expected, "expected run record")?;
        let replacement_value = serialize(&replacement, "replacement run record")?;
        let swapped = matches!(
            self.compare_and_set(&expected.run_id, Some(expected_value), replacement_value,)
                .await?,
            CasOutcome::Swapped
        );
        if swapped {
            self.sync_run_index_best_effort(&replacement.run_id).await;
            self.emit_run_update(&replacement);
        }
        Ok(swapped)
    }

    async fn delete_run_if_unchanged(&self, run: &RunRecordV1) -> Result<(), SecurityScanError> {
        let expected = serialize(run, "run record")?;
        let deleted = matches!(
            self.compare_and_set(&run.run_id, Some(expected), Value::Null)
                .await?,
            CasOutcome::Swapped
        );
        if deleted {
            self.sync_run_index_best_effort(&run.run_id).await;
        }
        Ok(())
    }

    async fn enqueue_execute(&self, request: EnqueueRequest) -> Result<(), SecurityScanError> {
        self.call(
            EXECUTE_ID,
            serialize(&request, "queue request")?,
            Some(TriggerAction::Enqueue {
                queue: RUN_QUEUE.into(),
            }),
            None,
        )
        .await
        .map(|_| ())
    }
}

#[async_trait]
impl ExecutionRuntime for IiiRuntime {
    async fn get_run_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<RunRecordV1>, SecurityScanError> {
        let mut matches = self
            .list_index_records()
            .await?
            .into_iter()
            .filter(|record| record.harness_session_id.as_deref() == Some(session_id));
        let found = matches.next();
        if matches.next().is_some() {
            return Err(SecurityScanError::Dependency(format!(
                "multiple runs reference Harness session {session_id}"
            )));
        }
        let Some(found) = found else {
            return Ok(None);
        };
        let run = self.get_run(&found.summary.run_id).await?;
        Ok(run.filter(|run| {
            run.harness
                .as_ref()
                .is_some_and(|harness| harness.session_id == session_id)
        }))
    }

    async fn materialize_target(
        &self,
        repository: &RepositoryConfigV1,
        run: &RunRecordV1,
    ) -> Result<MaterializedTargetV1, SecurityScanError> {
        let session_id = materialization_session_id(run);
        let existing = self
            .call(
                "worktree::list",
                json!({
                    "repo_path": repository.path,
                    "session_id": session_id,
                    "include_status": false,
                }),
                None,
                Some(RPC_TIMEOUT_MS),
            )
            .await?;
        let mut worktrees = serde_json::from_value::<WorktreeListWire>(existing)
            .map_err(|error| dependency_parse("worktree::list", error))?
            .worktrees;
        if worktrees.len() > 1 {
            return Err(SecurityScanError::Dependency(format!(
                "worktree::list returned multiple checkouts for {session_id}"
            )));
        }
        if let Some(worktree) = worktrees.pop() {
            match worktree.lifecycle.as_str() {
                "orphaned" => {
                    let removed = self
                        .call(
                            "worktree::remove",
                            json!({
                                "worktree_id": worktree.worktree_id,
                                "force": false,
                                "delete_branch": true,
                            }),
                            None,
                            Some(RPC_TIMEOUT_MS),
                        )
                        .await?;
                    if removed.get("removed").and_then(Value::as_bool) != Some(true) {
                        return Err(SecurityScanError::Dependency(
                            "worktree::remove did not clear an orphaned scanner checkout".into(),
                        ));
                    }
                }
                "active" | "claimed" => {
                    return materialized_from_existing(worktree, repository, run)
                }
                lifecycle => {
                    return Err(SecurityScanError::Dependency(format!(
                        "scanner checkout {} has unexpected lifecycle {lifecycle}",
                        worktree.worktree_id
                    )))
                }
            }
        }

        let created = self
            .call(
                "worktree::create",
                json!({
                    "repo_path": repository.path,
                    "base_ref": run.target_sha,
                    "session_id": session_id,
                }),
                None,
                Some(RPC_TIMEOUT_MS),
            )
            .await?;
        let worktree: WorktreeCreateWire = serde_json::from_value(created)
            .map_err(|error| dependency_parse("worktree::create", error))?;
        materialized_from_created(worktree, run)
    }

    async fn cleanup_target(&self, target: &MaterializedTargetV1) -> Result<(), SecurityScanError> {
        let response = match self
            .call(
                "worktree::remove",
                json!({
                    "worktree_id": target.worktree_id,
                    "force": false,
                    "delete_branch": true,
                }),
                None,
                Some(RPC_TIMEOUT_MS),
            )
            .await
        {
            Ok(response) => response,
            Err(error) if worktree_is_missing(&error) => return Ok(()),
            Err(error) => return Err(error),
        };
        if response.get("removed").and_then(Value::as_bool) != Some(true) {
            return Err(SecurityScanError::Dependency(format!(
                "worktree::remove did not remove scanner checkout {}",
                target.worktree_id
            )));
        }
        if response.get("branch_deleted").and_then(Value::as_bool) != Some(true) {
            tracing::warn!(
                worktree_id = %target.worktree_id,
                "scanner checkout was removed but its branch was not deleted"
            );
        }
        Ok(())
    }

    async fn start_analysis(
        &self,
        plan: AnalysisPlan,
    ) -> Result<AnalysisHandle, SecurityScanError> {
        let existing = self
            .call(
                "harness::status",
                json!({ "session_id": plan.session_id }),
                None,
                Some(RPC_TIMEOUT_MS),
            )
            .await?;
        if !existing.is_null() {
            let status: HarnessStatusWire = serde_json::from_value(existing)
                .map_err(|error| dependency_parse("harness::status", error))?;
            if let Some(turn_id) = status.turn_id {
                return Ok(AnalysisHandle {
                    session_id: plan.session_id,
                    turn_id,
                });
            }
        }
        let request = harness_request(&plan);
        let response = self
            .call("harness::send", request, None, Some(RPC_TIMEOUT_MS))
            .await?;
        let response: HarnessSendWire = serde_json::from_value(response)
            .map_err(|error| dependency_parse("harness::send", error))?;
        if !response.accepted {
            return Err(SecurityScanError::Dependency(
                "harness::send did not accept the analysis turn".into(),
            ));
        }
        Ok(AnalysisHandle {
            session_id: response.session_id,
            turn_id: response.turn_id,
        })
    }

    async fn completed_analysis(
        &self,
        run: &RunRecordV1,
    ) -> Result<Option<crate::TurnCompletedEventV1>, SecurityScanError> {
        let harness = run.harness.as_ref().ok_or_else(|| {
            SecurityScanError::Dependency(format!(
                "analyzing run {} has no Harness checkpoint",
                run.run_id
            ))
        })?;
        let response = self
            .call(
                "harness::status",
                json!({ "session_id": harness.session_id }),
                None,
                Some(RPC_TIMEOUT_MS),
            )
            .await?;
        if response.is_null() {
            return Ok(None);
        }
        let status: HarnessStatusWire = serde_json::from_value(response)
            .map_err(|error| dependency_parse("harness::status", error))?;
        completion_event(status, harness)
    }
}

#[derive(Debug)]
enum CasOutcome {
    Swapped,
    Current(Value),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunIndexRecordV1 {
    schema_version: String,
    summary: PublicRunSummaryV1,
    has_materialized: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    harness_session_id: Option<String>,
}

impl From<&RunRecordV1> for RunIndexRecordV1 {
    fn from(run: &RunRecordV1) -> Self {
        Self {
            schema_version: "1".into(),
            summary: PublicRunSummaryV1::from(run),
            has_materialized: run.materialized.is_some(),
            harness_session_id: run
                .harness
                .as_ref()
                .map(|harness| harness.session_id.clone()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct WorktreeListWire {
    #[serde(default)]
    worktrees: Vec<WorktreeWire>,
}

#[derive(Debug, Deserialize)]
struct WorktreeWire {
    worktree_id: String,
    repo_path: String,
    path: String,
    base_sha: String,
    lifecycle: String,
}

#[derive(Debug, Deserialize)]
struct WorktreeCreateWire {
    worktree_id: String,
    path: String,
    base_sha: String,
}

#[derive(Debug, Deserialize)]
struct HarnessSendWire {
    session_id: String,
    turn_id: String,
    accepted: bool,
}

#[derive(Debug, Deserialize)]
struct HarnessStatusWire {
    #[serde(default)]
    turn_id: Option<String>,
    status: String,
    #[serde(default)]
    expects_wake: bool,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    result_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GithubCompletenessWire {
    Complete,
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GithubAvailabilityWire {
    Available,
    AuthenticationRequired,
    PermissionDenied,
    FeatureDisabled,
    RepositoryUnavailable,
    TemporarilyUnavailable,
    ClientUnavailable,
    MalformedResponse,
}

#[derive(Debug, Deserialize)]
struct DependabotAlertsResponseWire {
    repository: String,
    completeness: GithubCompletenessWire,
    availability: GithubAvailabilityWire,
    collected_count: usize,
    alerts: Vec<DependabotAlertWire>,
}

#[derive(Debug, Deserialize)]
struct DependabotAlertWire {
    number: u64,
    state: String,
    severity: String,
    package_name: String,
    ecosystem: String,
    manifest_path: String,
    ghsa_id: String,
    cve_id: Option<String>,
    advisory_summary: String,
    vulnerable_version_range: String,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct CodeScanningAlertsResponseWire {
    repository: String,
    completeness: GithubCompletenessWire,
    availability: GithubAvailabilityWire,
    collected_count: usize,
    alerts: Vec<CodeScanningAlertWire>,
    latest_analysis: LatestCodeScanningAnalysisWire,
}

#[derive(Debug, Deserialize)]
struct CodeScanningAlertWire {
    number: u64,
    state: String,
    rule_id: String,
    rule_name: Option<String>,
    rule_description: String,
    security_severity: Option<String>,
    severity: String,
    tool_name: String,
    commit_sha: Option<String>,
    path: Option<String>,
    start_line: Option<u64>,
    end_line: Option<u64>,
    created_at: String,
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LatestCodeScanningAnalysisWire {
    availability: GithubAvailabilityWire,
    tool_name: Option<String>,
    commit_sha: Option<String>,
    created_at: Option<String>,
    error: Option<String>,
    warning: Option<String>,
}

fn normalize_dependabot_response(
    github_full_name: &str,
    collected_at: i64,
    response: DependabotAlertsResponseWire,
) -> Result<ReconciliationSourceCollectionV1, SecurityScanError> {
    validate_github_response(
        github_full_name,
        &response.repository,
        response.collected_count,
        response.alerts.len(),
    )?;
    let status = source_status(response.completeness, response.availability);
    let available = response.availability == GithubAvailabilityWire::Available;
    let records = if available {
        response
            .alerts
            .into_iter()
            .map(|alert| normalize_dependabot_alert(github_full_name, alert))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    let record_count = available.then(|| count_u32(records.len()));
    let health = ReconciliationSourceHealthV1 {
        status: match status {
            ReconciliationSourceStatusV1::Complete => ReconciliationHealthStatusV1::Healthy,
            ReconciliationSourceStatusV1::Partial => ReconciliationHealthStatusV1::Warning,
            _ => ReconciliationHealthStatusV1::Unknown,
        },
        tool: None,
        commit_sha: None,
        observed_at: None,
    };
    Ok(ReconciliationSourceCollectionV1 {
        summary: ReconciliationSourceSummaryV1 {
            source: ReconciliationSourceV1::Dependabot,
            status,
            scope: ReconciliationScopeV1::RepositoryDefaultBranch,
            collected_at: Some(collected_at),
            record_count,
            health,
        },
        records,
    })
}

fn normalize_code_scanning_response(
    github_full_name: &str,
    target_sha: &str,
    collected_at: i64,
    response: CodeScanningAlertsResponseWire,
) -> Result<ReconciliationSourceCollectionV1, SecurityScanError> {
    validate_github_response(
        github_full_name,
        &response.repository,
        response.collected_count,
        response.alerts.len(),
    )?;
    let primary_available = response.availability == GithubAvailabilityWire::Available;
    let mut records = if primary_available {
        response
            .alerts
            .into_iter()
            .map(|alert| normalize_code_scanning_alert(github_full_name, target_sha, alert))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    let mut status = source_status(response.completeness, response.availability);
    let mut record_count = primary_available.then(|| count_u32(records.len()));
    let latest_available =
        response.latest_analysis.availability == GithubAvailabilityWire::Available;
    if primary_available && !latest_available {
        if records.is_empty() {
            status = unavailable_status(response.latest_analysis.availability);
            record_count = None;
        } else {
            status = ReconciliationSourceStatusV1::Partial;
        }
    }
    if !primary_available {
        records.clear();
    }
    let health = code_scanning_health(&response.latest_analysis);
    Ok(ReconciliationSourceCollectionV1 {
        summary: ReconciliationSourceSummaryV1 {
            source: ReconciliationSourceV1::CodeScanning,
            status,
            scope: ReconciliationScopeV1::RepositorySnapshot,
            collected_at: Some(collected_at),
            record_count,
            health,
        },
        records,
    })
}

fn normalize_dependabot_alert(
    github_full_name: &str,
    alert: DependabotAlertWire,
) -> Result<ReconciliationAlertV1, SecurityScanError> {
    validate_open_state(&alert.state)?;
    let package_name = sanitize_public_text(&alert.package_name, 256);
    let ecosystem = sanitize_public_text(&alert.ecosystem, 64);
    let vulnerable_range = sanitize_public_text(&alert.vulnerable_version_range, 512);
    let mut structured_ids = Vec::new();
    if let Some(identifier) = structured_identifier(&alert.ghsa_id) {
        structured_ids.push(identifier);
    }
    if let Some(identifier) = alert.cve_id.as_deref().and_then(structured_identifier) {
        if !structured_ids.contains(&identifier) {
            structured_ids.push(identifier);
        }
    }
    Ok(ReconciliationAlertV1 {
        source: ReconciliationSourceV1::Dependabot,
        number: alert.number,
        severity: normalize_severity(&alert.severity),
        lifecycle: ReconciliationLifecycleV1::Open,
        scope: ReconciliationScopeV1::RepositoryDefaultBranch,
        title: sanitize_public_text(&alert.advisory_summary, 512),
        description: format!(
            "Affected package {package_name} ({ecosystem}); vulnerable range {vulnerable_range}."
        ),
        public_url: github_alert_url(
            github_full_name,
            ReconciliationSourceV1::Dependabot,
            alert.number,
        )?,
        structured_ids,
        path: safe_repository_path(&alert.manifest_path),
        start_line: None,
        end_line: None,
        observed_at: nonempty_text(&alert.updated_at, 64),
    })
}

fn normalize_code_scanning_alert(
    github_full_name: &str,
    target_sha: &str,
    alert: CodeScanningAlertWire,
) -> Result<ReconciliationAlertV1, SecurityScanError> {
    validate_open_state(&alert.state)?;
    let scope = if alert
        .commit_sha
        .as_deref()
        .is_some_and(|sha| sha.eq_ignore_ascii_case(target_sha))
    {
        ReconciliationScopeV1::ExactCommit
    } else {
        ReconciliationScopeV1::RepositorySnapshot
    };
    let rule_id = structured_identifier(&alert.rule_id);
    let title = alert
        .rule_name
        .as_deref()
        .map(|value| sanitize_public_text(value, 256))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| sanitize_public_text(&alert.rule_description, 512));
    let mut description = sanitize_public_text(&alert.rule_description, 512);
    if description.is_empty() {
        description = "Code-scanning alert".into();
    }
    let observed_at = alert
        .updated_at
        .as_deref()
        .and_then(|value| nonempty_text(value, 64))
        .or_else(|| nonempty_text(&alert.created_at, 64));
    let severity = alert
        .security_severity
        .as_deref()
        .unwrap_or(&alert.severity);
    let _tool_name = sanitize_public_text(&alert.tool_name, 256);
    Ok(ReconciliationAlertV1 {
        source: ReconciliationSourceV1::CodeScanning,
        number: alert.number,
        severity: normalize_severity(severity),
        lifecycle: ReconciliationLifecycleV1::Open,
        scope,
        title,
        description,
        public_url: github_alert_url(
            github_full_name,
            ReconciliationSourceV1::CodeScanning,
            alert.number,
        )?,
        structured_ids: rule_id.into_iter().collect(),
        path: alert.path.as_deref().and_then(safe_repository_path),
        start_line: alert.start_line,
        end_line: alert.end_line,
        observed_at,
    })
}

fn source_status(
    completeness: GithubCompletenessWire,
    availability: GithubAvailabilityWire,
) -> ReconciliationSourceStatusV1 {
    if availability != GithubAvailabilityWire::Available {
        return unavailable_status(availability);
    }
    match completeness {
        GithubCompletenessWire::Complete => ReconciliationSourceStatusV1::Complete,
        GithubCompletenessWire::Partial => ReconciliationSourceStatusV1::Partial,
    }
}

fn unavailable_status(availability: GithubAvailabilityWire) -> ReconciliationSourceStatusV1 {
    match availability {
        GithubAvailabilityWire::Available => ReconciliationSourceStatusV1::Complete,
        GithubAvailabilityWire::AuthenticationRequired => {
            ReconciliationSourceStatusV1::AuthenticationRequired
        }
        GithubAvailabilityWire::PermissionDenied => ReconciliationSourceStatusV1::PermissionDenied,
        GithubAvailabilityWire::FeatureDisabled => ReconciliationSourceStatusV1::Disabled,
        GithubAvailabilityWire::RepositoryUnavailable
        | GithubAvailabilityWire::TemporarilyUnavailable
        | GithubAvailabilityWire::ClientUnavailable
        | GithubAvailabilityWire::MalformedResponse => ReconciliationSourceStatusV1::Unavailable,
    }
}

fn code_scanning_health(latest: &LatestCodeScanningAnalysisWire) -> ReconciliationSourceHealthV1 {
    let tool = latest
        .tool_name
        .as_deref()
        .and_then(|value| nonempty_text(value, 256));
    let commit_sha = latest.commit_sha.as_deref().and_then(validated_sha);
    let observed_at = latest
        .created_at
        .as_deref()
        .and_then(|value| nonempty_text(value, 64));
    let status = if latest.availability != GithubAvailabilityWire::Available {
        ReconciliationHealthStatusV1::Unknown
    } else if latest.error.is_some() {
        ReconciliationHealthStatusV1::Error
    } else if latest.warning.is_some() {
        ReconciliationHealthStatusV1::Warning
    } else if tool.is_some() || commit_sha.is_some() || observed_at.is_some() {
        ReconciliationHealthStatusV1::Healthy
    } else {
        ReconciliationHealthStatusV1::Unknown
    };
    ReconciliationSourceHealthV1 {
        status,
        tool,
        commit_sha,
        observed_at,
    }
}

fn validate_github_response(
    expected_repository: &str,
    actual_repository: &str,
    collected_count: usize,
    alert_count: usize,
) -> Result<(), SecurityScanError> {
    if !crate::config::is_valid_github_full_name(expected_repository)
        || actual_repository != expected_repository
    {
        return Err(SecurityScanError::Dependency(
            "GitHub security response repository did not match the configured mapping".into(),
        ));
    }
    if collected_count != alert_count {
        return Err(SecurityScanError::Dependency(
            "GitHub security response count did not match its alert records".into(),
        ));
    }
    Ok(())
}

fn validate_open_state(state: &str) -> Result<(), SecurityScanError> {
    if state.eq_ignore_ascii_case("open") {
        Ok(())
    } else {
        Err(SecurityScanError::Dependency(
            "GitHub security response contained a non-open alert".into(),
        ))
    }
}

fn github_alert_url(
    github_full_name: &str,
    source: ReconciliationSourceV1,
    number: u64,
) -> Result<String, SecurityScanError> {
    if !crate::config::is_valid_github_full_name(github_full_name) {
        return Err(SecurityScanError::Dependency(
            "configured GitHub repository is not a valid owner/name".into(),
        ));
    }
    let kind = match source {
        ReconciliationSourceV1::Dependabot => "dependabot",
        ReconciliationSourceV1::CodeScanning => "code-scanning",
    };
    Ok(format!(
        "https://github.com/{github_full_name}/security/{kind}/{number}"
    ))
}

fn normalize_severity(value: &str) -> SeverityV1 {
    match value.trim().to_ascii_lowercase().as_str() {
        "critical" => SeverityV1::Critical,
        "high" | "error" => SeverityV1::High,
        "medium" | "moderate" | "warning" => SeverityV1::Medium,
        "low" => SeverityV1::Low,
        _ => SeverityV1::Info,
    }
}

fn safe_repository_path(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.starts_with('/')
        || value.contains('\\')
        || value.split('/').any(|part| part.is_empty() || part == "..")
        || value.chars().any(char::is_control)
    {
        return None;
    }
    nonempty_text(value, 1_024)
}

fn structured_identifier(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 256
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
        })
    {
        return None;
    }
    Some(value.to_string())
}

fn validated_sha(value: &str) -> Option<String> {
    (value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

fn nonempty_text(value: &str, max_chars: usize) -> Option<String> {
    let value = sanitize_public_text(value, max_chars);
    (!value.is_empty()).then_some(value)
}

fn sanitize_public_text(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    let mut pending_space = false;
    for character in value.chars() {
        if output.chars().count() == max_chars {
            break;
        }
        if character.is_control() || character.is_whitespace() {
            pending_space = !output.is_empty();
            continue;
        }
        if pending_space {
            output.push(' ');
            pending_space = false;
        }
        output.push(character);
    }
    output.trim().to_string()
}

fn count_u32(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn materialization_session_id(run: &RunRecordV1) -> String {
    format!(
        "security-scan-worktree-{}-attempt-{}",
        run.operation_nonce, run.attempt
    )
}

fn materialized_from_existing(
    worktree: WorktreeWire,
    repository: &RepositoryConfigV1,
    run: &RunRecordV1,
) -> Result<MaterializedTargetV1, SecurityScanError> {
    if worktree.repo_path != repository.path {
        return Err(SecurityScanError::Dependency(format!(
            "recovered worktree {} belongs to an unexpected repository",
            worktree.worktree_id
        )));
    }
    materialized(worktree.worktree_id, worktree.path, worktree.base_sha, run)
}

fn materialized_from_created(
    worktree: WorktreeCreateWire,
    run: &RunRecordV1,
) -> Result<MaterializedTargetV1, SecurityScanError> {
    materialized(worktree.worktree_id, worktree.path, worktree.base_sha, run)
}

fn materialized(
    worktree_id: String,
    path: String,
    base_sha: String,
    run: &RunRecordV1,
) -> Result<MaterializedTargetV1, SecurityScanError> {
    if !base_sha.eq_ignore_ascii_case(&run.target_sha) {
        return Err(SecurityScanError::Dependency(format!(
            "worktree resolved {} instead of requested {}",
            base_sha, run.target_sha
        )));
    }
    Ok(MaterializedTargetV1 {
        worktree_id,
        path,
        base_sha,
    })
}

fn harness_request(plan: &AnalysisPlan) -> Value {
    json!({
        "session_id": plan.session_id,
        "message": plan.message,
        "model": plan.model,
        "provider": plan.provider,
        "idempotency_key": plan.idempotency_key,
        "session": {
            "title": "Security review",
            "metadata": { "security_scan": true },
        },
        "options": {
            "system_prompt": plan.system_prompt,
            "system_prompt_strategy": "override",
            "mode": "agent",
            "max_turns": plan.max_turns,
            "max_output_tokens": plan.max_output_tokens,
            "max_total_tokens": plan.max_total_tokens,
            "max_cost_usd": plan.max_cost_usd,
            "output": {
                "type": "json",
                "schema": plan.output_schema,
            },
            "functions": {
                "allow": plan.allowed_functions,
                "deny": [
                    "shell::*",
                    "state::*",
                    "queue::*",
                    "worktree::*",
                    "harness::*",
                    "github::*",
                    "approval::*",
                    "configuration::*",
                    "storage::*",
                    "database::*",
                    "security-scan::*",
                ],
                "expose": "agent_trigger",
            },
            "metadata": {
                "fs_scope": { "root": plan.filesystem_root },
            },
        },
    })
}

fn completion_event(
    status: HarnessStatusWire,
    harness: &crate::HarnessRunV1,
) -> Result<Option<crate::TurnCompletedEventV1>, SecurityScanError> {
    if status.turn_id.as_deref() != Some(harness.turn_id.as_str()) {
        return Ok(None);
    }
    if status.expects_wake || matches!(status.status.as_str(), "running" | "awaiting_functions") {
        return Ok(None);
    }
    if !matches!(status.status.as_str(), "completed" | "cancelled" | "failed") {
        return Err(SecurityScanError::Dependency(format!(
            "harness::status returned unknown status {}",
            status.status
        )));
    }
    Ok(Some(crate::TurnCompletedEventV1 {
        session_id: harness.session_id.clone(),
        turn_id: harness.turn_id.clone(),
        status: status.status,
        terminal: true,
        result: status.result,
        result_error: status.result_error,
        reason: None,
    }))
}

fn queue_definition() -> Value {
    json!({
        "queue": RUN_QUEUE,
        "config": {
            "type": "fifo",
            "message_group_field": "repository",
            "concurrency": 4,
            "max_retries": 3,
            "backoff_ms": 1_000,
            "poll_interval_ms": 100,
            "redeliver_on_engine_restart": true,
        },
    })
}

fn run_update_payload(run: &RunRecordV1) -> Value {
    json!({
        "stream_name": RUN_STREAM_NAME,
        "group_id": RUN_STREAM_GROUP,
        "type": RUN_UPDATED_EVENT_TYPE,
        "data": {
            "run_id": run.run_id,
            "repository": run.repository,
            "status": run.status,
            "attempt": run.attempt,
            "updated_at": run.updated_at,
            "completed_at": run.completed_at,
        },
    })
}

fn reconciliation_update_payload(run_id: &str) -> Value {
    json!({
        "stream_name": RUN_STREAM_NAME,
        "group_id": RUN_STREAM_GROUP,
        "type": RECONCILIATION_UPDATED_EVENT_TYPE,
        "data": { "run_id": run_id },
    })
}

fn snapshot_is_newer(
    existing: &ReconciliationSnapshotV1,
    candidate: &ReconciliationSnapshotV1,
) -> bool {
    let latest = |snapshot: &ReconciliationSnapshotV1| {
        snapshot
            .sources
            .iter()
            .filter_map(|source| source.collected_at)
            .max()
    };
    match (latest(existing), latest(candidate)) {
        (Some(existing), Some(candidate)) => existing > candidate,
        (Some(_), None) => true,
        _ => false,
    }
}

fn serialize<T: serde::Serialize>(value: &T, label: &str) -> Result<Value, SecurityScanError> {
    serde_json::to_value(value).map_err(|error| {
        SecurityScanError::Dependency(format!("could not serialize {label}: {error}"))
    })
}

fn mark_backfill_complete<T>(pending: &AtomicBool, result: &Result<T, SecurityScanError>) {
    if result.is_ok() {
        pending.store(false, Ordering::Release);
    }
}

fn parse_optional_run(
    value: Value,
    run_id: &str,
) -> Result<Option<RunRecordV1>, SecurityScanError> {
    if value.is_null() {
        return Ok(None);
    }
    parse_run(value, run_id).map(Some)
}

fn parse_run(value: Value, run_id: &str) -> Result<RunRecordV1, SecurityScanError> {
    serde_json::from_value(value).map_err(|error| {
        SecurityScanError::Dependency(format!(
            "could not parse private state record {run_id}: {error}"
        ))
    })
}

fn parse_state_list<T>(value: &Value, label: &str) -> Result<Vec<T>, SecurityScanError>
where
    T: DeserializeOwned,
{
    let candidates: Vec<&Value> = match value {
        Value::Array(values) => values.iter().collect(),
        Value::Object(map) => {
            if let Some(Value::Array(values)) = map.get("values").or_else(|| map.get("items")) {
                values.iter().collect()
            } else {
                map.values().collect()
            }
        }
        Value::Null => Vec::new(),
        _ => {
            return Err(SecurityScanError::Dependency(
                "private state list returned an unsupported shape".into(),
            ))
        }
    };
    let mut records = Vec::new();
    for value in candidates {
        if value.is_null() {
            continue;
        }
        records.push(serde_json::from_value(value.clone()).map_err(|error| {
            SecurityScanError::Dependency(format!(
                "could not parse {label} state list record: {error}"
            ))
        })?);
    }
    Ok(records)
}

fn is_queueable(status: RunStatusV1) -> bool {
    matches!(
        status,
        RunStatusV1::Queued
            | RunStatusV1::Materializing
            | RunStatusV1::Materialized
            | RunStatusV1::Dispatching
    )
}

fn is_terminal(status: RunStatusV1) -> bool {
    matches!(
        status,
        RunStatusV1::Completed | RunStatusV1::Failed | RunStatusV1::Cancelled
    )
}

fn needs_full_reconciliation(record: &RunIndexRecordV1) -> bool {
    record.summary.status == RunStatusV1::Analyzing
        || (is_terminal(record.summary.status) && record.has_materialized)
}

fn dependency_parse(dependency: &str, error: serde_json::Error) -> SecurityScanError {
    SecurityScanError::Dependency(format!("could not parse {dependency} response: {error}"))
}

fn accessor_is_missing(error: &SecurityScanError) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("function_not_found") || message.contains("not found")
}

fn worktree_is_missing(error: &SecurityScanError) -> bool {
    error.to_string().contains("W200")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AnalysisConfigV1, HarnessRunV1, ScanModeV1, SecurityFindingV1, SecurityReportV1, SeverityV1,
    };

    fn private_run(status: RunStatusV1) -> RunRecordV1 {
        RunRecordV1 {
            schema_version: "1".into(),
            run_id: "sec_history".into(),
            repository: "iii-hq/iii".into(),
            target_sha: "a".repeat(40),
            mode: ScanModeV1::Scan,
            operation_nonce: "private_nonce".into(),
            status,
            attempt: 1,
            step: 2,
            step_failures: 0,
            materialized: Some(MaterializedTargetV1 {
                worktree_id: "wt_private".into(),
                path: "/private/checkout".into(),
                base_sha: "a".repeat(40),
            }),
            harness: Some(HarnessRunV1 {
                session_id: "session_private".into(),
                turn_id: "turn_private".into(),
            }),
            report: None,
            error: None,
            created_at: 1,
            updated_at: 2,
            completed_at: None,
        }
    }

    #[test]
    fn run_queue_uses_the_existing_durable_fifo_worker() {
        let definition = queue_definition();
        assert_eq!(definition["queue"], RUN_QUEUE);
        assert_eq!(definition["config"]["type"], "fifo");
        assert_eq!(definition["config"]["message_group_field"], "repository");
        assert_eq!(definition["config"]["redeliver_on_engine_restart"], true);
    }

    #[test]
    fn harness_request_is_read_only_and_scoped_to_the_materialized_checkout() {
        let run = RunRecordV1 {
            schema_version: "1".into(),
            run_id: "sec_123".into(),
            repository: "repo".into(),
            target_sha: "a".repeat(40),
            mode: ScanModeV1::Scan,
            operation_nonce: "private_nonce".into(),
            status: RunStatusV1::Materialized,
            attempt: 1,
            step: 1,
            step_failures: 0,
            materialized: None,
            harness: None,
            report: None,
            error: None,
            created_at: 1,
            updated_at: 1,
            completed_at: None,
        };
        let plan = crate::build_analysis_plan(
            &run,
            "/isolated/repo",
            &AnalysisConfigV1 {
                model: "model".into(),
                provider: None,
                max_turns: 4,
                max_output_tokens: 8_000,
                max_total_tokens: 50_000,
                max_cost_usd: Some(2.0),
            },
        );
        let request = harness_request(&plan);
        assert_eq!(
            request["options"]["metadata"]["fs_scope"]["root"],
            "/isolated/repo"
        );
        assert_eq!(request["options"]["mode"], "agent");
        assert_eq!(request["options"]["output"]["type"], "json");
        let allow = request["options"]["functions"]["allow"]
            .as_array()
            .expect("allow array");
        assert!(allow
            .iter()
            .all(|value| !value.as_str().unwrap_or_default().contains("shell")));
        assert!(allow
            .iter()
            .all(|value| !value.as_str().unwrap_or_default().contains("create-file")));
        assert_eq!(request["options"]["system_prompt_strategy"], "override");
    }

    #[test]
    fn private_state_list_parser_accepts_supported_worker_shapes() {
        let record = json!({
            "schema_version": "1",
            "run_id": "sec_x",
            "repository": "repo",
            "target_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "mode": "scan",
            "operation_nonce": "private_nonce",
            "status": "queued",
            "attempt": 1,
            "step": 0,
            "created_at": 1,
            "updated_at": 1
        });
        assert_eq!(
            parse_state_list::<RunRecordV1>(&json!([record.clone()]), "run")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            parse_state_list::<RunRecordV1>(&json!({ "values": [record.clone()] }), "run")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            parse_state_list::<RunRecordV1>(&json!({ "sec_x": record }), "run")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn top_level_history_list_and_parse_failures_keep_backfill_retry_pending() {
        let pending = AtomicBool::new(true);
        let list_failure: Result<(), SecurityScanError> = Err(SecurityScanError::Dependency(
            "private state list temporarily unavailable".into(),
        ));
        mark_backfill_complete(&pending, &list_failure);
        assert!(pending.load(Ordering::Acquire));

        let parse_failure =
            parse_state_list::<RunRecordV1>(&json!({ "values": [{ "invalid": true }] }), "run");
        mark_backfill_complete(&pending, &parse_failure);
        assert!(pending.load(Ordering::Acquire));

        let successful_parse = parse_state_list::<RunRecordV1>(&Value::Null, "run");
        mark_backfill_complete(&pending, &successful_parse);
        assert!(!pending.load(Ordering::Acquire));
    }

    #[test]
    fn run_index_backfills_previous_results_without_copying_full_reports() {
        let mut run = private_run(RunStatusV1::Completed);
        run.completed_at = Some(2);
        run.report = Some(SecurityReportV1 {
            summary: "One actionable finding".into(),
            assessments: crate::SecurityAssessmentsV1::default(),
            findings: vec![SecurityFindingV1 {
                rule_id: "SEC-001".into(),
                severity: SeverityV1::High,
                title: "Unsafe default".into(),
                description: "Details".into(),
                evidence: "Evidence".into(),
                location: None,
                remediation: "Fix it".into(),
                suggested_patch: Some("large patch contents".into()),
            }],
        });

        let index = RunIndexRecordV1::from(&run);
        let encoded = serde_json::to_value(&index).unwrap();

        assert_eq!(index.summary.finding_count, 1);
        assert_eq!(index.summary.status, RunStatusV1::Completed);
        assert_eq!(index.harness_session_id.as_deref(), Some("session_private"));
        assert!(index.has_materialized);
        let encoded = encoded.to_string();
        for private in [
            "private_nonce",
            "wt_private",
            "/private/checkout",
            "turn_private",
            "large patch contents",
            "One actionable finding",
        ] {
            assert!(!encoded.contains(private), "history index copied {private}");
        }
    }

    #[test]
    fn run_index_projection_tracks_authoritative_lifecycle_updates() {
        let queued = private_run(RunStatusV1::Queued);
        let queued_index = RunIndexRecordV1::from(&queued);
        assert_eq!(queued_index.summary.status, RunStatusV1::Queued);

        let mut completed = queued;
        completed.status = RunStatusV1::Completed;
        completed.materialized = None;
        completed.harness = None;
        completed.updated_at = 3;
        completed.completed_at = Some(3);
        completed.report = Some(SecurityReportV1 {
            summary: "No findings returned".into(),
            assessments: crate::SecurityAssessmentsV1::default(),
            findings: Vec::new(),
        });
        let completed_index = RunIndexRecordV1::from(&completed);

        assert_eq!(completed_index.summary.status, RunStatusV1::Completed);
        assert_eq!(completed_index.summary.finding_count, 0);
        assert_eq!(completed_index.summary.updated_at, 3);
        assert!(!completed_index.has_materialized);
        assert!(completed_index.harness_session_id.is_none());
        assert_ne!(queued_index, completed_index);
    }

    #[test]
    fn recovery_index_selects_only_active_or_dirty_terminal_runs() {
        let analyzing = RunIndexRecordV1::from(&private_run(RunStatusV1::Analyzing));
        let dirty_terminal = RunIndexRecordV1::from(&private_run(RunStatusV1::Failed));
        let mut clean_terminal = private_run(RunStatusV1::Completed);
        clean_terminal.materialized = None;
        let clean_terminal = RunIndexRecordV1::from(&clean_terminal);
        let queued = RunIndexRecordV1::from(&private_run(RunStatusV1::Queued));

        assert!(needs_full_reconciliation(&analyzing));
        assert!(needs_full_reconciliation(&dirty_terminal));
        assert!(!needs_full_reconciliation(&clean_terminal));
        assert!(!needs_full_reconciliation(&queued));
        assert!(is_queueable(queued.summary.status));
        assert!(!is_queueable(clean_terminal.summary.status));
    }

    #[test]
    fn run_index_parser_accepts_durable_state_list_shapes() {
        let index =
            serde_json::to_value(RunIndexRecordV1::from(&private_run(RunStatusV1::Analyzing)))
                .unwrap();
        assert_eq!(
            parse_state_list::<RunIndexRecordV1>(&json!([index.clone()]), "run index")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            parse_state_list::<RunIndexRecordV1>(&json!({ "sec_history": index }), "run index")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn harness_status_reconciliation_ignores_running_and_recovers_terminal_results() {
        let harness = crate::HarnessRunV1 {
            session_id: "s1".into(),
            turn_id: "t1".into(),
        };
        assert!(completion_event(
            HarnessStatusWire {
                turn_id: Some("t1".into()),
                status: "running".into(),
                expects_wake: false,
                result: None,
                result_error: None,
            },
            &harness,
        )
        .unwrap()
        .is_none());

        let completed = completion_event(
            HarnessStatusWire {
                turn_id: Some("t1".into()),
                status: "completed".into(),
                expects_wake: false,
                result: Some(json!({ "summary": "ok", "findings": [] })),
                result_error: None,
            },
            &harness,
        )
        .unwrap()
        .expect("terminal event");
        assert!(completed.terminal);
        assert_eq!(completed.status, "completed");
    }

    #[test]
    fn missing_worktree_record_is_an_idempotent_cleanup_success() {
        assert!(worktree_is_missing(&SecurityScanError::Dependency(
            "worktree::remove failed: W200 no record".into()
        )));
        assert!(!worktree_is_missing(&SecurityScanError::Dependency(
            "worktree::remove failed: W300 state unavailable".into()
        )));
    }

    #[test]
    fn materialization_identity_is_attempt_scoped() {
        let mut run = RunRecordV1 {
            schema_version: "1".into(),
            run_id: "sec_retry".into(),
            repository: "repo".into(),
            target_sha: "a".repeat(40),
            mode: ScanModeV1::Scan,
            operation_nonce: "private_nonce".into(),
            status: RunStatusV1::Queued,
            attempt: 2,
            step: 0,
            step_failures: 0,
            materialized: None,
            harness: None,
            report: None,
            error: None,
            created_at: 1,
            updated_at: 1,
            completed_at: None,
        };
        assert_eq!(
            materialization_session_id(&run),
            "security-scan-worktree-private_nonce-attempt-2"
        );
        run.attempt = 3;
        assert_ne!(
            materialization_session_id(&run),
            "security-scan-worktree-private_nonce-attempt-2"
        );
    }

    #[test]
    fn run_update_doorbell_contains_only_the_public_status_projection() {
        let run = RunRecordV1 {
            schema_version: "1".into(),
            run_id: "sec_live".into(),
            repository: "iii-hq/iii".into(),
            target_sha: "a".repeat(40),
            mode: ScanModeV1::Suggest,
            operation_nonce: "private_nonce".into(),
            status: RunStatusV1::Analyzing,
            attempt: 2,
            step: 2,
            step_failures: 0,
            materialized: Some(MaterializedTargetV1 {
                worktree_id: "wt_private".into(),
                path: "/private/checkout".into(),
                base_sha: "a".repeat(40),
            }),
            harness: Some(crate::HarnessRunV1 {
                session_id: "session_private".into(),
                turn_id: "turn_private".into(),
            }),
            report: None,
            error: None,
            created_at: 1,
            updated_at: 2,
            completed_at: None,
        };

        assert_eq!(
            run_update_payload(&run),
            json!({
                "stream_name": "security-scan:runs",
                "group_id": "all",
                "type": "security-scan:updated",
                "data": {
                    "run_id": "sec_live",
                    "repository": "iii-hq/iii",
                    "status": "analyzing",
                    "attempt": 2,
                    "updated_at": 2,
                    "completed_at": null,
                },
            })
        );
    }

    #[test]
    fn code_alert_for_another_commit_remains_a_repository_snapshot() {
        let target_sha = "a".repeat(40);
        let alert = CodeScanningAlertWire {
            number: 7,
            state: "open".into(),
            rule_id: "rust/sql-injection".into(),
            rule_name: Some("SQL injection".into()),
            rule_description: "Untrusted input reaches a query".into(),
            security_severity: Some("high".into()),
            severity: "error".into(),
            tool_name: "CodeQL".into(),
            commit_sha: Some("b".repeat(40)),
            path: Some("src/main.rs".into()),
            start_line: Some(10),
            end_line: Some(12),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: None,
        };

        let normalized = normalize_code_scanning_alert("iii-hq/iii", &target_sha, alert).unwrap();

        assert_eq!(normalized.scope, ReconciliationScopeV1::RepositorySnapshot);
        assert_eq!(
            normalized.public_url,
            "https://github.com/iii-hq/iii/security/code-scanning/7"
        );
    }

    #[test]
    fn reconciliation_snapshot_and_doorbell_exclude_dependency_diagnostics() {
        let target_sha = "a".repeat(40);
        let response: CodeScanningAlertsResponseWire = serde_json::from_value(json!({
            "repository": "iii-hq/iii",
            "completeness": "complete",
            "availability": "available",
            "collected_count": 1,
            "truncation_reason": null,
            "alerts": [{
                "number": 9,
                "state": "open",
                "rule_id": "rust/sql-injection",
                "rule_name": "SQL injection",
                "rule_description": "Untrusted input reaches a query",
                "security_severity": "high",
                "severity": "error",
                "tool_name": "CodeQL",
                "html_url": "https://internal.invalid/token-secret",
                "commit_sha": target_sha,
                "message": "raw diagnostic token-secret",
                "path": "src/main.rs",
                "start_line": 10,
                "end_line": 12,
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": null
            }],
            "latest_analysis": {
                "availability": "available",
                "tool_name": "Trivy",
                "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "git_ref": "refs/heads/main",
                "created_at": "2026-01-02T00:00:00Z",
                "error": "configuration failed token-secret",
                "warning": null
            }
        }))
        .unwrap();
        let collection =
            normalize_code_scanning_response("iii-hq/iii", &"a".repeat(40), 100, response).unwrap();
        assert_eq!(
            collection.summary.health.status,
            ReconciliationHealthStatusV1::Error
        );
        let snapshot = ReconciliationSnapshotV1 {
            schema_version: "1".into(),
            run_id: "sec_live".into(),
            repository: "iii".into(),
            target_sha: "a".repeat(40),
            harness: crate::HarnessReconciliationSummaryV1 {
                status: crate::HarnessReconciliationStatusV1::Verified,
                verified_count: Some(3),
                verified_at: Some(90),
                scope: ReconciliationScopeV1::ExactCommit,
            },
            github_repository: Some("iii-hq/iii".into()),
            sources: vec![collection.summary],
            matching: crate::ReconciliationMatchingV1 {
                status: crate::ReconciliationMatchingStatusV1::Unavailable,
                matched_records: None,
            },
            records: collection.records,
        };
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert!(!encoded.contains("internal.invalid"));
        assert!(!encoded.contains("raw diagnostic"));
        assert!(!encoded.contains("configuration failed"));
        assert!(!encoded.contains("token-secret"));

        let mut older = snapshot.clone();
        older.sources[0].collected_at = Some(99);
        assert!(snapshot_is_newer(&snapshot, &older));
        let mut newer = snapshot.clone();
        newer.sources[0].collected_at = Some(101);
        assert!(!snapshot_is_newer(&snapshot, &newer));

        let payload = reconciliation_update_payload("sec_live");
        assert_eq!(
            payload,
            json!({
                "stream_name": "security-scan:runs",
                "group_id": "all",
                "type": "security-scan:reconciliation-updated",
                "data": { "run_id": "sec_live" },
            })
        );
        assert!(serde_json::to_string(&payload).unwrap().len() < 256);
    }
}
