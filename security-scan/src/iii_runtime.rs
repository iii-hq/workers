use std::{
    collections::{BTreeMap, HashSet},
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

use crate::{
    archive, AnalysisHandle, AnalysisPlan, ArchiveConfigV1, CreateRunOutcome, EnqueueRequest,
    ExecutionRuntime, MaterializationRequest, MaterializedTargetV1, PublicRunSummaryV1,
    ReconciliationAlertV1, ReconciliationHealthStatusV1, ReconciliationLifecycleV1,
    ReconciliationScopeV1, ReconciliationSnapshotV1, ReconciliationSourceCollectionV1,
    ReconciliationSourceHealthV1, ReconciliationSourceStatusV1, ReconciliationSourceSummaryV1,
    ReconciliationSourceV1, RepositoryConfigV1, RunRecordV1, RunStatusV1, SecurityRuntime,
    SecurityScanError, SeverityV1,
};

mod archive_gateway;
mod execution_runtime;
mod git_gateway;
mod security_runtime;

pub const RUN_SCOPE: &str = "security_scan_runs";
pub const RUN_INDEX_SCOPE: &str = "security_scan_run_index";
pub const RECONCILIATION_SCOPE: &str = "security_scan_reconciliation";
pub const ACTION_SCOPE: &str = "security_scan_actions";
pub const ACTION_SESSION_SCOPE: &str = "security_scan_action_sessions";
pub const ARCHIVE_INDEX_SCOPE: &str = "security_scan_archive_index";
pub const RUN_QUEUE: &str = "security-scan-run";
pub const ACTION_QUEUE: &str = "security-scan-action";
const STATE_PREFIX: &str = "security-scan";
const STATE_GET_ID: &str = "security-scan::state::get";
const STATE_LIST_ID: &str = "security-scan::state::list";
const STATE_CAS_ID: &str = "security-scan::state::compare-and-set";
const CLAIM_NAMESPACE_ID: &str = "state::claim-namespace";
const EXECUTE_ID: &str = "security-scan::execute";
const ACTION_EXECUTE_ID: &str = "security-scan::action-execute";
const GITHUB_API_ID: &str = "github::api";
const GITHUB_ALERT_LIMIT: usize = 500;
const RUN_STREAM_NAME: &str = "security-scan:runs";
const RUN_STREAM_GROUP: &str = "all";
const RUN_UPDATED_EVENT_TYPE: &str = "security-scan:updated";
const RECONCILIATION_UPDATED_EVENT_TYPE: &str = "security-scan:reconciliation-updated";
const STORAGE_PUT_ID: &str = "storage::putObject";
const STORAGE_GET_ID: &str = "storage::getObject";
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
    action_session_backfill_pending: Arc<AtomicBool>,
    private_state_ready: Arc<AtomicBool>,
    archive: Arc<Mutex<Option<ArchiveConfigV1>>>,
}

impl IiiRuntime {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self {
            iii,
            pending_index_repairs: Arc::new(Mutex::new(HashSet::new())),
            run_index_backfill_pending: Arc::new(AtomicBool::new(true)),
            action_session_backfill_pending: Arc::new(AtomicBool::new(true)),
            private_state_ready: Arc::new(AtomicBool::new(false)),
            archive: Arc::new(Mutex::new(None)),
        }
    }

    pub fn private_state_is_ready(&self) -> bool {
        self.private_state_ready.load(Ordering::Acquire)
    }

    pub async fn claim_private_state(&self) -> Result<(), SecurityScanError> {
        self.retry_boot_call(CLAIM_NAMESPACE_ID, || {
            self.call(
                CLAIM_NAMESPACE_ID,
                json!({
                    "functions_prefix": STATE_PREFIX,
                    "scopes": [
                        RUN_SCOPE,
                        RUN_INDEX_SCOPE,
                        RECONCILIATION_SCOPE,
                        ACTION_SCOPE,
                        ACTION_SESSION_SCOPE,
                        ARCHIVE_INDEX_SCOPE
                    ],
                }),
                None,
                Some(5_000),
            )
        })
        .await?;
        self.private_state_ready.store(true, Ordering::Release);
        Ok(())
    }

    pub async fn ensure_queue(&self) -> Result<(), SecurityScanError> {
        for definition in [queue_definition(), action_queue_definition()] {
            let definition = definition.clone();
            self.retry_boot_call("queue::define", || {
                self.call("queue::define", definition.clone(), None, Some(5_000))
            })
            .await?;
        }
        Ok(())
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
        value: Option<Value>,
    ) -> Result<CasOutcome, SecurityScanError> {
        let payload = state_compare_and_set_payload(scope, key, expected, value);
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

    pub async fn backfill_action_session_index(&self) -> Result<usize, SecurityScanError> {
        let result = async {
            let mut inserted = 0;
            for action in self.list_actions().await? {
                let Some(harness) = action.harness.as_ref() else {
                    continue;
                };
                if self
                    .remember_action_session(&harness.session_id, &action.action_id)
                    .await?
                {
                    inserted += 1;
                }
            }
            Ok(inserted)
        }
        .await;
        mark_backfill_complete(&self.action_session_backfill_pending, &result);
        result
    }

    pub async fn retry_action_session_backfill(&self) -> Result<Option<usize>, SecurityScanError> {
        if !self.action_session_backfill_pending.load(Ordering::Acquire) {
            return Ok(None);
        }
        self.backfill_action_session_index().await.map(Some)
    }

    async fn remember_action_session(
        &self,
        session_id: &str,
        action_id: &str,
    ) -> Result<bool, SecurityScanError> {
        let value = json!({ "schema_version": "1", "action_id": action_id });
        match self
            .compare_and_set_in_scope(ACTION_SESSION_SCOPE, session_id, None, Some(value.clone()))
            .await?
        {
            CasOutcome::Swapped => Ok(true),
            CasOutcome::Current(current) if current == value => Ok(false),
            CasOutcome::Current(_) => Err(SecurityScanError::Dependency(format!(
                "Harness session {session_id} is already linked to another security action"
            ))),
        }
    }

    async fn forget_action_session(
        &self,
        session_id: &str,
        action_id: &str,
    ) -> Result<(), SecurityScanError> {
        let value = json!({ "schema_version": "1", "action_id": action_id });
        let _ = self
            .compare_and_set_in_scope(ACTION_SESSION_SCOPE, session_id, Some(value), None)
            .await?;
        Ok(())
    }

    pub async fn commit_action(
        &self,
        request: crate::ActionCommitRequestV1,
    ) -> Result<crate::ActionCommitResponseV1, SecurityScanError> {
        let action = self
            .get_action(&request.action_id)
            .await?
            .ok_or_else(|| SecurityScanError::InvalidRequest("unknown security action".into()))?;
        let target = crate::action::authorize_action_worktree(
            &action,
            &request.action_id,
            &request.capability,
        )?;
        let message = request.message.trim();
        if message.is_empty() || message.len() > 500 {
            return Err(SecurityScanError::InvalidRequest(
                "commit message must contain 1 to 500 characters".into(),
            ));
        }
        let commit_sha = git_gateway::commit(target, message).await?;
        Ok(crate::ActionCommitResponseV1 { commit_sha })
    }

    pub async fn push_action(
        &self,
        request: crate::ActionPushRequestV1,
    ) -> Result<crate::ActionPushResponseV1, SecurityScanError> {
        let action = self
            .get_action(&request.action_id)
            .await?
            .ok_or_else(|| SecurityScanError::InvalidRequest("unknown security action".into()))?;
        let target = crate::action::authorize_action_worktree(
            &action,
            &request.action_id,
            &request.capability,
        )?;
        let branch = git_gateway::push(target).await?;
        Ok(crate::ActionPushResponseV1 { branch })
    }

    async fn completed_session(
        &self,
        harness: &crate::HarnessRunV1,
    ) -> Result<Option<crate::TurnCompletedEventV1>, SecurityScanError> {
        let response = self
            .call(
                "harness::status",
                json!({ "session_id": harness.session_id, "verbose": true }),
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

    async fn compare_and_set(
        &self,
        key: &str,
        expected: Option<Value>,
        value: Option<Value>,
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
                    .transpose()?;
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

    fn emit_action_update(&self, action: &crate::SecurityActionRecordV1) {
        let runtime = self.clone();
        let payload = action_update_payload(action);
        let action_id = action.action_id.clone();
        tokio::spawn(async move {
            if let Err(error) = runtime
                .call("stream::send", payload, None, Some(EVENT_TIMEOUT_MS))
                .await
            {
                tracing::warn!(
                    %action_id,
                    %error,
                    "security scan action live-update doorbell failed"
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

    async fn jail_unattended_session(&self, plan: &AnalysisPlan) {
        if !plan.unattended {
            return;
        }
        match self.approval_gate_is_live().await {
            Ok(true) => {
                if let Err(error) = self
                    .call(
                        "approval::set-mode",
                        json!({
                            "session_id": plan.session_id,
                            "mode": "full",
                        }),
                        None,
                        Some(RPC_TIMEOUT_MS),
                    )
                    .await
                {
                    tracing::warn!(
                        session_id = %plan.session_id,
                        %error,
                        "could not auto-approve the read-only analysis session"
                    );
                }
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(
                    session_id = %plan.session_id,
                    %error,
                    "could not detect approval-gate for analysis session jail"
                );
            }
        }
        if plan.filesystem_root.is_empty() {
            return;
        }
        if let Err(error) = self
            .call(
                "harness::filesystem::grant",
                json!({
                    "session_id": plan.session_id,
                    "root": plan.filesystem_root,
                }),
                None,
                Some(RPC_TIMEOUT_MS),
            )
            .await
        {
            tracing::warn!(
                session_id = %plan.session_id,
                %error,
                "could not pre-grant the analysis checkout filesystem jail"
            );
        }
    }
}

fn state_compare_and_set_payload(
    scope: &str,
    key: &str,
    expected: Option<Value>,
    value: Option<Value>,
) -> Value {
    let mut payload = json!({
        "scope": scope,
        "key": key,
        "value": value.unwrap_or(Value::Null),
    });
    if let Some(expected) = expected {
        payload["expected"] = expected;
    }
    payload
}

include!("iii_runtime/wire.rs");
include!("iii_runtime/tests.rs");
