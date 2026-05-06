use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use dashmap::{DashMap, DashSet};
use iii_sdk::{FunctionRef, III, RegisterFunctionMessage, RegisterTriggerInput};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::session::{
    self, AGENT_EVENTS_STREAM, SessionRecord, append_history, append_session_to_index,
    durable_publish, now_ms, read_history, read_session_index, remove_session_from_index, scope,
    session_history_key, session_key, state_delete, state_get, state_set,
};
use crate::transport::Outbound;
use crate::types::{
    ACP_PROTOCOL_VERSION, INTERNAL_ERROR, INVALID_PARAMS, JsonRpcResponse, METHOD_NOT_FOUND,
    SessionCancelParams, SessionLoadParams, SessionNewParams, SessionPromptParams, parse,
};

// Canonical iii brain function. Any worker exposing this id with the
// turn-orchestrator wire shape (session_id, messages, model, ...) can
// drive iii-acp without an adapter.
pub const DEFAULT_BRAIN_FN: &str = "run::start_and_wait";
const BRAIN_TIMEOUT_MS: u64 = 600_000;

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
        self.notify.notify_waiters();
    }

    async fn wait(&self) {
        if self.flag.load(Ordering::SeqCst) {
            return;
        }
        self.notify.notified().await;
    }
}

pub struct AcpHandler {
    iii: III,
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
    _event_subscriber: Option<iii_sdk::Trigger>,
    _event_function: Option<FunctionRef>,
    // True iff agent::events stream subscriber registered cleanly. When an
    // external brain is configured but this is false, session/prompt fails
    // fast with an actionable error rather than running the brain whose
    // updates would silently never reach stdout.
    event_subscriber_healthy: bool,
}

pub struct BrainConfig {
    pub function_id: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub system_prompt: Option<String>,
}

impl AcpHandler {
    pub fn new(iii: III, outbound: Arc<Outbound>, brain: BrainConfig) -> Self {
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
            "session/list" => self.session_list().await,
            "session/prompt" => self.session_prompt(params).await,
            "session/cancel" => self.session_cancel(params).await.map(|_| Value::Null),
            "session/close" => self.session_close(params).await.map(|_| Value::Null),
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
                "sessionCapabilities": {}
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
        };
        let key = session_key(&session_id);
        let value = serde_json::to_value(&record).map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
        state_set(&self.iii, &scope(), &key, value)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
        append_session_to_index(&self.iii, &session_id)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
        self.owned_sessions.insert(session_id.clone());
        Ok(json!({ "sessionId": session_id }))
    }

    async fn session_load(&self, params: Option<Value>) -> Result<Value, (i32, String)> {
        self.require_initialized()?;
        let p: SessionLoadParams = parse(params).map_err(|e| (INVALID_PARAMS, e))?;
        let key = session_key(&p.session_id);
        let _record = state_get(&self.iii, &scope(), &key)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?
            .ok_or_else(|| {
                (
                    INVALID_PARAMS,
                    format!("session not found: {}", p.session_id),
                )
            })?;
        // session/load on a session this subprocess didn't create still
        // gets ownership transferred so agent::events route here from now on.
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

    async fn session_prompt(&self, params: Option<Value>) -> Result<Value, (i32, String)> {
        self.require_initialized()?;
        let p: SessionPromptParams = parse(params).map_err(|e| (INVALID_PARAMS, e))?;
        let key = session_key(&p.session_id);
        if state_get(&self.iii, &scope(), &key)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?
            .is_none()
        {
            return Err((
                INVALID_PARAMS,
                format!("session not found: {}", p.session_id),
            ));
        }
        // Defensive: if this is the first time we see the session in this
        // subprocess (e.g., an external brain prompted it via run::start
        // without going through ACP session/new), claim it now so
        // agent::events for it forward to our stdout.
        self.owned_sessions.insert(p.session_id.clone());

        if self.brain_fn.is_some() && !self.event_subscriber_healthy {
            return Err((
                INTERNAL_ERROR,
                "iii-acp: agent::events stream subscriber failed to register at startup; \
                 external brain updates would not reach the editor. Check engine logs and \
                 ensure `iii-stream` worker is active before retrying."
                    .to_string(),
            ));
        }

        let cancel = CancelHandle::new();
        self.cancels.insert(p.session_id.clone(), cancel.clone());

        {
            let lock = self.history_lock(&p.session_id);
            let _g = lock.lock().await;
            append_history(
                &self.iii,
                &p.session_id,
                json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": { "type": "text", "text": prompt_to_text(&p.prompt) }
                }),
            )
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
        }

        let stop_reason = self.run_brain(&p.session_id, &p.prompt, &cancel).await;

        self.cancels.remove(&p.session_id);

        let mut record_payload = json!({});
        if let Some(rec) = state_get(&self.iii, &scope(), &key)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?
        {
            if let Ok(mut r) = serde_json::from_value::<SessionRecord>(rec) {
                r.last_activity_ms = now_ms();
                record_payload = serde_json::to_value(&r).unwrap_or(Value::Null);
                let _ = state_set(&self.iii, &scope(), &key, record_payload.clone()).await;
            }
        }

        Ok(json!({ "stopReason": stop_reason }))
    }

    async fn session_cancel(&self, params: Option<Value>) -> Result<(), (i32, String)> {
        let p: SessionCancelParams = parse(params).map_err(|e| (INVALID_PARAMS, e))?;
        if let Some(handle) = self.cancels.get(&p.session_id) {
            handle.cancel();
        }
        let _ = durable_publish(
            &self.iii,
            &session::cancel_topic(&self.conn_id, &p.session_id),
            json!({ "reason": "client" }),
        )
        .await;
        Ok(())
    }

    async fn session_close(&self, params: Option<Value>) -> Result<(), (i32, String)> {
        let p: SessionLoadParams = parse(params).map_err(|e| (INVALID_PARAMS, e))?;
        let scope = scope();
        let mut errs: Vec<String> = Vec::new();

        if let Err(e) = state_delete(&self.iii, &scope, &session_key(&p.session_id)).await {
            errs.push(format!("session record: {}", e));
        }
        if let Err(e) = state_delete(&self.iii, &scope, &session_history_key(&p.session_id)).await {
            errs.push(format!("history: {}", e));
        }
        if let Err(e) = remove_session_from_index(&self.iii, &p.session_id).await {
            errs.push(format!("index: {}", e));
        }

        // In-flight cancel slot for this session is a process-local concern,
        // unrelated to engine state. Drop it. Same for session ownership —
        // any further agent::events for this session are foreign now.
        self.cancels.remove(&p.session_id);
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

    async fn run_brain(&self, session_id: &str, prompt: &[Value], cancel: &CancelHandle) -> String {
        if let Some(fn_id) = self.brain_fn.as_deref() {
            return self
                .run_external_brain(fn_id, session_id, prompt, cancel)
                .await;
        }
        self.run_echo_brain(session_id, prompt, cancel).await
    }

    async fn run_echo_brain(
        &self,
        session_id: &str,
        prompt: &[Value],
        cancel: &CancelHandle,
    ) -> String {
        let text = prompt_to_text(prompt);
        let chunks: Vec<&str> = text.split_inclusive(' ').collect();
        for chunk in chunks {
            if cancel.flag.load(Ordering::SeqCst) {
                return "cancelled".to_string();
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
        "end_turn".to_string()
    }

    async fn run_external_brain(
        &self,
        fn_id: &str,
        session_id: &str,
        prompt: &[Value],
        cancel: &CancelHandle,
    ) -> String {
        // Canonical iii brain shape: feed run::start_and_wait (or any
        // function with the same input contract) a User message built
        // from ACP prompt content blocks. The brain emits AgentEvent
        // frames into agent::events/<session_id>; our stream subscriber
        // (registered in AcpHandler::new) translates them to ACP
        // session/update notifications on stdout. The brain returns
        // synchronously with the final transcript when the turn ends.
        let user_msg = json!({
            "role": "user",
            "content": acp_prompt_to_content_blocks(prompt),
            "timestamp": now_ms(),
        });

        let mut payload = json!({
            "session_id": session_id,
            "messages": [user_msg],
            "timeout_ms": BRAIN_TIMEOUT_MS,
        });
        if let Some(model) = self.brain_model.as_deref() {
            payload["model"] = json!(model);
        }
        if let Some(provider) = self.brain_provider.as_deref() {
            payload["provider"] = json!(provider);
        }
        if let Some(sys) = self.brain_system_prompt.as_deref() {
            payload["system_prompt"] = json!(sys);
        }

        let req = iii_sdk::TriggerRequest {
            function_id: fn_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(BRAIN_TIMEOUT_MS + 5_000),
        };
        let res = tokio::select! {
            r = self.iii.trigger(req) => r,
            _ = cancel.wait() => return "cancelled".to_string(),
        };
        match res {
            Ok(v) => derive_stop_reason(&v).unwrap_or("end_turn").to_string(),
            Err(e) => {
                tracing::error!(error = %e, fn_id, "external brain failed");
                "refusal".to_string()
            }
        }
    }

    async fn emit_update(&self, session_id: &str, update: Value) {
        let payload = json!({ "sessionId": session_id, "update": update });
        {
            let lock = self.history_lock(session_id);
            let _g = lock.lock().await;
            let _ = append_history(&self.iii, session_id, update.clone()).await;
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
    iii: &III,
    conn_id: &str,
    outbound: &Arc<Outbound>,
    update_seq: &Arc<AtomicU64>,
    owned_sessions: &Arc<DashSet<String>>,
    history_locks: &Arc<DashMap<String, Arc<tokio::sync::Mutex<()>>>>,
) -> (Option<iii_sdk::Trigger>, Option<FunctionRef>) {
    let fn_id = format!("acp::__on_event::{}", conn_id);

    let outbound_inner = outbound.clone();
    let seq_inner = update_seq.clone();
    let iii_inner = iii.clone();
    let owned_inner = owned_sessions.clone();
    let locks_inner = history_locks.clone();
    let function = iii.register_function((
        RegisterFunctionMessage::with_id(fn_id.clone())
            .with_description("ACP agent::events → stdout fan-in".into()),
        move |payload: Value| {
            let outbound = outbound_inner.clone();
            let seq = seq_inner.clone();
            let iii = iii_inner.clone();
            let owned = owned_inner.clone();
            let locks = locks_inner.clone();
            async move {
                forward_agent_event(&iii, &outbound, &seq, &owned, &locks, payload).await;
                Ok(json!({ "ok": true }))
            }
        },
    ));

    let trigger = match iii.register_trigger(RegisterTriggerInput {
        trigger_type: "stream".into(),
        function_id: fn_id,
        config: json!({ "stream_name": AGENT_EVENTS_STREAM }),
        metadata: None,
    }) {
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
    iii: &III,
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
    let Some((sid, data)) = extract_event_payload(&payload) else {
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
    for update in updates {
        {
            let _g = lock.lock().await;
            if let Err(e) = append_history(iii, &sid, update.clone()).await {
                tracing::warn!(error = %e, sid, "append_history failed for agent event");
            }
        }
        let params = json!({ "sessionId": sid, "update": update });
        write_notification(outbound, seq, "session/update", params).await;
    }
}

// Pull (group_id, AgentEvent) out of the engine's stream-trigger envelope.
// Accepts both nested camelCase (current) and flat snake_case (older).
fn extract_event_payload(payload: &Value) -> Option<(String, Value)> {
    let sid = payload
        .get("groupId")
        .or_else(|| payload.get("group_id"))
        .and_then(|v| v.as_str())?
        .to_string();
    let data = payload
        .get("event")
        .and_then(|e| e.get("data"))
        .cloned()
        .or_else(|| payload.get("data").cloned())
        .unwrap_or(Value::Null);
    Some((sid, data))
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
        // turn-orchestrator (current head) does not emit message_update
        // text deltas — provider-router consumes the streaming response
        // internally and returns the fully-assembled assistant message,
        // surfaced as a single message_end event. Translate those to one
        // agent_message_chunk per text content block so Zed renders the
        // full reply. message_start and tool-result message_end variants
        // are dropped to avoid duplication.
        "message_end" => {
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
        "tool_execution_start" => {
            let id = event.get("tool_call_id").and_then(|v| v.as_str())?;
            let name = event.get("tool_name").and_then(|v| v.as_str())?;
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
        "tool_execution_end" => {
            let id = event.get("tool_call_id").and_then(|v| v.as_str())?;
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

// Read run::start_and_wait return shape and pick a stop reason for ACP.
// Result envelope: { session_id, messages, turn_count }. The final
// assistant message in `messages` carries iii's `stop_reason`; map to
// ACP's vocabulary.
fn derive_stop_reason(result: &Value) -> Option<&'static str> {
    let messages = result.get("messages")?.as_array()?;
    let last_assistant = messages
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(|v| v.as_str()) == Some("assistant"))?;
    let iii_reason = last_assistant.get("stop_reason").and_then(|v| v.as_str())?;
    Some(match iii_reason {
        "end" => "end_turn",
        "length" => "max_tokens",
        "tool" => "end_turn",
        "aborted" => "cancelled",
        "error" => "refusal",
        _ => "end_turn",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_to_text_joins_text_blocks() {
        let p = vec![
            json!({"type": "text", "text": "hello"}),
            json!({"type": "text", "text": "world"}),
        ];
        assert_eq!(prompt_to_text(&p), "hello\nworld");
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
        assert!(translate_agent_event(&json!({ "type": "agent_start" })).is_none());
        assert!(translate_agent_event(&json!({ "type": "turn_start" })).is_none());
    }

    #[test]
    fn translate_message_end_assistant_emits_chunk() {
        let ev = json!({
            "type": "message_end",
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
    fn translate_message_end_user_dropped() {
        let ev = json!({
            "type": "message_end",
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
}
