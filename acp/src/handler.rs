use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use dashmap::mapref::entry::Entry;
use dashmap::{DashMap, DashSet};
use iii_sdk::errors::Error as IiiError;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::runtime::FunctionRef;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::session::{
    self, AGENT_EVENTS_STREAM, ActivePromptClaim, CloseHistoryResult, HistoryOwnerResult,
    PromptClaimResult, PromptDispatchIdentity, PromptRecoveryFinishResult, PromptRecoveryResult,
    SessionRecord, append_history_once, append_session_to_index, begin_prompt_recovery,
    claim_prompt, close_history_owned_by, durable_publish, finish_prompt_recovery,
    history_owned_by, now_ms, read_active_prompt_claim, read_history, read_session_index,
    release_prompt_claim, remove_session_from_index, restore_history_owner, scope,
    session_history_key, session_key, set_history_owner, state_compare_and_set, state_delete,
    state_get, state_set,
};
use crate::transport::Outbound;
use crate::types::{
    ACP_PROTOCOL_VERSION, INTERNAL_ERROR, INVALID_PARAMS, JsonRpcResponse, METHOD_NOT_FOUND,
    SessionCancelParams, SessionLoadParams, SessionNewParams, SessionPromptParams,
    SessionResumeParams, SessionSetConfigOptionParams, SessionSetModeParams, parse,
};

// Canonical iii brain function. Any worker exposing this id with the
// turn-orchestrator wire shape (session_id, messages, model, ...) can
// drive iii-acp without an adapter.
pub const DEFAULT_BRAIN_FN: &str = "run::start_and_wait";
const BRAIN_TIMEOUT_MS: u64 = 600_000;
const BRAIN_CANCEL_GRACE_MS: u64 = 60_000;
const PROMPT_RECOVERY_AFTER_MS: i64 = (BRAIN_TIMEOUT_MS + BRAIN_CANCEL_GRACE_MS) as i64;

#[derive(Clone)]
struct CancelHandle {
    flag: Arc<AtomicBool>,
    notify: Arc<tokio::sync::Notify>,
}

impl CancelHandle {
    fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(tokio::sync::Notify::new()),
        }
    }

    fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_one();
    }

    fn same_instance(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.flag, &other.flag)
    }

    async fn wait(&self) {
        if self.flag.load(Ordering::SeqCst) {
            return;
        }
        self.notify.notified().await;
    }
}

struct BrainOutcome {
    stop_reason: String,
    terminal_confirmed: bool,
    pending: Option<tokio::task::JoinHandle<Result<Value, IiiError>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptClaimFinish {
    Definitive,
    Ambiguous,
}

pub struct AcpHandler {
    iii: IIIClient,
    conn_id: String,
    initialized: AtomicBool,
    // Cancel handle per active session: AtomicBool gates the abort state for
    // tool handlers that poll, Notify wakes any awaiter (run_external_brain)
    // immediately on session/cancel without a 100ms poll loop.
    cancels: DashMap<String, CancelHandle>,
    // Per-session write mutex serializing append_history calls in-process.
    // Engine state::update lacks an array-append op so each append is a
    // read-modify-write; without this lock concurrent agent::events for one
    // session race and drop entries.
    history_locks: Arc<DashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    update_seq: Arc<AtomicU64>,
    outbound: Arc<Outbound>,
    brain_fn: Option<String>,
    brain_stop_fn: Option<String>,
    brain_model: Option<String>,
    brain_provider: Option<String>,
    brain_system_prompt: Option<String>,
    // Session ids owned by this connection. The agent::events stream
    // subscriber filters by this set so we don't forward events for
    // sessions another iii-acp subprocess owns. Also written by
    // session/new and session/close so close cleans up.
    owned_sessions: Arc<DashSet<String>>,
    // Trigger + function guards. Dropping them tears the registration
    // down on the engine, so they live for the lifetime of the handler.
    _event_subscriber: Option<iii_sdk::trigger::Trigger>,
    _event_function: Option<FunctionRef>,
    // True iff agent::events stream subscriber registered cleanly. When an
    // external brain is configured but this is false, session/prompt fails
    // fast with an actionable error rather than running the brain whose
    // updates would silently never reach stdout.
    event_subscriber_healthy: bool,
}

pub struct BrainConfig {
    pub function_id: Option<String>,
    pub stop_function_id: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub system_prompt: Option<String>,
}

impl AcpHandler {
    pub fn new(iii: IIIClient, outbound: Arc<Outbound>, brain: BrainConfig) -> Self {
        let conn_id = Uuid::new_v4().to_string();
        let update_seq = Arc::new(AtomicU64::new(0));
        let owned_sessions: Arc<DashSet<String>> = Arc::new(DashSet::new());
        let history_locks: Arc<DashMap<String, Arc<tokio::sync::Mutex<()>>>> =
            Arc::new(DashMap::new());
        tracing::info!(%conn_id, "acp handler initialized");

        // Subscribe to the canonical agent::events stream once when an
        // external brain is configured. Echo brain bypasses (it emits
        // session/update directly via send_notification).
        let brain_configured = brain.function_id.is_some();
        let (event_subscriber, event_function) = if brain_configured {
            register_event_subscriber(
                &iii,
                &conn_id,
                &outbound,
                &update_seq,
                &owned_sessions,
                &history_locks,
            )
        } else {
            (None, None)
        };
        // Healthy when (a) no external brain is configured (echo path
        // doesn't need the subscriber) or (b) both function + trigger
        // registered. Failed registration here is logged inside
        // register_event_subscriber.
        let event_subscriber_healthy =
            !brain_configured || (event_subscriber.is_some() && event_function.is_some());

        Self {
            iii,
            conn_id,
            initialized: AtomicBool::new(false),
            cancels: DashMap::new(),
            history_locks,
            update_seq,
            outbound,
            brain_fn: brain.function_id,
            brain_stop_fn: brain.stop_function_id,
            brain_model: brain.model,
            brain_provider: brain.provider,
            brain_system_prompt: brain.system_prompt,
            owned_sessions,
            _event_subscriber: event_subscriber,
            _event_function: event_function,
            event_subscriber_healthy,
        }
    }

    fn history_lock(&self, session_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.history_locks
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    pub fn outbound(&self) -> Arc<Outbound> {
        self.outbound.clone()
    }

    pub fn conn_id(&self) -> &str {
        &self.conn_id
    }

    pub async fn handle(self: &Arc<Self>, body: Value) -> Option<Value> {
        let method = body.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = body.get("id").cloned();
        let params = body.get("params").cloned();
        let is_notification = id.is_none();

        let result = match method {
            "initialize" => self.initialize(params).await,
            "authenticate" => Ok(json!({})),
            "session/new" => self.session_new(params).await,
            "session/load" => self.session_load(params).await,
            "session/resume" => self.session_resume(params).await,
            "session/list" => self.session_list().await,
            "session/prompt" => self.session_prompt(params).await,
            "session/cancel" => self.session_cancel(params).await.map(|_| Value::Null),
            "session/close" => self.session_close(params).await.map(|_| Value::Null),
            "session/set_mode" => self.session_set_mode(params).await,
            "session/set_config_option" => self.session_set_config_option(params).await,
            _ => Err((METHOD_NOT_FOUND, format!("Unknown method: {}", method))),
        };

        if is_notification {
            if let Err((code, msg)) = result {
                tracing::warn!(method, code, %msg, "notification handler returned error");
            }
            return None;
        }

        Some(json!(match result {
            Ok(v) => JsonRpcResponse::success(id, v),
            Err((code, msg)) => JsonRpcResponse::error(id, code, msg),
        }))
    }

    async fn initialize(&self, _params: Option<Value>) -> Result<Value, (i32, String)> {
        self.initialized.store(true, Ordering::SeqCst);
        Ok(json!({
            "protocolVersion": ACP_PROTOCOL_VERSION,
            "agentCapabilities": {
                "loadSession": true,
                "promptCapabilities": {
                    "image": false,
                    "audio": false,
                    "embeddedContext": true
                },
                "mcpCapabilities": {
                    "http": true,
                    "sse": false
                },
                "sessionCapabilities": {
                    "list": {},
                    "close": {},
                    "resume": {}
                }
            },
            "agentInfo": {
                "name": "iii-acp",
                "title": "iii Agent",
                "version": env!("CARGO_PKG_VERSION")
            }
        }))
    }

    async fn session_new(&self, params: Option<Value>) -> Result<Value, (i32, String)> {
        self.require_initialized()?;
        let p: SessionNewParams = parse(params).map_err(|e| (INVALID_PARAMS, e))?;
        let session_id = format!("sess_{}", Uuid::new_v4().simple());
        let now = now_ms();
        let record = SessionRecord {
            session_id: session_id.clone(),
            conn_id: self.conn_id.clone(),
            cwd: p.cwd,
            mcp_servers: p.mcp_servers,
            created_at_ms: now,
            last_activity_ms: now,
            mode: None,
            config_options: serde_json::Map::new(),
        };
        let key = session_key(&session_id);
        let value = serde_json::to_value(&record).map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
        state_set(&self.iii, &scope(), &key, value)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
        append_session_to_index(&self.iii, &session_id)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
        match set_history_owner(&self.iii, &session_id, &self.conn_id)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?
        {
            HistoryOwnerResult::AlreadyOwned | HistoryOwnerResult::Transferred { .. } => {}
            HistoryOwnerResult::ActivePrompt(_) | HistoryOwnerResult::Closed => {
                return Err((
                    INTERNAL_ERROR,
                    "new session history could not be claimed".to_string(),
                ));
            }
        }
        self.owned_sessions.insert(session_id.clone());
        Ok(json!({ "sessionId": session_id }))
    }

    async fn session_load(&self, params: Option<Value>) -> Result<Value, (i32, String)> {
        self.require_initialized()?;
        let p: SessionLoadParams = parse(params).map_err(|e| (INVALID_PARAMS, e))?;
        let key = session_key(&p.session_id);
        let record_value = state_get(&self.iii, &scope(), &key)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?
            .ok_or_else(|| {
                (
                    INVALID_PARAMS,
                    format!("session not found: {}", p.session_id),
                )
            })?;
        self.transfer_session_ownership(&p.session_id, record_value, |_| {})
            .await?;
        self.owned_sessions.insert(p.session_id.clone());
        let history = read_history(&self.iii, &p.session_id)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
        for entry in history {
            // _meta belongs inside params per ACP spec (per-type extensibility).
            // The JSON-RPC envelope itself only carries jsonrpc/method/params.
            let update = json!({
                "sessionId": p.session_id,
                "update": entry,
                "_meta": { "iii.dev/historical": true },
            });
            self.send_notification("session/update", update).await;
        }
        Ok(json!({}))
    }

    async fn session_list(&self) -> Result<Value, (i32, String)> {
        self.require_initialized()?;
        let ids = read_session_index(&self.iii)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
        let mut sessions = Vec::with_capacity(ids.len());
        for id in ids {
            let key = session_key(&id);
            if let Some(rec) = state_get(&self.iii, &scope(), &key)
                .await
                .map_err(|e| (INTERNAL_ERROR, e.to_string()))?
            {
                sessions.push(rec);
            }
        }
        Ok(json!({ "sessions": sessions }))
    }

    async fn session_prompt(
        self: &Arc<Self>,
        params: Option<Value>,
    ) -> Result<Value, (i32, String)> {
        self.require_initialized()?;
        let p: SessionPromptParams = parse(params).map_err(|e| (INVALID_PARAMS, e))?;
        let cancel = CancelHandle::new();
        match self.cancels.entry(p.session_id.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(cancel.clone());
            }
            Entry::Occupied(_) => {
                return Err((
                    INVALID_PARAMS,
                    "a prompt is already active for this session".to_string(),
                ));
            }
        }
        let result = self.session_prompt_inner(&p, &cancel).await;
        let (claim_id, outcome) = match result {
            Ok(result) => result,
            Err(error) => {
                remove_local_cancel(&self.cancels, &p.session_id, &cancel);
                return Err(error);
            }
        };
        let BrainOutcome {
            stop_reason,
            terminal_confirmed,
            pending,
        } = outcome;
        if terminal_confirmed {
            if self.finish_prompt_claim(&p.session_id, &claim_id).await
                == PromptClaimFinish::Definitive
            {
                remove_local_cancel(&self.cancels, &p.session_id, &cancel);
            }
        } else {
            if let Some(brain) = pending {
                self.monitor_pending_brain(p.session_id.clone(), claim_id, cancel, brain);
            } else {
                tracing::error!(
                    session_id = p.session_id,
                    "external brain completion is unknown; prompt claim remains recovery-required"
                );
            }
        }
        Ok(json!({ "stopReason": stop_reason }))
    }

    async fn session_prompt_inner(
        &self,
        p: &SessionPromptParams,
        cancel: &CancelHandle,
    ) -> Result<(String, BrainOutcome), (i32, String)> {
        let key = session_key(&p.session_id);
        let record_value = state_get(&self.iii, &scope(), &key)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?
            .ok_or_else(|| {
                (
                    INVALID_PARAMS,
                    format!("session not found: {}", p.session_id),
                )
            })?;
        let record: SessionRecord = serde_json::from_value(record_value)
            .map_err(|e| (INTERNAL_ERROR, format!("session decode: {}", e)))?;
        if !record_owner_matches(&record, &self.conn_id) {
            return Err((
                INVALID_PARAMS,
                "session is owned by another connection; load or resume it before prompting"
                    .to_string(),
            ));
        }
        if self.brain_fn.is_some() && !self.event_subscriber_healthy {
            return Err((
                INTERNAL_ERROR,
                "iii-acp: agent::events stream subscriber failed to register at startup; \
                 external brain updates would not reach the editor. Check engine logs and \
                 ensure `iii-stream` worker is active before retrying."
                    .to_string(),
            ));
        }

        let claim_id = Uuid::new_v4().to_string();
        let claim_result = {
            let lock = self.history_lock(&p.session_id);
            let _g = lock.lock().await;
            claim_prompt(
                &self.iii,
                &p.session_id,
                &self.conn_id,
                &claim_id,
                now_ms(),
                self.prompt_dispatch_identity(),
                json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": { "type": "text", "text": prompt_to_text(&p.prompt) }
                }),
            )
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?
        };
        match claim_result {
            PromptClaimResult::Claimed => {}
            PromptClaimResult::AlreadyActive => {
                return Err((
                    INVALID_PARAMS,
                    "an active or recovery-required prompt already exists for this session"
                        .to_string(),
                ));
            }
            PromptClaimResult::NotOwner => {
                return Err((
                    INVALID_PARAMS,
                    "session is owned by another connection; load or resume it before prompting"
                        .to_string(),
                ));
            }
            PromptClaimResult::Closed => {
                return Err((INVALID_PARAMS, "session is closed".to_string()));
            }
        }
        self.owned_sessions.insert(p.session_id.clone());

        let outcome = self
            .run_brain(&p.session_id, &record.cwd, &p.prompt, cancel)
            .await;
        Ok((claim_id, outcome))
    }

    async fn session_cancel(&self, params: Option<Value>) -> Result<(), (i32, String)> {
        self.require_initialized()?;
        let p: SessionCancelParams = parse(params).map_err(|e| (INVALID_PARAMS, e))?;
        self.require_session_owner(&p.session_id).await?;
        let local_cancel = self.cancels.get(&p.session_id).map(|handle| handle.clone());
        signal_local_cancel(local_cancel.as_ref());
        let _ = durable_publish(
            &self.iii,
            &session::cancel_topic(&self.conn_id, &p.session_id),
            json!({ "reason": "client" }),
        )
        .await;
        Ok(())
    }

    async fn session_close(&self, params: Option<Value>) -> Result<(), (i32, String)> {
        self.require_initialized()?;
        let p: SessionLoadParams = parse(params).map_err(|e| (INVALID_PARAMS, e))?;
        let lock = self.history_lock(&p.session_id);
        let _g = lock.lock().await;
        let closed = close_history_owned_by(&self.iii, &p.session_id, &self.conn_id)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
        match closed {
            // Only the current owner can create the tombstone. Once it exists,
            // any reconnect may safely retry the remaining idempotent cleanup.
            CloseHistoryResult::Closed | CloseHistoryResult::AlreadyClosed => {}
            CloseHistoryResult::ActivePrompt => {
                return Err((
                    INVALID_PARAMS,
                    "session has an active or recovery-required prompt; cancel it and wait for terminal completion before closing"
                        .to_string(),
                ));
            }
            CloseHistoryResult::NotOwner => {
                return Err((
                    INVALID_PARAMS,
                    "session is owned by another connection".to_string(),
                ));
            }
        }
        let scope = scope();
        let mut errs: Vec<String> = Vec::new();

        if let Err(e) = state_delete(&self.iii, &scope, &session_key(&p.session_id)).await {
            errs.push(format!("session record: {}", e));
        }
        if let Err(e) = remove_session_from_index(&self.iii, &p.session_id).await {
            errs.push(format!("index: {}", e));
        }
        if errs.is_empty()
            && let Err(error) =
                state_delete(&self.iii, &scope, &session_history_key(&p.session_id)).await
        {
            match state_get(&self.iii, &scope, &session_history_key(&p.session_id)).await {
                Ok(None) => {}
                _ => errs.push(format!("history tombstone: {}", error)),
            }
        }

        if let Some((_, handle)) = self.cancels.remove(&p.session_id) {
            handle.cancel();
        }
        self.owned_sessions.remove(&p.session_id);

        if errs.is_empty() {
            Ok(())
        } else {
            Err((
                INTERNAL_ERROR,
                format!("session_close partial failure: {}", errs.join("; ")),
            ))
        }
    }

    // session/resume — like session/load but skips history replay. Per
    // ACP spec: "useful for agents that can resume sessions but don't
    // implement full session loading." We keep history but emit nothing.
    // The new cwd / mcpServers fields are persisted on the session record
    // so subsequent session/prompt calls see the refreshed environment.
    async fn session_resume(&self, params: Option<Value>) -> Result<Value, (i32, String)> {
        self.require_initialized()?;
        let p: SessionResumeParams = parse(params).map_err(|e| (INVALID_PARAMS, e))?;
        let key = session_key(&p.session_id);
        let scope = scope();
        let rec_value = state_get(&self.iii, &scope, &key)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?
            .ok_or_else(|| {
                (
                    INVALID_PARAMS,
                    format!("session not found: {}", p.session_id),
                )
            })?;
        self.transfer_session_ownership(&p.session_id, rec_value, |record| {
            record.cwd = p.cwd;
            record.mcp_servers = p.mcp_servers;
            record.last_activity_ms = now_ms();
        })
        .await?;
        self.owned_sessions.insert(p.session_id.clone());
        Ok(json!({}))
    }

    async fn transfer_session_ownership<F>(
        &self,
        session_id: &str,
        record_value: Value,
        mutate: F,
    ) -> Result<SessionRecord, (i32, String)>
    where
        F: FnOnce(&mut SessionRecord),
    {
        let lock = self.history_lock(session_id);
        let _g = lock.lock().await;
        let mut record: SessionRecord = serde_json::from_value(record_value.clone())
            .map_err(|e| (INTERNAL_ERROR, format!("session decode: {}", e)))?;
        let owner_result = self.acquire_history_owner(session_id).await?;
        match &owner_result {
            HistoryOwnerResult::ActivePrompt(_) => unreachable!("active prompt handled above"),
            HistoryOwnerResult::Closed => {
                return Err((INVALID_PARAMS, "session is closed".to_string()));
            }
            HistoryOwnerResult::AlreadyOwned | HistoryOwnerResult::Transferred { .. } => {}
        }

        record.conn_id = self.conn_id.clone();
        mutate(&mut record);
        let mut new_value =
            serde_json::to_value(&record).map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
        if new_value == record_value {
            return Ok(record);
        }
        let scope = scope();
        let key = session_key(session_id);
        let swapped = match state_compare_and_set(
            &self.iii,
            &scope,
            &key,
            Some(&record_value),
            new_value.clone(),
        )
        .await
        {
            Ok(swapped) => swapped,
            Err(error) => {
                let observed = state_get(&self.iii, &scope, &key).await;
                match observed {
                    Ok(Some(value)) if value == new_value => true,
                    Ok(Some(value)) => {
                        let observed_record =
                            serde_json::from_value::<SessionRecord>(value.clone()).map_err(
                                |decode| (INTERNAL_ERROR, format!("session decode: {}", decode)),
                            )?;
                        if record_owner_matches(&observed_record, &self.conn_id) {
                            record = observed_record;
                            new_value = value;
                            true
                        } else {
                            self.rollback_history_transfer(session_id, &owner_result)
                                .await;
                            return Err((INTERNAL_ERROR, error.to_string()));
                        }
                    }
                    _ => {
                        self.rollback_history_transfer(session_id, &owner_result)
                            .await;
                        return Err((INTERNAL_ERROR, error.to_string()));
                    }
                }
            }
        };
        if !swapped {
            self.rollback_history_transfer(session_id, &owner_result)
                .await;
            return Err((
                INVALID_PARAMS,
                "session ownership changed concurrently; retry load or resume".to_string(),
            ));
        }
        match history_owned_by(&self.iii, session_id, &self.conn_id).await {
            Ok(true) => {}
            ownership => {
                return match ownership {
                    Ok(false) => {
                        if matches!(
                            state_compare_and_set(
                                &self.iii,
                                &scope,
                                &key,
                                Some(&new_value),
                                record_value,
                            )
                            .await,
                            Ok(true)
                        ) {
                            self.rollback_history_transfer(session_id, &owner_result)
                                .await;
                        }
                        Err((
                            INVALID_PARAMS,
                            "session ownership changed concurrently; retry load or resume"
                                .to_string(),
                        ))
                    }
                    Err(error) => Err((INTERNAL_ERROR, error.to_string())),
                    Ok(true) => unreachable!(),
                };
            }
        }
        Ok(record)
    }

    async fn acquire_history_owner(
        &self,
        session_id: &str,
    ) -> Result<HistoryOwnerResult, (i32, String)> {
        let owner_result = set_history_owner(&self.iii, session_id, &self.conn_id)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
        let HistoryOwnerResult::ActivePrompt(claim) = owner_result else {
            return Ok(owner_result);
        };
        let age_ms = now_ms().saturating_sub(claim.started_at_ms);
        if age_ms < PROMPT_RECOVERY_AFTER_MS {
            return Err((
                INVALID_PARAMS,
                format!(
                    "session has an active prompt; retry recovery in {} seconds",
                    (PROMPT_RECOVERY_AFTER_MS - age_ms + 999) / 1_000
                ),
            ));
        }
        self.recover_stale_prompt(session_id, claim).await
    }

    async fn recover_stale_prompt(
        &self,
        session_id: &str,
        claim: ActivePromptClaim,
    ) -> Result<HistoryOwnerResult, (i32, String)> {
        let current_dispatch = self.prompt_dispatch_identity();
        let Some(claim_dispatch) = claim.dispatch.as_ref() else {
            return Err((
                INVALID_PARAMS,
                "session has a legacy recovery-required prompt without dispatch identity; manual recovery is required"
                    .to_string(),
            ));
        };
        if !prompt_dispatch_matches(&claim, &current_dispatch) {
            return Err((
                INVALID_PARAMS,
                "session prompt was dispatched by a different brain or namespace; recovery with the original ACP configuration is required"
                    .to_string(),
            ));
        }
        let Some(stop_fn) = claim_dispatch.stop_function_id.as_deref() else {
            return Err((
                INVALID_PARAMS,
                "session has a stale recovery-required prompt without a matching stop function; manual recovery is required"
                    .to_string(),
            ));
        };
        let stop_fn = stop_fn.to_string();
        let local_cancel = self.cancels.get(session_id).map(|handle| handle.clone());
        let recovery_claim_id = Uuid::new_v4().to_string();
        let recovery_claim = match begin_prompt_recovery(
            &self.iii,
            session_id,
            &claim,
            &self.conn_id,
            &recovery_claim_id,
            now_ms(),
        )
        .await
        .map_err(|error| (INTERNAL_ERROR, error.to_string()))?
        {
            PromptRecoveryResult::Claimed(recovery_claim) => recovery_claim,
            PromptRecoveryResult::Changed => {
                return Err((
                    INVALID_PARAMS,
                    "the stale prompt claim changed before recovery could isolate it; retry load or resume"
                        .to_string(),
                ));
            }
            PromptRecoveryResult::Closed => {
                return Err((INVALID_PARAMS, "session is closed".to_string()));
            }
        };
        let deadline = tokio::time::Instant::now() + Duration::from_millis(BRAIN_CANCEL_GRACE_MS);
        loop {
            let current_claim = read_active_prompt_claim(&self.iii, session_id)
                .await
                .map_err(|error| (INTERNAL_ERROR, error.to_string()))?;
            if current_claim.as_ref() != Some(&recovery_claim) {
                return Err((
                    INVALID_PARAMS,
                    "the prompt recovery claim changed; refusing to stop an unpinned turn"
                        .to_string(),
                ));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err((
                    INVALID_PARAMS,
                    "the external brain accepted cancellation but did not confirm terminal completion; the recovery claim remains active"
                        .to_string(),
                ));
            }
            let stop = self
                .iii
                .trigger(external_brain_stop_request(&stop_fn, session_id))
                .await
                .map_err(|error| (INTERNAL_ERROR, error.to_string()))?;
            if stop_reports_terminal(&stop) {
                let owner_result = match finish_prompt_recovery(
                    &self.iii,
                    session_id,
                    &recovery_claim,
                    &self.conn_id,
                )
                .await
                .map_err(|error| (INTERNAL_ERROR, error.to_string()))?
                {
                    PromptRecoveryFinishResult::AlreadyOwned => HistoryOwnerResult::AlreadyOwned,
                    PromptRecoveryFinishResult::Transferred { previous_owner } => {
                        HistoryOwnerResult::Transferred { previous_owner }
                    }
                    PromptRecoveryFinishResult::Changed => {
                        return Err((
                            INVALID_PARAMS,
                            "the prompt recovery claim changed before ownership transfer; retry load or resume"
                                .to_string(),
                        ));
                    }
                    PromptRecoveryFinishResult::Closed => {
                        return Err((INVALID_PARAMS, "session is closed".to_string()));
                    }
                };
                if let Some(local_cancel) = local_cancel.as_ref() {
                    remove_local_cancel(&self.cancels, session_id, local_cancel);
                }
                return Ok(owner_result);
            }
            if !stop_was_accepted(&stop) {
                return Err((
                    INVALID_PARAMS,
                    "the external brain could not confirm stale prompt cancellation".to_string(),
                ));
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    fn prompt_dispatch_identity(&self) -> PromptDispatchIdentity {
        PromptDispatchIdentity {
            namespace: self.iii.namespace(),
            brain_function_id: self.brain_fn.clone(),
            stop_function_id: self.brain_stop_fn.clone(),
        }
    }

    async fn rollback_history_transfer(&self, session_id: &str, owner_result: &HistoryOwnerResult) {
        let HistoryOwnerResult::Transferred { previous_owner } = owner_result else {
            return;
        };
        match restore_history_owner(
            &self.iii,
            session_id,
            &self.conn_id,
            previous_owner.as_deref(),
        )
        .await
        {
            Ok(true) => {}
            Ok(false) => tracing::error!(
                session_id,
                "session history ownership changed before transfer rollback"
            ),
            Err(error) => tracing::error!(
                %error,
                session_id,
                "failed to roll back session history ownership"
            ),
        }
    }

    // session/set_mode — store the mode id on the session record so the
    // brain can read it on the next prompt turn (e.g. via a system prompt
    // suffix or per-mode tool gating). Validation of the mode id against
    // any agent-specific catalog is left to the brain worker.
    async fn session_set_mode(&self, params: Option<Value>) -> Result<Value, (i32, String)> {
        self.require_initialized()?;
        let p: SessionSetModeParams = parse(params).map_err(|e| (INVALID_PARAMS, e))?;
        self.update_session_record(&p.session_id, |rec| {
            rec.mode = Some(p.mode_id.clone());
        })
        .await?;
        Ok(json!({}))
    }

    // session/set_config_option — store an arbitrary configId/value pair
    // on the session record. Same rationale as set_mode: persistence lives
    // here, semantics live in the brain.
    async fn session_set_config_option(
        &self,
        params: Option<Value>,
    ) -> Result<Value, (i32, String)> {
        self.require_initialized()?;
        let p: SessionSetConfigOptionParams = parse(params).map_err(|e| (INVALID_PARAMS, e))?;
        self.update_session_record(&p.session_id, |rec| {
            rec.config_options
                .insert(p.config_id.clone(), p.value.clone());
        })
        .await?;
        Ok(json!({}))
    }

    // Read-modify-write of one session record under the per-session
    // history mutex (we reuse it to avoid adding a parallel lock map for
    // the same session). Returns INVALID_PARAMS if the session is
    // missing, INTERNAL_ERROR on backend failure.
    async fn update_session_record<F>(
        &self,
        session_id: &str,
        mutate: F,
    ) -> Result<(), (i32, String)>
    where
        F: FnOnce(&mut SessionRecord),
    {
        let scope = scope();
        let key = session_key(session_id);
        let lock = self.history_lock(session_id);
        let _g = lock.lock().await;
        self.require_session_owner(session_id).await?;
        let rec_value = state_get(&self.iii, &scope, &key)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?
            .ok_or_else(|| (INVALID_PARAMS, format!("session not found: {}", session_id)))?;
        let mut record: SessionRecord = serde_json::from_value(rec_value.clone())
            .map_err(|e| (INTERNAL_ERROR, format!("session decode: {}", e)))?;
        if !record_owner_matches(&record, &self.conn_id) {
            return Err((
                INVALID_PARAMS,
                "session ownership changed concurrently; load or resume it first".to_string(),
            ));
        }
        mutate(&mut record);
        record.last_activity_ms = now_ms();
        let new_value =
            serde_json::to_value(&record).map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
        if state_compare_and_set(&self.iii, &scope, &key, Some(&rec_value), new_value)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?
        {
            Ok(())
        } else {
            Err((
                INVALID_PARAMS,
                "session ownership or configuration changed concurrently; retry".to_string(),
            ))
        }
    }

    async fn require_session_owner(&self, session_id: &str) -> Result<(), (i32, String)> {
        let record = state_get(&self.iii, &scope(), &session_key(session_id))
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?
            .ok_or_else(|| (INVALID_PARAMS, format!("session not found: {}", session_id)))?;
        let record: SessionRecord = serde_json::from_value(record)
            .map_err(|e| (INTERNAL_ERROR, format!("session decode: {}", e)))?;
        if !record_owner_matches(&record, &self.conn_id) {
            return Err((
                INVALID_PARAMS,
                "session is owned by another connection; load or resume it first".to_string(),
            ));
        }
        let owned = history_owned_by(&self.iii, session_id, &self.conn_id)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
        if owned {
            Ok(())
        } else {
            Err((
                INVALID_PARAMS,
                "session is owned by another connection; load or resume it first".to_string(),
            ))
        }
    }

    async fn run_brain(
        &self,
        session_id: &str,
        cwd: &str,
        prompt: &[Value],
        cancel: &CancelHandle,
    ) -> BrainOutcome {
        if let Some(fn_id) = self.brain_fn.as_deref() {
            return self
                .run_external_brain(fn_id, session_id, cwd, prompt, cancel)
                .await;
        }
        self.run_echo_brain(session_id, prompt, cancel).await
    }

    async fn run_echo_brain(
        &self,
        session_id: &str,
        prompt: &[Value],
        cancel: &CancelHandle,
    ) -> BrainOutcome {
        let text = prompt_to_text(prompt);
        let chunks: Vec<&str> = text.split_inclusive(' ').collect();
        for chunk in chunks {
            if cancel.flag.load(Ordering::SeqCst) {
                return BrainOutcome {
                    stop_reason: "cancelled".to_string(),
                    terminal_confirmed: true,
                    pending: None,
                };
            }
            self.emit_update(
                session_id,
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": chunk }
                }),
            )
            .await;
        }
        BrainOutcome {
            stop_reason: "end_turn".to_string(),
            terminal_confirmed: true,
            pending: None,
        }
    }

    async fn run_external_brain(
        &self,
        fn_id: &str,
        session_id: &str,
        cwd: &str,
        prompt: &[Value],
        cancel: &CancelHandle,
    ) -> BrainOutcome {
        // Canonical iii brain shape: feed run::start_and_wait (or any
        // function with the same input contract) a User message built
        // from ACP prompt content blocks. The brain emits AgentEvent
        // frames into agent::events/<session_id>; our stream subscriber
        // (registered in AcpHandler::new) translates them to ACP
        // session/update notifications on stdout. The brain returns
        // synchronously with the final transcript when the turn ends.
        let payload = external_brain_payload(
            session_id,
            cwd,
            prompt,
            self.brain_model.as_deref(),
            self.brain_provider.as_deref(),
            self.brain_system_prompt.as_deref(),
        );

        let req = TriggerRequest {
            function_id: fn_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(BRAIN_TIMEOUT_MS + 5_000),
        };
        let iii = self.iii.clone();
        let mut brain = tokio::spawn(async move { iii.trigger(req).await });
        let ((res, error_terminal), cancellation_accepted) = tokio::select! {
            r = &mut brain => (flatten_brain_result(r), None),
            _ = cancel.wait() => {
                let mut stop_accepted = false;
                if let Some(stop_fn) = self.brain_stop_fn.as_deref() {
                    let stop = external_brain_stop_request(stop_fn, session_id);
                    match self.iii.trigger(stop).await {
                        Ok(result) => {
                            stop_accepted = stop_was_accepted(&result);
                            if !stop_accepted {
                                tracing::warn!(stop_fn, session_id, "external brain rejected stop request");
                            }
                        }
                        Err(error) => {
                            tracing::error!(%error, stop_fn, session_id, "external brain stop failed");
                        }
                    }
                }
                match tokio::time::timeout(
                    Duration::from_millis(BRAIN_CANCEL_GRACE_MS),
                    &mut brain,
                )
                .await
                {
                    Ok(result) => (flatten_brain_result(result), Some(stop_accepted)),
                    Err(_) => {
                        tracing::error!(
                            session_id,
                            stop_accepted,
                            "external brain did not settle after cancellation"
                        );
                        return BrainOutcome {
                            stop_reason: "refusal".to_string(),
                            terminal_confirmed: false,
                            pending: Some(brain),
                        };
                    }
                }
            },
        };
        match res {
            Ok(v) => BrainOutcome {
                stop_reason: external_brain_stop_reason(&v, cancellation_accepted).to_string(),
                terminal_confirmed: true,
                pending: None,
            },
            Err(e) => {
                tracing::error!(error = %e, fn_id, "external brain failed");
                BrainOutcome {
                    stop_reason: "refusal".to_string(),
                    terminal_confirmed: error_terminal,
                    pending: None,
                }
            }
        }
    }

    async fn finish_prompt_claim(&self, session_id: &str, claim_id: &str) -> PromptClaimFinish {
        let _ = self.update_session_record(session_id, |_| {}).await;
        let lock = self.history_lock(session_id);
        let _g = lock.lock().await;
        for attempt in 0..3 {
            match release_prompt_claim(&self.iii, session_id, &self.conn_id, claim_id).await {
                Ok(true) => return PromptClaimFinish::Definitive,
                Ok(false) => {
                    tracing::info!(
                        session_id,
                        claim_id,
                        "prompt claim was replaced or cleared; releasing its exact local guard"
                    );
                    return PromptClaimFinish::Definitive;
                }
                Err(error) if attempt < 2 => {
                    tracing::warn!(
                        %error,
                        session_id,
                        claim_id,
                        attempt,
                        "prompt claim release failed; retrying"
                    );
                    tokio::time::sleep(Duration::from_millis(100 * (attempt + 1))).await;
                }
                Err(error) => {
                    tracing::error!(
                        %error,
                        session_id,
                        claim_id,
                        "failed to release prompt claim; session remains recovery-required"
                    );
                    return PromptClaimFinish::Ambiguous;
                }
            }
        }
        PromptClaimFinish::Ambiguous
    }

    fn monitor_pending_brain(
        self: &Arc<Self>,
        session_id: String,
        claim_id: String,
        cancel: CancelHandle,
        brain: tokio::task::JoinHandle<Result<Value, IiiError>>,
    ) {
        let handler = Arc::clone(self);
        tokio::spawn(async move {
            match brain.await {
                Ok(Ok(_)) => {
                    if handler.finish_prompt_claim(&session_id, &claim_id).await
                        == PromptClaimFinish::Definitive
                    {
                        remove_local_cancel(&handler.cancels, &session_id, &cancel);
                    }
                }
                Ok(Err(error)) if brain_error_is_terminal(&error) => {
                    if handler.finish_prompt_claim(&session_id, &claim_id).await
                        == PromptClaimFinish::Definitive
                    {
                        remove_local_cancel(&handler.cancels, &session_id, &cancel);
                    }
                }
                Ok(Err(error)) => tracing::error!(
                    %error,
                    session_id,
                    "detached external brain ended ambiguously; prompt claim remains recovery-required"
                ),
                Err(error) => tracing::error!(
                    %error,
                    session_id,
                    "detached external brain task failed; prompt claim remains recovery-required"
                ),
            }
        });
    }

    async fn emit_update(&self, session_id: &str, update: Value) {
        let payload = json!({ "sessionId": session_id, "update": update });
        let appended = {
            let lock = self.history_lock(session_id);
            let _g = lock.lock().await;
            append_history_once(
                &self.iii,
                session_id,
                Some(&self.conn_id),
                None,
                vec![update.clone()],
            )
            .await
            .unwrap_or(false)
        };
        if !appended {
            return;
        }
        self.send_notification("session/update", payload).await;
    }

    async fn send_notification(&self, method: &str, params: Value) {
        write_notification(&self.outbound, &self.update_seq, method, params).await;
    }

    fn require_initialized(&self) -> Result<(), (i32, String)> {
        if self.initialized.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err((INTERNAL_ERROR, "not initialized".to_string()))
        }
    }
}

fn flatten_brain_result(
    result: Result<Result<Value, IiiError>, tokio::task::JoinError>,
) -> (Result<Value, String>, bool) {
    match result {
        Ok(Ok(value)) => (Ok(value), false),
        Ok(Err(error)) => {
            let terminal = brain_error_is_terminal(&error);
            (Err(error.to_string()), terminal)
        }
        Err(error) => (Err(format!("external brain task failed: {error}")), false),
    }
}

fn brain_error_is_terminal(error: &IiiError) -> bool {
    matches!(
        error,
        IiiError::Remote { .. }
            | IiiError::Handler(_)
            | IiiError::Serde(_)
            | IiiError::RegistrationRejected { .. }
    )
}

fn signal_local_cancel(handle: Option<&CancelHandle>) {
    if let Some(handle) = handle {
        handle.cancel();
    }
}

fn remove_local_cancel(
    cancels: &DashMap<String, CancelHandle>,
    session_id: &str,
    expected: &CancelHandle,
) -> bool {
    match cancels.entry(session_id.to_string()) {
        Entry::Occupied(entry) if entry.get().same_instance(expected) => {
            entry.remove();
            true
        }
        _ => false,
    }
}

fn record_owner_matches(record: &SessionRecord, conn_id: &str) -> bool {
    record.conn_id == conn_id
}

fn prompt_dispatch_matches(claim: &ActivePromptClaim, current: &PromptDispatchIdentity) -> bool {
    claim.dispatch.as_ref() == Some(current)
}

fn prompt_to_text(prompt: &[Value]) -> String {
    prompt
        .iter()
        .filter_map(|p| {
            let kind = p.get("type").and_then(|v| v.as_str())?;
            match kind {
                "text" => p.get("text").and_then(|v| v.as_str()).map(String::from),
                "resource" => p
                    .get("resource")
                    .and_then(|r| r.get("text"))
                    .and_then(|v| v.as_str())
                    .map(String::from),
                _ => None,
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// Stamp params._meta["iii.dev/seq"] (without clobbering caller-supplied
// _meta), wrap in a JSON-RPC notification frame, and write to stdout. One
// helper for both the in-handler send_notification and the stream-subscriber
// fan-in path so both flow through the same shape.
async fn write_notification(outbound: &Outbound, seq: &AtomicU64, method: &str, mut params: Value) {
    let s = seq.fetch_add(1, Ordering::SeqCst);
    if let Some(obj) = params.as_object_mut() {
        let meta = obj.entry("_meta").or_insert_with(|| json!({}));
        if let Some(meta_obj) = meta.as_object_mut() {
            meta_obj.insert("iii.dev/seq".to_string(), json!(s));
        }
    }
    let frame = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    });
    if let Err(e) = outbound.write(&frame).await {
        tracing::error!(error = %e, "outbound write failed");
    }
}

// Register a stream subscriber on the canonical `agent::events` stream.
// Both function ref and trigger handle are returned so AcpHandler can
// hold them; dropping either tears the registration down.
//
// The same stream is used by `turn-orchestrator`, every provider worker,
// `context-compaction`, etc. iii-acp filters frames by group_id (the
// session_id) against the per-process owned_sessions set so multiple
// iii-acp subprocesses don't fight over the same events.
fn register_event_subscriber(
    iii: &IIIClient,
    conn_id: &str,
    outbound: &Arc<Outbound>,
    update_seq: &Arc<AtomicU64>,
    owned_sessions: &Arc<DashSet<String>>,
    history_locks: &Arc<DashMap<String, Arc<tokio::sync::Mutex<()>>>>,
) -> (Option<iii_sdk::trigger::Trigger>, Option<FunctionRef>) {
    let fn_id = format!("acp::__on_event::{}", conn_id);

    let outbound_inner = outbound.clone();
    let seq_inner = update_seq.clone();
    let iii_inner = iii.clone();
    let conn_id_inner = conn_id.to_string();
    let owned_inner = owned_sessions.clone();
    let locks_inner = history_locks.clone();
    let function = iii.register_function(
        fn_id.clone(),
        RegisterFunction::new_async(move |payload: Value| {
            let outbound = outbound_inner.clone();
            let seq = seq_inner.clone();
            let iii = iii_inner.clone();
            let conn_id = conn_id_inner.clone();
            let owned = owned_inner.clone();
            let locks = locks_inner.clone();
            async move {
                forward_agent_event(&iii, &conn_id, &outbound, &seq, &owned, &locks, payload).await;
                Ok(json!({ "ok": true }))
            }
        })
        .description("ACP agent::events → stdout fan-in"),
    );

    let trigger = match iii.register_trigger(RegisterTriggerInput::new(
        "stream",
        fn_id,
        json!({ "stream_name": AGENT_EVENTS_STREAM }),
    )) {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::error!(error = %e, "failed to register acp event stream subscriber");
            None
        }
    };

    (trigger, Some(function))
}

// Stream-trigger envelope: `{ stream_name, group_id, item_id, data }`.
// `group_id` is the session_id; `data` is an `AgentEvent` JSON.
//
// Translates the iii AgentEvent shape to the ACP `session/update` shape
// and writes it to stdout. Frames for sessions we don't own (another
// connection's editor, or sessions closed mid-flight) are skipped.
async fn forward_agent_event(
    iii: &IIIClient,
    conn_id: &str,
    outbound: &Outbound,
    seq: &AtomicU64,
    owned: &DashSet<String>,
    history_locks: &DashMap<String, Arc<tokio::sync::Mutex<()>>>,
    payload: Value,
) {
    // Stream-trigger envelope (engine 0.11.x):
    //   { type:"stream", streamName, groupId, id, timestamp,
    //     event: { type: "create"|"update", data: <AgentEvent> } }
    // Older envelopes used snake_case (group_id / data at top level); we
    // accept both so this code keeps working if the engine envelope flips.
    let Some((sid, item_id, data)) = extract_event_payload(&payload) else {
        return;
    };
    if !owned.contains(&sid) {
        return;
    }
    let Some(updates) = translate_agent_event(&data) else {
        return;
    };
    let lock = history_locks
        .entry(sid.clone())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone();
    let cursor_item_id = item_id
        .as_deref()
        .filter(|item_id| item_id.starts_with("cursor-"));
    let appended = {
        let _g = lock.lock().await;
        match append_history_once(iii, &sid, Some(conn_id), cursor_item_id, updates.clone()).await {
            Ok(appended) => appended,
            Err(error) => {
                tracing::warn!(%error, sid, "append_history failed for agent event");
                return;
            }
        }
    };
    if !appended {
        return;
    }
    for update in updates {
        let params = json!({ "sessionId": sid, "update": update });
        write_notification(outbound, seq, "session/update", params).await;
    }
}

// Pull (group_id, AgentEvent) out of the engine's stream-trigger envelope.
// Accepts both nested camelCase (current) and flat snake_case (older).
fn extract_event_payload(payload: &Value) -> Option<(String, Option<String>, Value)> {
    let sid = payload
        .get("groupId")
        .or_else(|| payload.get("group_id"))
        .and_then(|v| v.as_str())?
        .to_string();
    let item_id = payload
        .get("id")
        .or_else(|| payload.get("item_id"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let data = payload
        .get("event")
        .and_then(|e| e.get("data"))
        .cloned()
        .or_else(|| payload.get("data").cloned())
        .unwrap_or(Value::Null);
    Some((sid, item_id, data))
}

// AgentEvent → ACP `session/update.update` payload(s). Returns None when
// the event doesn't map to a user-visible update (e.g., AgentStart,
// TurnStart — useful internally but no ACP equivalent).
fn translate_agent_event(event: &Value) -> Option<Vec<Value>> {
    let kind = event.get("type").and_then(|v| v.as_str())?;
    match kind {
        "message_update" => {
            let llm_event = event.get("llm_event")?;
            let llm_kind = llm_event.get("type").and_then(|v| v.as_str())?;
            let delta = llm_event
                .get("delta")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if delta.is_empty() {
                return None;
            }
            let session_update = match llm_kind {
                "text_delta" => "agent_message_chunk",
                "thinking_delta" => "agent_thought_chunk",
                _ => return None,
            };
            Some(vec![json!({
                "sessionUpdate": session_update,
                "content": { "type": "text", "text": delta },
            })])
        }
        // Batch/non-delta clients receive the fully-assembled assistant
        // message as a single message_complete event. Translate those to
        // one agent_message_chunk per text content block so Zed renders the
        // full reply.
        "message_complete" => {
            if event
                .get("body_streamed")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                return None;
            }
            let message = event.get("message")?;
            if message.get("role").and_then(|v| v.as_str()) != Some("assistant") {
                return None;
            }
            let content = message.get("content")?.as_array()?;
            let chunks: Vec<Value> = content
                .iter()
                .filter_map(|cb| {
                    let cb_type = cb.get("type").and_then(|v| v.as_str())?;
                    let session_update = match cb_type {
                        "text" => "agent_message_chunk",
                        "thinking" => "agent_thought_chunk",
                        _ => return None,
                    };
                    let text = cb.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    if text.is_empty() {
                        return None;
                    }
                    Some(json!({
                        "sessionUpdate": session_update,
                        "content": { "type": "text", "text": text },
                    }))
                })
                .collect();
            if chunks.is_empty() {
                None
            } else {
                Some(chunks)
            }
        }
        "tool_execution_start" | "function_execution_start" => {
            let id = event
                .get("tool_call_id")
                .or_else(|| event.get("function_call_id"))
                .and_then(|v| v.as_str())?;
            let name = event
                .get("tool_name")
                .or_else(|| event.get("function_id"))
                .and_then(|v| v.as_str())?;
            let args = event.get("args").cloned().unwrap_or(json!({}));
            Some(vec![json!({
                "sessionUpdate": "tool_call",
                "toolCallId": id,
                "title": format!("Running {}", name),
                "kind": tool_kind(name),
                "status": "in_progress",
                "rawInput": args,
            })])
        }
        "tool_execution_end" | "function_execution_end" => {
            let id = event
                .get("tool_call_id")
                .or_else(|| event.get("function_call_id"))
                .and_then(|v| v.as_str())?;
            let is_error = event
                .get("is_error")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let result = event.get("result").cloned().unwrap_or(Value::Null);
            Some(vec![json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": id,
                "status": if is_error { "failed" } else { "completed" },
                "rawOutput": result,
            })])
        }
        _ => None,
    }
}

// Map iii tool name → ACP ToolKind hint. Best-effort heuristics; default
// to `other` so unknown tools still surface.
fn tool_kind(name: &str) -> &'static str {
    let n = name.to_ascii_lowercase();
    match n.as_str() {
        s if s.contains("read") || s.contains("cat") || s.contains("grep") => "read",
        s if s.contains("write") || s.contains("edit") || s.contains("patch") => "edit",
        s if s.contains("delete") || s.contains("rm") => "delete",
        s if s.contains("move") || s.contains("rename") => "move",
        s if s.contains("search") || s.contains("find") => "search",
        s if s.contains("exec")
            || s.contains("bash")
            || s.contains("shell")
            || s.contains("run") =>
        {
            "execute"
        }
        s if s.contains("think") || s.contains("plan") => "think",
        s if s.contains("fetch") || s.contains("http") || s.contains("curl") => "fetch",
        _ => "other",
    }
}

// Build ACP-shape ContentBlock array from the ACP prompt content blocks.
// Currently passes text and resource-text through; non-text blocks
// (image, audio) become text placeholders so the brain still gets
// something — refine when iii content types catch up.
fn acp_prompt_to_content_blocks(prompt: &[Value]) -> Vec<Value> {
    prompt
        .iter()
        .filter_map(|p| {
            let kind = p.get("type").and_then(|v| v.as_str())?;
            match kind {
                "text" => p
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(|t| json!({ "type": "text", "text": t })),
                "resource" => {
                    let r = p.get("resource")?;
                    let text = r.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    let uri = r.get("uri").and_then(|v| v.as_str()).unwrap_or("");
                    Some(json!({
                        "type": "text",
                        "text": format!("<resource uri=\"{uri}\">\n{text}\n</resource>"),
                    }))
                }
                _ => None,
            }
        })
        .collect()
}

fn external_brain_payload(
    session_id: &str,
    cwd: &str,
    prompt: &[Value],
    model: Option<&str>,
    provider: Option<&str>,
    system_prompt: Option<&str>,
) -> Value {
    let user_msg = json!({
        "role": "user",
        "content": acp_prompt_to_content_blocks(prompt),
        "timestamp": now_ms(),
    });
    let mut payload = json!({
        "session_id": session_id,
        "cwd": cwd,
        "messages": [user_msg],
        "timeout_ms": BRAIN_TIMEOUT_MS,
    });
    if let Some(model) = model {
        payload["model"] = json!(model);
    }
    if let Some(provider) = provider {
        payload["provider"] = json!(provider);
    }
    if let Some(system_prompt) = system_prompt {
        payload["system_prompt"] = json!(system_prompt);
    }
    payload
}

fn external_brain_stop_request(function_id: &str, session_id: &str) -> TriggerRequest {
    TriggerRequest {
        function_id: function_id.to_string(),
        payload: json!({ "session_id": session_id }),
        action: None,
        timeout_ms: Some(5_000),
    }
}

fn stop_was_accepted(result: &Value) -> bool {
    result
        .get("stopped")
        .or_else(|| result.get("value").and_then(|value| value.get("stopped")))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn stop_reports_terminal(result: &Value) -> bool {
    let result = result.get("value").unwrap_or(result);
    if result
        .get("terminal")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    if let Some(status) = result.get("status").and_then(Value::as_str) {
        let status = status
            .strip_prefix("RUN_LIFECYCLE_STATUS_")
            .unwrap_or(status);
        if matches!(status, "FINISHED" | "CANCELLED" | "ERROR" | "EXPIRED") {
            return true;
        }
    }
    !result
        .get("stopped")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && result.get("reason").and_then(Value::as_str) == Some("no cancellable run id")
}

// Read run::start_and_wait return shape and pick a stop reason for ACP.
// Result envelope: { session_id, messages, turn_count }. The final
// assistant message in `messages` carries iii's `stop_reason`; map to
// ACP's vocabulary.
fn derive_stop_reason(result: &Value) -> Option<&'static str> {
    if let Some(status) = result.get("status").and_then(Value::as_str) {
        let status = status
            .strip_prefix("RUN_LIFECYCLE_STATUS_")
            .unwrap_or(status);
        match status {
            "MAX_TURN_REQUESTS" => return Some("max_turn_requests"),
            "MAX_TOKENS" => return Some("max_tokens"),
            "CANCELLED" => return Some("cancelled"),
            _ => {}
        }
    }
    let iii_reason = result
        .get("stop_reason")
        .and_then(|v| v.as_str())
        .or_else(|| {
            result
                .get("messages")?
                .as_array()?
                .iter()
                .rev()
                .find(|m| m.get("role").and_then(|v| v.as_str()) == Some("assistant"))?
                .get("stop_reason")?
                .as_str()
        })?;
    Some(match iii_reason {
        "end" => "end_turn",
        "length" => "max_tokens",
        "tool" => "end_turn",
        "aborted" => "cancelled",
        "error" => "refusal",
        _ => "end_turn",
    })
}

fn external_brain_stop_reason(result: &Value, cancellation_accepted: Option<bool>) -> &'static str {
    let reason = derive_stop_reason(result).unwrap_or("end_turn");
    if cancellation_accepted == Some(false) && reason == "cancelled" {
        "refusal"
    } else {
        reason
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancel_before_wait_keeps_a_stored_wakeup() {
        let cancel = CancelHandle::new();
        cancel.cancel();
        tokio::time::timeout(Duration::from_millis(50), cancel.wait())
            .await
            .expect("pre-wait cancellation must not be lost");
    }

    #[test]
    fn idle_or_transferred_session_has_no_external_cancel_target() {
        signal_local_cancel(None);

        let local = CancelHandle::new();
        signal_local_cancel(Some(&local));
        assert!(local.flag.load(Ordering::SeqCst));
    }

    #[test]
    fn stale_monitor_only_removes_its_exact_local_cancel_guard() {
        let cancels = DashMap::new();
        let stale = CancelHandle::new();
        let successor = CancelHandle::new();

        cancels.insert("session-one".to_string(), stale.clone());
        assert!(remove_local_cancel(&cancels, "session-one", &stale));

        cancels.insert("session-one".to_string(), successor.clone());
        assert!(!remove_local_cancel(&cancels, "session-one", &stale));
        assert!(
            cancels
                .get("session-one")
                .is_some_and(|current| current.same_instance(&successor))
        );
    }

    #[test]
    fn prompt_to_text_joins_text_blocks() {
        let p = vec![
            json!({"type": "text", "text": "hello"}),
            json!({"type": "text", "text": "world"}),
        ];
        assert_eq!(prompt_to_text(&p), "hello\nworld");
    }

    #[test]
    fn extracts_stable_stream_item_id() {
        let payload = json!({
            "groupId": "session-one",
            "id": "cursor-stable-item",
            "event": { "type": "update", "data": { "type": "message_update" } }
        });
        let (session_id, item_id, data) = extract_event_payload(&payload).unwrap();
        assert_eq!(session_id, "session-one");
        assert_eq!(item_id.as_deref(), Some("cursor-stable-item"));
        assert_eq!(data["type"], "message_update");
    }

    #[test]
    fn prompt_to_text_pulls_resource_text() {
        let p = vec![
            json!({"type": "text", "text": "look:"}),
            json!({
                "type": "resource",
                "resource": {"uri": "file:///x", "text": "contents"}
            }),
        ];
        assert_eq!(prompt_to_text(&p), "look:\ncontents");
    }

    #[test]
    fn prompt_to_text_skips_unknown_kinds() {
        let p = vec![
            json!({"type": "image"}),
            json!({"type": "text", "text": "ok"}),
        ];
        assert_eq!(prompt_to_text(&p), "ok");
    }

    #[test]
    fn translate_text_delta_to_agent_message_chunk() {
        let ev = json!({
            "type": "message_update",
            "llm_event": { "type": "text_delta", "delta": "hello" }
        });
        let updates = translate_agent_event(&ev).unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0]["sessionUpdate"], "agent_message_chunk");
        assert_eq!(updates[0]["content"]["text"], "hello");
    }

    #[test]
    fn translate_thinking_delta_to_thought_chunk() {
        let ev = json!({
            "type": "message_update",
            "llm_event": { "type": "thinking_delta", "delta": "musing..." }
        });
        let updates = translate_agent_event(&ev).unwrap();
        assert_eq!(updates[0]["sessionUpdate"], "agent_thought_chunk");
    }

    #[test]
    fn translate_tool_start_to_tool_call() {
        let ev = json!({
            "type": "tool_execution_start",
            "tool_call_id": "tc1",
            "tool_name": "shell::exec",
            "args": { "cmd": "ls" }
        });
        let updates = translate_agent_event(&ev).unwrap();
        assert_eq!(updates[0]["sessionUpdate"], "tool_call");
        assert_eq!(updates[0]["toolCallId"], "tc1");
        assert_eq!(updates[0]["kind"], "execute");
        assert_eq!(updates[0]["status"], "in_progress");
    }

    #[test]
    fn translate_function_start_to_tool_call() {
        let ev = json!({
            "type": "function_execution_start",
            "function_call_id": "fc1",
            "function_id": "cursor::tool::read",
            "args": { "path": "README.md" }
        });
        let updates = translate_agent_event(&ev).unwrap();
        assert_eq!(updates[0]["sessionUpdate"], "tool_call");
        assert_eq!(updates[0]["toolCallId"], "fc1");
        assert_eq!(updates[0]["kind"], "read");
    }

    #[test]
    fn translate_tool_end_marks_completed() {
        let ev = json!({
            "type": "tool_execution_end",
            "tool_call_id": "tc1",
            "tool_name": "shell::exec",
            "result": { "stdout": "ok" },
            "is_error": false
        });
        let updates = translate_agent_event(&ev).unwrap();
        assert_eq!(updates[0]["sessionUpdate"], "tool_call_update");
        assert_eq!(updates[0]["status"], "completed");
    }

    #[test]
    fn translate_tool_end_marks_failed_on_error() {
        let ev = json!({
            "type": "tool_execution_end",
            "tool_call_id": "tc1",
            "tool_name": "x",
            "result": null,
            "is_error": true
        });
        let updates = translate_agent_event(&ev).unwrap();
        assert_eq!(updates[0]["status"], "failed");
    }

    #[test]
    fn translate_unknown_event_drops_silently() {
        assert!(translate_agent_event(&json!({ "type": "not_a_real_event" })).is_none());
        assert!(translate_agent_event(&json!({ "type": "message_start" })).is_none());
    }

    #[test]
    fn translate_message_complete_assistant_emits_chunk() {
        let ev = json!({
            "type": "message_complete",
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": "hi there" }],
                "stop_reason": "end",
                "model": "x", "provider": "y", "timestamp": 0
            }
        });
        let updates = translate_agent_event(&ev).unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0]["sessionUpdate"], "agent_message_chunk");
        assert_eq!(updates[0]["content"]["text"], "hi there");
    }

    #[test]
    fn translate_streamed_message_complete_drops_duplicate_body() {
        let ev = json!({
            "type": "message_complete",
            "body_streamed": true,
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": "already streamed" }],
                "stop_reason": "end",
                "model": "x", "provider": "cursor", "timestamp": 0
            }
        });
        assert!(translate_agent_event(&ev).is_none());
    }

    #[test]
    fn translate_message_complete_user_dropped() {
        let ev = json!({
            "type": "message_complete",
            "message": {
                "role": "user",
                "content": [{ "type": "text", "text": "x" }],
                "timestamp": 0
            }
        });
        assert!(translate_agent_event(&ev).is_none());
    }

    #[test]
    fn derive_stop_reason_maps_iii_to_acp() {
        let result = json!({
            "messages": [
                { "role": "user", "content": [] },
                { "role": "assistant", "stop_reason": "end" }
            ]
        });
        assert_eq!(derive_stop_reason(&result), Some("end_turn"));

        let result = json!({
            "messages": [{ "role": "assistant", "stop_reason": "length" }]
        });
        assert_eq!(derive_stop_reason(&result), Some("max_tokens"));

        let result = json!({
            "messages": [{ "role": "assistant", "stop_reason": "aborted" }]
        });
        assert_eq!(derive_stop_reason(&result), Some("cancelled"));

        assert_eq!(
            derive_stop_reason(&json!({ "stop_reason": "error" })),
            Some("refusal")
        );

        assert_eq!(
            derive_stop_reason(&json!({
                "status": "RUN_LIFECYCLE_STATUS_MAX_TURN_REQUESTS",
                "stop_reason": "length"
            })),
            Some("max_turn_requests")
        );
    }

    #[test]
    fn stop_acknowledgement_requires_an_explicit_true_value() {
        assert!(stop_was_accepted(&json!({ "stopped": true })));
        assert!(stop_was_accepted(&json!({ "value": { "stopped": true } })));
        assert!(!stop_was_accepted(&json!({ "stopped": false })));
        assert!(!stop_was_accepted(&json!({ "reason": "not running" })));
    }

    #[test]
    fn stale_prompt_recovery_requires_terminal_or_no_active_confirmation() {
        assert!(stop_reports_terminal(&json!({ "terminal": true })));
        assert!(stop_reports_terminal(
            &json!({ "status": "RUN_LIFECYCLE_STATUS_CANCELLED" })
        ));
        assert!(stop_reports_terminal(
            &json!({ "stopped": false, "reason": "no cancellable run id" })
        ));
        assert!(!stop_reports_terminal(&json!({ "stopped": true })));
        assert!(!stop_reports_terminal(
            &json!({ "stopped": false, "reason": "active run changed" })
        ));
    }

    #[test]
    fn only_definitive_sdk_failures_are_terminal() {
        assert!(brain_error_is_terminal(&IiiError::Remote {
            code: "INVALID_PARAMS".to_string(),
            message: "bad model".to_string(),
            stacktrace: None,
        }));
        assert!(brain_error_is_terminal(&IiiError::Handler(
            "pre-dispatch validation".to_string()
        )));
        assert!(!brain_error_is_terminal(&IiiError::Timeout));
        assert!(!brain_error_is_terminal(&IiiError::NotConnected));
        assert!(!brain_error_is_terminal(&IiiError::WebSocket(
            "connection lost".to_string()
        )));
    }

    #[test]
    fn stale_owner_cannot_mutate_the_new_owners_expected_record() {
        let record = SessionRecord {
            session_id: "session-one".to_string(),
            conn_id: "new-owner".to_string(),
            cwd: "/workspace".to_string(),
            mcp_servers: Vec::new(),
            created_at_ms: 1,
            last_activity_ms: 1,
            mode: None,
            config_options: serde_json::Map::new(),
        };
        assert!(!record_owner_matches(&record, "old-owner"));
        assert!(record_owner_matches(&record, "new-owner"));
    }

    #[test]
    fn stale_recovery_requires_the_original_dispatch_identity() {
        let original = PromptDispatchIdentity {
            namespace: Some("team-a".to_string()),
            brain_function_id: Some("brain::run".to_string()),
            stop_function_id: Some("brain::stop".to_string()),
        };
        let claim = ActivePromptClaim {
            claim_id: "claim-one".to_string(),
            owner_conn_id: "owner".to_string(),
            started_at_ms: 1,
            dispatch: Some(original.clone()),
        };
        assert!(prompt_dispatch_matches(&claim, &original));
        assert!(!prompt_dispatch_matches(
            &claim,
            &PromptDispatchIdentity {
                brain_function_id: Some("other::run".to_string()),
                ..original.clone()
            }
        ));
        assert!(!prompt_dispatch_matches(
            &claim,
            &PromptDispatchIdentity {
                stop_function_id: Some("cursor::stop".to_string()),
                ..original.clone()
            }
        ));
        assert!(!prompt_dispatch_matches(
            &claim,
            &PromptDispatchIdentity {
                namespace: Some("team-b".to_string()),
                ..original.clone()
            }
        ));
        assert!(!prompt_dispatch_matches(
            &ActivePromptClaim {
                dispatch: None,
                ..claim
            },
            &original
        ));
    }

    #[test]
    fn rejected_stop_cannot_report_cancelled() {
        let cancelled = json!({ "stop_reason": "aborted" });
        assert_eq!(
            external_brain_stop_reason(&cancelled, Some(false)),
            "refusal"
        );
        assert_eq!(
            external_brain_stop_reason(&cancelled, Some(true)),
            "cancelled"
        );
    }

    #[test]
    fn acp_prompt_to_content_blocks_passes_text() {
        let p = vec![json!({"type": "text", "text": "hello"})];
        let cb = acp_prompt_to_content_blocks(&p);
        assert_eq!(cb.len(), 1);
        assert_eq!(cb[0]["type"], "text");
        assert_eq!(cb[0]["text"], "hello");
    }

    #[test]
    fn acp_prompt_to_content_blocks_inlines_resource() {
        let p = vec![json!({
            "type": "resource",
            "resource": { "uri": "file:///x", "text": "contents" }
        })];
        let cb = acp_prompt_to_content_blocks(&p);
        let s = cb[0]["text"].as_str().unwrap();
        assert!(s.contains("file:///x"));
        assert!(s.contains("contents"));
    }

    #[test]
    fn external_brain_payload_includes_editor_cwd_and_stop_mapping() {
        let payload = external_brain_payload(
            "session-one",
            "/workspace",
            &[json!({ "type": "text", "text": "hello" })],
            Some("composer-2"),
            Some("cursor"),
            None,
        );
        assert_eq!(payload["cwd"], "/workspace");
        assert_eq!(payload["model"], "composer-2");
        assert_eq!(payload["messages"][0]["content"][0]["text"], "hello");

        let stop = external_brain_stop_request("cursor::stop", "session-one");
        assert_eq!(stop.function_id, "cursor::stop");
        assert_eq!(stop.payload, json!({ "session_id": "session-one" }));
    }
}
