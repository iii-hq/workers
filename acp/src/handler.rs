use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use dashmap::DashMap;
use iii_sdk::{III, RegisterFunctionMessage, RegisterTriggerInput};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::session::{
    self, SessionRecord, append_history, append_session_to_index, connection_updates_topic,
    durable_publish, now_ms, read_history, read_session_index, remove_session_from_index, scope,
    session_history_key, session_key, state_delete, state_get, state_set,
};
use crate::transport::Outbound;
use crate::types::{
    ACP_PROTOCOL_VERSION, INTERNAL_ERROR, INVALID_PARAMS, JsonRpcResponse, METHOD_NOT_FOUND,
    SessionCancelParams, SessionLoadParams, SessionNewParams, SessionPromptParams, parse,
};

pub struct AcpHandler {
    iii: III,
    conn_id: String,
    initialized: AtomicBool,
    cancels: DashMap<String, Arc<AtomicBool>>,
    update_seq: Arc<AtomicU64>,
    outbound: Arc<Outbound>,
    brain_fn: Option<String>,
    // Trigger guard: dropping it deregisters the durable subscriber on the
    // engine. Held for the lifetime of the handler so brain updates keep
    // flowing.
    _update_subscriber: Option<iii_sdk::Trigger>,
}

impl AcpHandler {
    pub fn new(iii: III, outbound: Arc<Outbound>, brain_fn: Option<String>) -> Self {
        let conn_id = Uuid::new_v4().to_string();
        let update_seq = Arc::new(AtomicU64::new(0));
        tracing::info!(%conn_id, "acp handler initialized");

        // Wire the brain update fan-in only when an external brain is
        // configured. The echo brain writes session/update notifications
        // directly via send_notification, so it doesn't need the round trip
        // through durable publish.
        let update_subscriber = brain_fn
            .as_deref()
            .and_then(|_| register_update_subscriber(&iii, &conn_id, &outbound, &update_seq));

        Self {
            iii,
            conn_id,
            initialized: AtomicBool::new(false),
            cancels: DashMap::new(),
            update_seq,
            outbound,
            brain_fn,
            _update_subscriber: update_subscriber,
        }
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
        let key = session_key(&self.conn_id, &session_id);
        let value = serde_json::to_value(&record).map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
        state_set(&self.iii, &scope(), &key, value)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
        append_session_to_index(&self.iii, &self.conn_id, &session_id)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
        Ok(json!({ "sessionId": session_id }))
    }

    async fn session_load(&self, params: Option<Value>) -> Result<Value, (i32, String)> {
        self.require_initialized()?;
        let p: SessionLoadParams = parse(params).map_err(|e| (INVALID_PARAMS, e))?;
        let key = session_key(&self.conn_id, &p.session_id);
        let _record = state_get(&self.iii, &scope(), &key)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?
            .ok_or_else(|| {
                (
                    INVALID_PARAMS,
                    format!("session not found: {}", p.session_id),
                )
            })?;
        let history = read_history(&self.iii, &self.conn_id, &p.session_id)
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
        let ids = read_session_index(&self.iii, &self.conn_id)
            .await
            .map_err(|e| (INTERNAL_ERROR, e.to_string()))?;
        let mut sessions = Vec::with_capacity(ids.len());
        for id in ids {
            let key = session_key(&self.conn_id, &id);
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
        let key = session_key(&self.conn_id, &p.session_id);
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

        let abort = Arc::new(AtomicBool::new(false));
        self.cancels.insert(p.session_id.clone(), abort.clone());

        append_history(
            &self.iii,
            &self.conn_id,
            &p.session_id,
            json!({
                "sessionUpdate": "user_message_chunk",
                "content": { "type": "text", "text": prompt_to_text(&p.prompt) }
            }),
        )
        .await
        .map_err(|e| (INTERNAL_ERROR, e.to_string()))?;

        let stop_reason = self
            .run_brain(&p.session_id, &p.prompt, abort.clone())
            .await;

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
        if let Some(flag) = self.cancels.get(&p.session_id) {
            flag.store(true, Ordering::SeqCst);
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

        if let Err(e) = state_delete(
            &self.iii,
            &scope,
            &session_key(&self.conn_id, &p.session_id),
        )
        .await
        {
            errs.push(format!("session record: {}", e));
        }
        if let Err(e) = state_delete(
            &self.iii,
            &scope,
            &session_history_key(&self.conn_id, &p.session_id),
        )
        .await
        {
            errs.push(format!("history: {}", e));
        }
        if let Err(e) = remove_session_from_index(&self.iii, &self.conn_id, &p.session_id).await {
            errs.push(format!("index: {}", e));
        }

        // In-flight cancel slot for this session is a process-local concern,
        // unrelated to engine state. Drop it.
        self.cancels.remove(&p.session_id);

        if errs.is_empty() {
            Ok(())
        } else {
            Err((
                INTERNAL_ERROR,
                format!("session_close partial failure: {}", errs.join("; ")),
            ))
        }
    }

    async fn run_brain(
        &self,
        session_id: &str,
        prompt: &[Value],
        abort: Arc<AtomicBool>,
    ) -> String {
        if let Some(fn_id) = self.brain_fn.as_deref() {
            return self
                .run_external_brain(fn_id, session_id, prompt, abort)
                .await;
        }
        self.run_echo_brain(session_id, prompt, abort).await
    }

    async fn run_echo_brain(
        &self,
        session_id: &str,
        prompt: &[Value],
        abort: Arc<AtomicBool>,
    ) -> String {
        let text = prompt_to_text(prompt);
        let chunks: Vec<&str> = text.split_inclusive(' ').collect();
        for chunk in chunks {
            if abort.load(Ordering::SeqCst) {
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
        let echo = json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "" }
        });
        let _ = append_history(&self.iii, &self.conn_id, session_id, echo).await;
        "end_turn".to_string()
    }

    async fn run_external_brain(
        &self,
        fn_id: &str,
        session_id: &str,
        prompt: &[Value],
        abort: Arc<AtomicBool>,
    ) -> String {
        // The brain publishes `session/update` payloads to this topic via
        // `iii::durable::publish`. Our durable subscriber (registered in
        // AcpHandler::new) decodes them and writes to stdout. Brain returns
        // `{ stopReason }` synchronously when the turn is over.
        let respond_topic = connection_updates_topic(&self.conn_id);
        let payload = json!({
            "sessionId": session_id,
            "connId": self.conn_id,
            "prompt": prompt,
            "respondTopic": respond_topic,
        });
        let req = iii_sdk::TriggerRequest {
            function_id: fn_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(120_000),
        };
        let res = tokio::select! {
            r = self.iii.trigger(req) => r,
            _ = wait_aborted(abort.clone()) => return "cancelled".to_string(),
        };
        match res {
            Ok(v) => v
                .get("stopReason")
                .and_then(|s| s.as_str())
                .unwrap_or("end_turn")
                .to_string(),
            Err(e) => {
                tracing::error!(error = %e, fn_id, "external brain failed");
                "refusal".to_string()
            }
        }
    }

    async fn emit_update(&self, session_id: &str, update: Value) {
        let payload = json!({ "sessionId": session_id, "update": update });
        let _ = append_history(&self.iii, &self.conn_id, session_id, update.clone()).await;
        self.send_notification("session/update", payload).await;
    }

    async fn send_notification(&self, method: &str, mut params: Value) {
        let seq = self.update_seq.fetch_add(1, Ordering::SeqCst);
        // Stamp seq into params._meta (per ACP extensibility rules) without
        // overwriting any existing _meta keys callers already set.
        if let Some(obj) = params.as_object_mut() {
            let meta = obj.entry("_meta").or_insert_with(|| json!({}));
            if let Some(meta_obj) = meta.as_object_mut() {
                meta_obj.insert("iii.dev/seq".to_string(), json!(seq));
            }
        }
        let frame = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        if let Err(e) = self.outbound.write(&frame).await {
            tracing::error!(error = %e, "outbound write failed");
        }
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

async fn wait_aborted(flag: Arc<AtomicBool>) {
    loop {
        if flag.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

// Register a durable subscriber on the connection's updates topic. The
// returned Trigger handle MUST be kept alive for the lifetime of the
// handler — dropping it tears the subscription down on the engine.
fn register_update_subscriber(
    iii: &III,
    conn_id: &str,
    outbound: &Arc<Outbound>,
    update_seq: &Arc<AtomicU64>,
) -> Option<iii_sdk::Trigger> {
    let fn_id = format!("acp::__on_update::{}", conn_id);
    let topic = connection_updates_topic(conn_id);

    let outbound_inner = outbound.clone();
    let seq_inner = update_seq.clone();
    iii.register_function_with(
        RegisterFunctionMessage {
            id: fn_id.clone(),
            description: Some("ACP brain → stdout fan-in".into()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "sessionId": { "type": "string" },
                    "update": {}
                },
                "required": ["sessionId", "update"]
            })),
            response_format: None,
            metadata: None,
            invocation: None,
        },
        move |input: Value| {
            let outbound = outbound_inner.clone();
            let seq = seq_inner.clone();
            async move {
                forward_brain_update(&outbound, &seq, input).await;
                Ok(json!({}))
            }
        },
    );

    match iii.register_trigger(RegisterTriggerInput {
        trigger_type: "durable::subscriber".to_string(),
        function_id: fn_id,
        config: json!({ "topic": topic }),
        metadata: None,
    }) {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::error!(error = %e, "failed to register acp update subscriber");
            None
        }
    }
}

// Subscriber payload may arrive bare ({sessionId, update}) or wrapped under
// a `data` envelope depending on engine version. Accept either.
async fn forward_brain_update(outbound: &Outbound, seq: &AtomicU64, input: Value) {
    let body = input.get("data").cloned().unwrap_or(input);
    let session_id = body.get("sessionId").and_then(|v| v.as_str());
    let update = body.get("update").cloned();
    let (Some(sid), Some(up)) = (session_id, update) else {
        tracing::warn!(?body, "brain update missing sessionId/update");
        return;
    };
    let mut params = json!({ "sessionId": sid, "update": up });
    let s = seq.fetch_add(1, Ordering::SeqCst);
    if let Some(obj) = params.as_object_mut() {
        let meta = obj.entry("_meta").or_insert_with(|| json!({}));
        if let Some(meta_obj) = meta.as_object_mut() {
            meta_obj.insert("iii.dev/seq".to_string(), json!(s));
        }
    }
    let frame = json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": params,
    });
    if let Err(e) = outbound.write(&frame).await {
        tracing::error!(error = %e, "outbound write failed (brain update)");
    }
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
}
