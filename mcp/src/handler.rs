use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use dashmap::DashMap;
use iii_sdk::{
    FunctionInfo, FunctionsAvailableGuard, III, RegisterFunctionMessage, RegisterTriggerInput,
    Trigger, TriggerAction, TriggerRequest, Value,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::prompts;
use crate::spec::{
    self, LOG_INFO, PAGE_SIZE, ToolAnnotations, level_from_str, log_message_notification,
    make_tool_annotations, paginate, progress_notification, resolve_templated_uri,
    resource_updated_notification,
};
use crate::worker_manager::{WorkerCreateParams, WorkerManager, WorkerStopParams};

// Current published MCP spec revision. Bump when moving to a newer one.
// Real spec versions are date-stamps from
// https://spec.modelcontextprotocol.io — not arbitrary dates. The earlier
// "2025-11-25" value in this file was a future date that no real client
// recognized; MCP Inspector rejected it with "protocol version is not
// supported" until this was corrected.
const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const INTERNAL_ERROR: i32 = -32603;
const INVALID_PARAMS: i32 = -32602;
const METHOD_NOT_FOUND: i32 = -32601;

fn has_metadata_flag(f: &FunctionInfo, key: &str) -> bool {
    f.metadata
        .as_ref()
        .and_then(|m| m.get(key))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn metadata_string(f: &FunctionInfo, key: &str) -> Option<String> {
    f.metadata
        .as_ref()
        .and_then(|m| m.get(key))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

// Prefixes that are NEVER surfaced as MCP tools, even under --expose-all.
// These are iii-engine internals — surfacing `state::set` or `engine::*` as
// an MCP tool lets an agent poke at engine plumbing, which is categorically
// not an agent-facing surface. Matches the same carve-out the `agent`
// worker enforces via DEFAULT_EXCLUDED_PREFIXES.
pub const ALWAYS_HIDDEN_PREFIXES: &[&str] = &[
    "engine::", "state::", "stream::",
    // SDK internals come in two notations — callback-style
    // `iii.on_functions_available.<uuid>` and namespace-style
    // `iii::durable::publish`, `iii::config`. Match both.
    "iii.", "iii::",
    // Protocol-worker entry points are stateless RPC dispatchers. Routing
    // an MCP tools/call through `mcp::handler` recurses into ourselves;
    // through `a2a::jsonrpc` double-envelopes an A2A request inside MCP.
    // Neither is a useful tool surface. Hide both even under --expose-all.
    "mcp::", "a2a::",
];

fn is_always_hidden(function_id: &str) -> bool {
    ALWAYS_HIDDEN_PREFIXES
        .iter()
        .any(|p| function_id.starts_with(p))
}

#[derive(Debug, Clone)]
pub struct ExposureConfig {
    /// Ignore the `mcp.expose` metadata gate (dev only).
    pub expose_all: bool,
    /// Skip the 6 built-in management tools in tools/list. Default true for
    /// HTTP transport where worker-management isn't applicable anyway.
    pub no_builtins: bool,
    /// Optional tier filter. When set, only functions whose `mcp.tier`
    /// metadata string equals this value survive the filter.
    pub tier: Option<String>,
}

impl ExposureConfig {
    pub fn new(expose_all: bool, no_builtins: bool, tier: Option<String>) -> Self {
        Self {
            expose_all,
            no_builtins,
            tier,
        }
    }
}

fn is_function_exposed(f: &FunctionInfo, cfg: &ExposureConfig) -> bool {
    // Hard floor: infra prefixes never surface, even with --expose-all.
    if is_always_hidden(&f.function_id) {
        return false;
    }
    if !cfg.expose_all && !has_metadata_flag(f, "mcp.expose") {
        return false;
    }
    if let Some(want_tier) = &cfg.tier {
        match metadata_string(f, "mcp.tier") {
            Some(got) if &got == want_tier => {}
            _ => return false,
        }
    }
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

impl JsonRpcResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

// Per-session MCP spec state (subscriptions, log level, progress tokens,
// in-flight cancellation senders). Lives in an Arc so the HTTP `dispatch_http`
// path can share an instance keyed by session and the stdio handler can hold
// a single instance for its single attached client.
pub struct SessionState {
    pub subscriptions: std::sync::Mutex<HashSet<String>>,
    pub log_level: AtomicU8,
    // request_id (from tools/call) → progress token. Unused for routing today
    // (we simply emit notifications/progress with the token the user provided
    // via `mcp::progress`), but tracked so cancellation/progress can correlate
    // when richer routing is needed.
    pub progress_tokens: DashMap<Value, Value>,
    // request_id → cancel sender. When `notifications/cancelled` arrives,
    // we fire the matching sender to abort an in-flight tools/call.
    pub cancellation_tokens: DashMap<String, oneshot::Sender<()>>,
    pub notifier: mpsc::Sender<String>,
}

impl SessionState {
    pub fn new(notifier: mpsc::Sender<String>) -> Self {
        Self {
            subscriptions: std::sync::Mutex::new(HashSet::new()),
            log_level: AtomicU8::new(LOG_INFO),
            progress_tokens: DashMap::new(),
            cancellation_tokens: DashMap::new(),
            notifier,
        }
    }

    pub fn try_send(&self, msg: String) {
        if self.notifier.try_send(msg).is_err() {
            tracing::warn!("Notification channel full; dropping message");
        }
    }

    pub fn is_subscribed(&self, uri: &str) -> bool {
        self.subscriptions
            .lock()
            .map(|g| g.contains(uri))
            .unwrap_or(false)
    }

    fn request_id_key(id: &Value) -> String {
        // request ids in JSON-RPC can be string or number. Normalize to a
        // single string key so DashMap keys are uniform.
        match id {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    }
}

pub struct McpHandler {
    initialized: AtomicBool,
    iii: III,
    exposure: ExposureConfig,
    worker_manager: WorkerManager,
    triggers: Mutex<HashMap<String, Trigger>>,
    notification_rx: tokio::sync::Mutex<mpsc::Receiver<String>>,
    state: Arc<SessionState>,
    _functions_guard: FunctionsAvailableGuard,
    _bg_tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl McpHandler {
    pub fn new(iii: III, engine_url: String, exposure: ExposureConfig) -> Self {
        let (tx, notification_rx) = mpsc::channel(64);
        let state = Arc::new(SessionState::new(tx.clone()));

        // tools/list_changed: piggyback on the SDK's on_functions_available
        // callback. resources/list_changed shares the same trigger because
        // listing functions is what `iii://functions` resolves to.
        let state_for_fn_cb = state.clone();
        let guard = iii.on_functions_available(move |_| {
            let n = json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" });
            if let Ok(s) = serde_json::to_string(&n) {
                state_for_fn_cb.try_send(s);
            }
            // Also fire resources/updated for iii://functions if subscribed.
            if state_for_fn_cb.is_subscribed("iii://functions") {
                if let Ok(s) =
                    serde_json::to_string(&resource_updated_notification("iii://functions"))
                {
                    state_for_fn_cb.try_send(s);
                }
            }
        });

        // Background poll: workers + triggers don't have an SDK callback, so
        // poll every 5s and emit resources/updated on diff. Cheap (two engine
        // round-trips) and only runs while the handler is alive.
        let mut bg = Vec::new();
        let iii_poll = iii.clone();
        let state_poll = state.clone();
        let poll_handle = tokio::spawn(async move {
            poll_resource_updates(iii_poll, state_poll).await;
        });
        bg.push(poll_handle);

        // Register `mcp::log_message` and `mcp::progress` so user functions
        // can drive notifications. Both are plain iii functions, kept inside
        // the always-hidden `mcp::` namespace per ALWAYS_HIDDEN_PREFIXES so
        // they never appear in tools/list.
        register_notification_helpers(&iii, state.clone());

        Self {
            initialized: AtomicBool::new(false),
            iii,
            exposure,
            worker_manager: WorkerManager::new(engine_url),
            triggers: Mutex::new(HashMap::new()),
            notification_rx: tokio::sync::Mutex::new(notification_rx),
            state,
            _functions_guard: guard,
            _bg_tasks: bg,
        }
    }

    pub fn state(&self) -> &Arc<SessionState> {
        &self.state
    }

    pub async fn take_notification(&self) -> Option<String> {
        self.notification_rx.lock().await.try_recv().ok()
    }

    pub async fn handle(&self, body: Value) -> Option<Value> {
        let method = body.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = body.get("id").cloned();

        if method.starts_with("notifications/") {
            if method == "notifications/initialized" {
                self.initialized.store(true, Ordering::SeqCst);
            }
            if method == "notifications/cancelled" {
                if let Some(req_id) = body.get("params").and_then(|p| p.get("requestId")).cloned() {
                    let key = SessionState::request_id_key(&req_id);
                    if let Some((_, sender)) = self.state.cancellation_tokens.remove(&key) {
                        let _ = sender.send(());
                    }
                }
            }
            return None;
        }

        if method == "initialize" {
            self.initialized.store(true, Ordering::SeqCst);
        } else if !self.initialized.load(Ordering::SeqCst) && method != "ping" {
            return Some(json!(JsonRpcResponse::error(
                id,
                INTERNAL_ERROR,
                "Not initialized"
            )));
        }

        Some(self.dispatch(&body).await)
    }

    async fn dispatch(&self, body: &Value) -> Value {
        let method = body.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = body.get("id").cloned();
        let params = body.get("params").cloned();
        let cursor = params
            .as_ref()
            .and_then(|p| p.get("cursor"))
            .and_then(|c| c.as_str())
            .map(String::from);

        let result = match method {
            "initialize" => Ok(initialize_result()),
            "ping" => Ok(json!({})),
            "tools/list" => self
                .tools_list(cursor.as_deref())
                .await
                .map_err(|e| (INTERNAL_ERROR, e)),
            "tools/call" => self
                .tools_call(id.clone(), body)
                .await
                .map_err(|e| (INVALID_PARAMS, e)),
            "resources/list" => Ok(self.resources_list(cursor.as_deref())),
            "resources/read" => self
                .resources_read(params)
                .await
                .map_err(|e| (INVALID_PARAMS, e)),
            "resources/templates/list" => Ok(json!({
                "resourceTemplates": spec::make_resource_templates()
            })),
            "resources/subscribe" => {
                spec::handle_resources_subscribe(&self.state.subscriptions, params)
                    .map_err(|e| (INVALID_PARAMS, e))
            }
            "resources/unsubscribe" => {
                spec::handle_resources_unsubscribe(&self.state.subscriptions, params)
                    .map_err(|e| (INVALID_PARAMS, e))
            }
            "prompts/list" => Ok(prompts_list_paginated(cursor.as_deref())),
            "prompts/get" => Ok(prompts::get(params)),
            "completion/complete" => spec::handle_completion_complete(&self.iii, params)
                .await
                .map_err(|e| (INVALID_PARAMS, e)),
            "logging/setLevel" => spec::handle_logging_set_level(&self.state.log_level, params)
                .map_err(|e| (INVALID_PARAMS, e)),
            _ => Err((METHOD_NOT_FOUND, format!("Unknown method: {}", method))),
        };

        json!(match result {
            Ok(value) => JsonRpcResponse::success(id, value),
            Err((code, msg)) => JsonRpcResponse::error(id, code, msg),
        })
    }

    async fn tools_list(&self, cursor: Option<&str>) -> Result<Value, String> {
        let mut tools = if self.exposure.no_builtins {
            Vec::new()
        } else {
            builtin_tools()
        };
        if let Ok(functions) = self.iii.list_functions().await {
            tools.extend(
                functions
                    .iter()
                    .filter(|f| is_function_exposed(f, &self.exposure))
                    .map(function_to_tool),
            );
        }
        let (page, next) = paginate(&tools, cursor, PAGE_SIZE);
        let page_owned: Vec<McpTool> = page.into_iter().cloned().collect();
        let mut out = json!({ "tools": page_owned });
        if let Some(c) = next {
            out["nextCursor"] = json!(c);
        }
        Ok(out)
    }

    async fn tools_call(&self, id: Option<Value>, body: &Value) -> Result<Value, String> {
        let raw_params = body.get("params").cloned();
        let params: CallParams = parse(raw_params.clone())?;

        // _meta.progressToken: bind it to the request id so a tool implementation
        // calling `mcp::progress` with this token gets routed through.
        let progress_token = raw_params
            .as_ref()
            .and_then(|p| p.get("_meta"))
            .and_then(|m| m.get("progressToken"))
            .cloned();
        if let (Some(req_id), Some(token)) = (id.clone(), progress_token.clone()) {
            self.state.progress_tokens.insert(req_id, token);
        }

        // Register cancellation receiver. Even if tools_call short-circuits
        // before reaching iii.trigger, the slot is removed at end via the
        // RAII-ish scope guard pattern below.
        let req_key = id.as_ref().map(SessionState::request_id_key);
        let cancel_rx = if let Some(ref key) = req_key {
            let (tx, rx) = oneshot::channel();
            self.state.cancellation_tokens.insert(key.clone(), tx);
            Some(rx)
        } else {
            None
        };

        let result = self.tools_call_inner(params, cancel_rx).await;

        // Cleanup keyed state regardless of outcome.
        if let Some(key) = req_key {
            self.state.cancellation_tokens.remove(&key);
        }
        if let Some(req_id) = id {
            self.state.progress_tokens.remove(&req_id);
        }
        result
    }

    async fn tools_call_inner(
        &self,
        params: CallParams,
        cancel_rx: Option<oneshot::Receiver<()>>,
    ) -> Result<Value, String> {
        // When builtins are hidden from tools/list, also refuse to dispatch
        // them by name. Otherwise a client could invoke `iii_worker_register`
        // on a server that claimed it had no such tool — bypassing the policy.
        if self.exposure.no_builtins && is_builtin_tool(&params.name) {
            return Ok(tool_error(&format!(
                "Tool '{}' is disabled on this server (--no-builtins is set)",
                params.name
            )));
        }

        match params.name.as_str() {
            "iii_worker_register" => {
                let p: WorkerCreateParams = parse(Some(params.arguments))?;
                return match self.worker_manager.create_worker(p).await {
                    Ok(r) => Ok(tool_json(&serde_json::to_value(&r).unwrap_or_default())),
                    Err(e) => Ok(tool_error(&e)),
                };
            }
            "iii_worker_stop" => {
                let p: WorkerStopParams = parse(Some(params.arguments))?;
                return match self.worker_manager.stop_worker(p).await {
                    Ok(r) => Ok(tool_json(&serde_json::to_value(&r).unwrap_or_default())),
                    Err(e) => Ok(tool_error(&e)),
                };
            }
            "iii_trigger_register" => return Ok(self.trigger_register(params.arguments).await),
            "iii_trigger_unregister" => return Ok(self.trigger_unregister(params.arguments).await),
            "iii_trigger_void" => {
                let fid = str_field(&params.arguments, "function_id");
                if fid.is_empty() {
                    return Ok(tool_error("Missing required field: function_id"));
                }
                let payload = params
                    .arguments
                    .get("payload")
                    .cloned()
                    .unwrap_or(json!({}));
                return match self
                    .iii
                    .trigger(TriggerRequest {
                        function_id: fid.clone(),
                        payload,
                        action: Some(TriggerAction::Void),
                        timeout_ms: None,
                    })
                    .await
                {
                    Ok(_) => Ok(tool_result(&format!("Triggered (void): {}", fid))),
                    Err(e) => Ok(tool_error(&format!("Error: {}", e))),
                };
            }
            "iii_trigger_enqueue" => {
                let fid = str_field(&params.arguments, "function_id");
                if fid.is_empty() {
                    return Ok(tool_error("Missing required field: function_id"));
                }
                let payload = params
                    .arguments
                    .get("payload")
                    .cloned()
                    .unwrap_or(json!({}));
                let queue = str_field_or(&params.arguments, "queue", "default");
                return match self
                    .iii
                    .trigger(TriggerRequest {
                        function_id: fid,
                        payload,
                        action: Some(TriggerAction::Enqueue { queue }),
                        timeout_ms: None,
                    })
                    .await
                {
                    Ok(r) => Ok(tool_json(&r)),
                    Err(e) => Ok(tool_error(&format!("Error: {}", e))),
                };
            }
            _ => {}
        }

        let function_id = params.name.replace("__", "::");
        // Hard floor applies before the metadata gate — a hidden infra
        // function stays unreachable even when --expose-all is set.
        if is_always_hidden(&function_id) {
            return Ok(tool_error(&format!(
                "Function '{}' is in the iii-engine internal namespace and is not exposed as an MCP tool",
                function_id
            )));
        }
        // Fail closed: if we can't verify exposure (engine unreachable,
        // list_functions errored), refuse the call. The earlier code
        // silently dropped the check on Err and handed through to
        // iii.trigger — that meant an engine outage masked the policy gate.
        let fns = match self.iii.list_functions().await {
            Ok(fns) => fns,
            Err(e) => {
                tracing::error!(error = %e, function_id = %function_id, "list_functions failed; denying call");
                return Ok(tool_error(&format!(
                    "Function '{}' exposure could not be verified (engine list_functions error); denying call",
                    function_id
                )));
            }
        };
        let matched = fns.iter().find(|f| f.function_id == function_id);
        let exposed = matched
            .map(|f| is_function_exposed(f, &self.exposure))
            .unwrap_or(false);
        if !exposed {
            return Ok(tool_error(&format!(
                "Function '{}' is not exposed via mcp.expose metadata (or does not match the active --tier filter)",
                function_id
            )));
        }
        let trigger_fut = self.iii.trigger(TriggerRequest {
            function_id: function_id.clone(),
            payload: params.arguments,
            action: None,
            timeout_ms: None,
        });

        // notifications/cancelled tracking. If the receiver fires before the
        // trigger resolves, return a cancelled tool_error rather than an
        // ordinary failure — clients distinguish via the message text.
        let outcome = if let Some(rx) = cancel_rx {
            tokio::select! {
                biased;
                _ = rx => return Ok(tool_error("Request cancelled")),
                r = trigger_fut => r,
            }
        } else {
            trigger_fut.await
        };

        match outcome {
            Ok(result) => Ok(tool_json(&result)),
            Err(err) => {
                tracing::error!(function_id = %function_id, error = %err, "Trigger failed");
                Ok(tool_error(&format!("Error: {}", err)))
            }
        }
    }

    fn resources_list(&self, cursor: Option<&str>) -> Value {
        resources_list_paginated(cursor)
    }

    async fn resources_read(&self, params: Option<Value>) -> Result<Value, String> {
        read_resource(&self.iii, &self.exposure, params).await
    }

    async fn trigger_register(&self, args: Value) -> Value {
        #[derive(Deserialize)]
        struct P {
            trigger_type: String,
            function_id: String,
            config: Value,
        }
        let p: P = match parse(Some(args)) {
            Ok(p) => p,
            Err(e) => return tool_error(&e),
        };
        match self.iii.register_trigger(RegisterTriggerInput {
            trigger_type: p.trigger_type.clone(),
            function_id: p.function_id.clone(),
            config: p.config,
            metadata: None,
        }) {
            Ok(trigger) => {
                let id = uuid::Uuid::new_v4().to_string();
                self.triggers.lock().await.insert(id.clone(), trigger);
                tool_json(
                    &json!({ "id": id, "trigger_type": p.trigger_type, "function_id": p.function_id }),
                )
            }
            Err(e) => tool_error(&format!("Error: {}", e)),
        }
    }

    async fn trigger_unregister(&self, args: Value) -> Value {
        #[derive(Deserialize)]
        struct P {
            id: String,
        }
        let p: P = match parse(Some(args)) {
            Ok(p) => p,
            Err(e) => return tool_error(&e),
        };
        match self.triggers.lock().await.remove(&p.id) {
            Some(trigger) => {
                trigger.unregister();
                tool_json(&json!({ "id": p.id, "message": "Unregistered" }))
            }
            None => tool_error(&format!("Trigger not found: {}", p.id)),
        }
    }
}

pub fn register_http(iii: &III, exposure: ExposureConfig) {
    let iii_fn = iii.clone();
    let cfg = exposure;
    // Shared session state for the HTTP transport. Single-tenant: every POST
    // /mcp request reuses the same subscriptions/log_level/progress state
    // until the worker is restarted. Sufficient for the current Phase 2a
    // surface; per-session multiplexing (Streamable HTTP SSE) is explicitly
    // out of scope. The notifier mpsc receiver is dropped on the spot — HTTP
    // has no return channel for notifications, so emitting them is a no-op.
    let (notifier_tx, notifier_rx) = mpsc::channel(64);
    drop(notifier_rx);
    let state = Arc::new(SessionState::new(notifier_tx));
    register_notification_helpers(iii, state.clone());

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "mcp::handler".to_string(),
            description: Some("MCP JSON-RPC handler".to_string()),
            request_format: Some(json!({ "type": "object", "properties": { "body": { "type": "object" } } })),
            response_format: None, metadata: None, invocation: None,
        },
        move |input: Value| {
            let iii_inner = iii_fn.clone();
            let cfg_inner = cfg.clone();
            let state_inner = state.clone();
            async move {
                let body = input.get("body").cloned().unwrap_or(input);
                let response = dispatch_http(&iii_inner, &body, &cfg_inner, &state_inner).await;
                Ok(json!({ "status_code": 200, "headers": { "content-type": "application/json" }, "body": response }))
            }
        },
    );

    if let Err(e) = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "mcp::handler".to_string(),
        config: json!({ "api_path": "mcp", "http_method": "POST" }),
        metadata: None,
    }) {
        tracing::error!(error = %e, "Failed to register MCP HTTP trigger");
    } else {
        tracing::info!("MCP Streamable HTTP registered: POST /mcp");
    }
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "capabilities": {
            "tools": { "listChanged": true },
            "resources": { "subscribe": true, "listChanged": true },
            "prompts": { "listChanged": false },
            "logging": {},
            "completions": {}
        },
        "serverInfo": { "name": "iii-mcp", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "iii-engine MCP server. Only functions with metadata `mcp.expose: true` are exposed as tools. Infra namespaces (engine::*, state::*, stream::*, iii.*, mcp::*) are always hidden. Optional `mcp.tier` metadata is filtered by the server's --tier flag."
    })
}

// Static resource list shared by stdio + HTTP. Pagination on this short list
// is mostly future-proofing — the four built-in URIs always fit in one page.
fn static_resources() -> Vec<Value> {
    vec![
        json!({ "uri": "iii://functions", "name": "Functions", "mimeType": "application/json" }),
        json!({ "uri": "iii://workers", "name": "Workers", "mimeType": "application/json" }),
        json!({ "uri": "iii://triggers", "name": "Triggers", "mimeType": "application/json" }),
        json!({ "uri": "iii://context", "name": "Context", "mimeType": "application/json" }),
    ]
}

fn resources_list_paginated(cursor: Option<&str>) -> Value {
    let items = static_resources();
    let (page, next) = paginate(&items, cursor, PAGE_SIZE);
    let page_owned: Vec<Value> = page.into_iter().cloned().collect();
    let mut out = json!({ "resources": page_owned });
    if let Some(c) = next {
        out["nextCursor"] = json!(c);
    }
    out
}

fn prompts_list_paginated(cursor: Option<&str>) -> Value {
    // prompts::list() returns `{prompts: [...]}`. Pull the array, paginate,
    // re-wrap. Keeping the wire shape stable — clients call prompts/list
    // identically with or without cursor support.
    let full = prompts::list();
    let arr = full
        .get("prompts")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let (page, next) = paginate(&arr, cursor, PAGE_SIZE);
    let page_owned: Vec<Value> = page.into_iter().cloned().collect();
    let mut out = json!({ "prompts": page_owned });
    if let Some(c) = next {
        out["nextCursor"] = json!(c);
    }
    out
}

// Drives `notifications/resources/updated` for `iii://workers` and
// `iii://triggers` — neither has an SDK callback. 5s poll, content-hash
// diff so we don't fire on every tick. Stops when the channel closes
// (handler dropped).
async fn poll_resource_updates(iii: III, state: Arc<SessionState>) {
    // Prime baselines from one initial fetch BEFORE the loop so the first
    // diff has something to compare against. Without this, the first
    // successful list_workers() always produced Some(_) != None and emitted
    // a phantom resources/updated to every fresh subscriber.
    let mut last_workers: Option<String> = iii
        .list_workers()
        .await
        .ok()
        .and_then(|ws| serde_json::to_string(&ws).ok());
    let mut last_triggers: Option<String> = iii
        .list_triggers(true)
        .await
        .ok()
        .and_then(|ts| serde_json::to_string(&ts).ok());
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Skip the immediate first tick; we want diffs against a known baseline.
    interval.tick().await;
    loop {
        if state.notifier.is_closed() {
            return;
        }
        interval.tick().await;
        if let Ok(ws) = iii.list_workers().await {
            let h = serde_json::to_string(&ws).unwrap_or_default();
            if last_workers.as_ref() != Some(&h) {
                last_workers = Some(h);
                if state.is_subscribed("iii://workers") {
                    if let Ok(s) =
                        serde_json::to_string(&resource_updated_notification("iii://workers"))
                    {
                        state.try_send(s);
                    }
                }
            }
        }
        if let Ok(ts) = iii.list_triggers(true).await {
            let h = serde_json::to_string(&ts).unwrap_or_default();
            if last_triggers.as_ref() != Some(&h) {
                last_triggers = Some(h);
                if state.is_subscribed("iii://triggers") {
                    if let Ok(s) =
                        serde_json::to_string(&resource_updated_notification("iii://triggers"))
                    {
                        state.try_send(s);
                    }
                }
            }
        }
    }
}

// Register `mcp::log_message` and `mcp::progress` as iii functions. User
// code triggers them to emit MCP notifications. Both gated by the current
// session state — the log level filter is enforced here so no notification
// leaks past the threshold.
fn register_notification_helpers(iii: &III, state: Arc<SessionState>) {
    // Idempotent: the same process may construct an HTTP register_http() AND
    // a stdio McpHandler::new(); both call this. Engine register_function_with
    // on a duplicate id silently overwrites the prior closure, so the second
    // call would orphan the first SessionState. Skip after the first call.
    static REGISTERED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if REGISTERED.set(()).is_err() {
        tracing::debug!("mcp::log_message and mcp::progress already registered; skipping");
        return;
    }
    let state_log = state.clone();
    iii.register_function_with(
        RegisterFunctionMessage {
            id: "mcp::log_message".to_string(),
            description: Some("Emit an MCP notifications/message".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "level": { "type": "string" },
                    "data": {},
                    "logger": { "type": "string" }
                },
                "required": ["level", "data"]
            })),
            response_format: None,
            metadata: None,
            invocation: None,
        },
        move |input: Value| {
            let state_inner = state_log.clone();
            async move {
                let level_str = input
                    .get("level")
                    .and_then(|v| v.as_str())
                    .unwrap_or("info");
                let data = input.get("data").cloned().unwrap_or(Value::Null);
                let logger = input.get("logger").and_then(|v| v.as_str());
                let msg_lvl = match level_from_str(level_str) {
                    Some(l) => l,
                    None => return Ok(json!({ "ok": false, "error": "unknown level" })),
                };
                let cur = state_inner.log_level.load(Ordering::SeqCst);
                if msg_lvl < cur {
                    return Ok(json!({ "ok": true, "filtered": true }));
                }
                let n = log_message_notification(level_str, &data, logger);
                if let Ok(s) = serde_json::to_string(&n) {
                    state_inner.try_send(s);
                }
                Ok(json!({ "ok": true }))
            }
        },
    );

    let state_prog = state.clone();
    iii.register_function_with(
        RegisterFunctionMessage {
            id: "mcp::progress".to_string(),
            description: Some("Emit an MCP notifications/progress".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "token": {},
                    "progress": { "type": "number" },
                    "total": { "type": "number" },
                    "message": { "type": "string" }
                },
                "required": ["token", "progress"]
            })),
            response_format: None,
            metadata: None,
            invocation: None,
        },
        move |input: Value| {
            let state_inner = state_prog.clone();
            async move {
                let token = match input.get("token").cloned() {
                    Some(t) => t,
                    None => return Ok(json!({ "ok": false, "error": "missing token" })),
                };
                let progress = input
                    .get("progress")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let total = input.get("total").and_then(|v| v.as_f64());
                let message = input.get("message").and_then(|v| v.as_str());
                let n = progress_notification(&token, progress, total, message);
                if let Ok(s) = serde_json::to_string(&n) {
                    state_inner.try_send(s);
                }
                Ok(json!({ "ok": true }))
            }
        },
    );
}

async fn dispatch_http(
    iii: &III,
    body: &Value,
    cfg: &ExposureConfig,
    state: &Arc<SessionState>,
) -> Value {
    let method = body.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let id = body.get("id").cloned();
    let params = body.get("params").cloned();
    let cursor = params
        .as_ref()
        .and_then(|p| p.get("cursor"))
        .and_then(|c| c.as_str())
        .map(String::from);

    if method.starts_with("notifications/") {
        if method == "notifications/cancelled" {
            if let Some(req_id) = body.get("params").and_then(|p| p.get("requestId")).cloned() {
                let key = SessionState::request_id_key(&req_id);
                if let Some((_, sender)) = state.cancellation_tokens.remove(&key) {
                    let _ = sender.send(());
                }
            }
        }
        return json!(null);
    }

    let result = match method {
        "initialize" => Ok(initialize_result()),
        "ping" => Ok(json!({})),
        "tools/list" => {
            // HTTP transport hides builtins by default — worker/trigger
            // management requires stdio, so listing them over HTTP is pure
            // noise that errors on invocation anyway.
            let mut tools = if cfg.no_builtins {
                Vec::new()
            } else {
                builtin_tools()
            };
            if let Ok(fns) = iii.list_functions().await {
                tools.extend(
                    fns.iter()
                        .filter(|f| is_function_exposed(f, cfg))
                        .map(function_to_tool),
                );
            }
            let (page, next) = paginate(&tools, cursor.as_deref(), PAGE_SIZE);
            let page_owned: Vec<McpTool> = page.into_iter().cloned().collect();
            let mut out = json!({ "tools": page_owned });
            if let Some(c) = next {
                out["nextCursor"] = json!(c);
            }
            Ok(out)
        }
        "tools/call" => {
            // _meta.progressToken parity with stdio: bind it to the request
            // id so a future Streamable HTTP SSE transport (Phase 2c) can
            // route notifications/progress correlated to the right request.
            // Today the HTTP `register_http` drops its notifier_rx, so the
            // notification itself is a no-op — the binding is bookkeeping.
            let progress_token = params
                .as_ref()
                .and_then(|p| p.get("_meta"))
                .and_then(|m| m.get("progressToken"))
                .cloned();
            if let (Some(req_id), Some(token)) = (id.clone(), progress_token) {
                state.progress_tokens.insert(req_id, token);
            }
            let p: CallParams = match params {
                Some(p) => match serde_json::from_value(p) {
                    Ok(p) => p,
                    Err(e) => {
                        if let Some(req_id) = id.clone() {
                            state.progress_tokens.remove(&req_id);
                        }
                        return json!(JsonRpcResponse::error(id, INVALID_PARAMS, format!("{}", e)));
                    }
                },
                None => {
                    if let Some(req_id) = id.clone() {
                        state.progress_tokens.remove(&req_id);
                    }
                    return json!(JsonRpcResponse::error(id, INVALID_PARAMS, "Missing params"));
                }
            };
            // HTTP path: each POST is single-shot, no notifications/cancelled
            // correlation. Streamable HTTP SSE (Phase 2c) will add it.
            // When --no-builtins is active, the six management tools are
            // also not invocable by name. tools/list hides them; this
            // matches the listing shape at call time.
            if cfg.no_builtins && is_builtin_tool(&p.name) {
                if let Some(req_id) = id.clone() {
                    state.progress_tokens.remove(&req_id);
                }
                return json!(JsonRpcResponse::success(
                    id,
                    tool_error(&format!(
                        "Tool '{}' is disabled on this server (--no-builtins is set)",
                        p.name
                    ))
                ));
            }
            match p.name.as_str() {
                "iii_worker_register"
                | "iii_worker_stop"
                | "iii_trigger_register"
                | "iii_trigger_unregister" => Ok(tool_error(
                    "This tool requires stdio transport (worker/trigger management is not available over HTTP)",
                )),
                "iii_trigger_void" => {
                    let fid = str_field(&p.arguments, "function_id");
                    if fid.is_empty() {
                        Ok(tool_error("Missing required field: function_id"))
                    } else {
                        let payload = p.arguments.get("payload").cloned().unwrap_or(json!({}));
                        match iii
                            .trigger(TriggerRequest {
                                function_id: fid.clone(),
                                payload,
                                action: Some(TriggerAction::Void),
                                timeout_ms: None,
                            })
                            .await
                        {
                            Ok(_) => Ok(tool_result(&format!("Triggered (void): {}", fid))),
                            Err(e) => Ok(tool_error(&format!("Error: {}", e))),
                        }
                    }
                }
                "iii_trigger_enqueue" => {
                    let fid = str_field(&p.arguments, "function_id");
                    if fid.is_empty() {
                        Ok(tool_error("Missing required field: function_id"))
                    } else {
                        let payload = p.arguments.get("payload").cloned().unwrap_or(json!({}));
                        let queue = str_field_or(&p.arguments, "queue", "default");
                        match iii
                            .trigger(TriggerRequest {
                                function_id: fid,
                                payload,
                                action: Some(TriggerAction::Enqueue { queue }),
                                timeout_ms: None,
                            })
                            .await
                        {
                            Ok(r) => Ok(tool_json(&r)),
                            Err(e) => Ok(tool_error(&format!("Error: {}", e))),
                        }
                    }
                }
                _ => {
                    let function_id = p.name.replace("__", "::");
                    if is_always_hidden(&function_id) {
                        return json!(JsonRpcResponse::success(
                            id,
                            tool_error(&format!(
                                "Function '{}' is in the iii-engine internal namespace and is not exposed as an MCP tool",
                                function_id
                            ))
                        ));
                    }
                    // Fail closed on list_functions error — see stdio
                    // tools_call for the reasoning.
                    let fns = match iii.list_functions().await {
                        Ok(fns) => fns,
                        Err(e) => {
                            tracing::error!(error = %e, function_id = %function_id, "list_functions failed; denying call");
                            return json!(JsonRpcResponse::success(
                                id,
                                tool_error(&format!(
                                    "Function '{}' exposure could not be verified (engine list_functions error); denying call",
                                    function_id
                                ))
                            ));
                        }
                    };
                    let matched = fns.iter().find(|f| f.function_id == function_id);
                    let exposed = matched
                        .map(|f| is_function_exposed(f, cfg))
                        .unwrap_or(false);
                    if !exposed {
                        return json!(JsonRpcResponse::success(
                            id,
                            tool_error(&format!(
                                "Function '{}' is not exposed via mcp.expose metadata (or does not match the active --tier filter)",
                                function_id
                            ))
                        ));
                    }
                    match iii
                        .trigger(TriggerRequest {
                            function_id,
                            payload: p.arguments,
                            action: None,
                            timeout_ms: None,
                        })
                        .await
                    {
                        Ok(r) => Ok(tool_json(&r)),
                        Err(e) => Ok(tool_error(&format!("Error: {}", e))),
                    }
                }
            }
        }
        "resources/list" => Ok(resources_list_paginated(cursor.as_deref())),
        // HTTP transport previously omitted these; MCP Inspector hit
        // -32601 Unknown method on both. Parity with stdio McpHandler
        // via the shared read_resource / templates paths.
        "resources/read" => read_resource(iii, cfg, params)
            .await
            .map_err(|e| (INVALID_PARAMS, e)),
        "resources/templates/list" => Ok(json!({
            "resourceTemplates": spec::make_resource_templates()
        })),
        "resources/subscribe" => spec::handle_resources_subscribe(&state.subscriptions, params)
            .map_err(|e| (INVALID_PARAMS, e)),
        "resources/unsubscribe" => spec::handle_resources_unsubscribe(&state.subscriptions, params)
            .map_err(|e| (INVALID_PARAMS, e)),
        "prompts/list" => Ok(prompts_list_paginated(cursor.as_deref())),
        "prompts/get" => Ok(prompts::get(params)),
        "completion/complete" => spec::handle_completion_complete(iii, params)
            .await
            .map_err(|e| (INVALID_PARAMS, e)),
        "logging/setLevel" => spec::handle_logging_set_level(&state.log_level, params)
            .map_err(|e| (INVALID_PARAMS, e)),
        _ => Err((METHOD_NOT_FOUND, format!("Unknown method: {}", method))),
    };

    json!(match result {
        Ok(v) => JsonRpcResponse::success(id, v),
        Err((code, msg)) => JsonRpcResponse::error(id, code, msg),
    })
}

fn parse<T: serde::de::DeserializeOwned>(params: Option<Value>) -> Result<T, String> {
    match params {
        Some(p) => serde_json::from_value(p).map_err(|e| format!("Invalid params: {}", e)),
        None => Err("Missing params".to_string()),
    }
}

// Shared resource reader used by both stdio McpHandler::resources_read and
// HTTP dispatch_http. Keeps the `iii://functions` exposure filter in exactly
// one place so stdio and HTTP can't drift (stdio filtered, HTTP didn't →
// MCP Inspector surfaced the gap when it called resources/read over HTTP
// and got -32601 Unknown method, which is how this refactor landed).
pub async fn read_resource(
    iii: &III,
    cfg: &ExposureConfig,
    params: Option<Value>,
) -> Result<Value, String> {
    #[derive(Deserialize)]
    struct P {
        uri: String,
    }
    let p: P = parse(params)?;
    let (text, mime) = match p.uri.as_str() {
        "iii://functions" => {
            let v = iii.list_functions().await.map_err(|e| format!("{}", e))?;
            let filtered: Vec<_> = v
                .into_iter()
                .filter(|f| is_function_exposed(f, cfg))
                .collect();
            (
                serde_json::to_string_pretty(&filtered).unwrap_or_else(|_| "[]".into()),
                "application/json",
            )
        }
        "iii://workers" => {
            let v = iii.list_workers().await.map_err(|e| format!("{}", e))?;
            (
                serde_json::to_string_pretty(&v).unwrap_or_else(|_| "[]".into()),
                "application/json",
            )
        }
        "iii://triggers" => {
            let v = iii
                .list_triggers(true)
                .await
                .map_err(|e| format!("{}", e))?;
            (
                serde_json::to_string_pretty(&v).unwrap_or_else(|_| "[]".into()),
                "application/json",
            )
        }
        "iii://context" => (
            serde_json::to_string_pretty(&json!({
                "sdk_version": env!("CARGO_PKG_VERSION"),
                "function_id_delimiter": "::",
                "metadata_filtering": { "mcp.expose": true, "a2a.expose": true }
            }))
            .unwrap(),
            "application/json",
        ),
        // Templated URIs: iii://function/{id}, iii://worker/{id}, iii://trigger/{id}.
        // Resolved through spec::resolve_templated_uri which filters by id only;
        // exposure metadata is enforced for `iii://function/{id}` here so a
        // `mcp.expose: false` function can't leak via direct templated read.
        other => match resolve_templated_uri(other, iii).await {
            Some((body, mime)) => {
                if let Some(id) = other.strip_prefix("iii://function/") {
                    if let Ok(fns) = iii.list_functions().await {
                        let allowed = fns
                            .iter()
                            .find(|f| f.function_id == id)
                            .map(|f| is_function_exposed(f, cfg))
                            .unwrap_or(false);
                        if !allowed {
                            return Err(format!(
                                "Function '{}' is not exposed via mcp.expose metadata",
                                id
                            ));
                        }
                    }
                }
                (body, mime)
            }
            None => return Err(format!("Resource not found: {}", p.uri)),
        },
    };
    Ok(json!({ "contents": [{ "uri": p.uri, "mimeType": mime, "text": text }] }))
}

fn function_to_tool(f: &FunctionInfo) -> McpTool {
    let (title, annotations) = match f.metadata.as_ref() {
        Some(meta) => {
            let ann = make_tool_annotations(meta);
            // Title goes both at top level (per 2025-06-18 spec) and inside
            // `annotations` if the metadata supplied it; the spec is permissive
            // about either location, but tools/list clients prefer top-level.
            let t = ann.as_ref().and_then(|a| a.title.clone());
            (t, ann)
        }
        None => (None, None),
    };
    McpTool {
        name: f.function_id.replace("::", "__"),
        title,
        description: f.description.clone(),
        input_schema: f
            .request_format
            .clone()
            .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
        output_schema: f.response_format.clone(),
        annotations,
    }
}

// Single source of truth for the built-in tool name list. Both the
// listing path (builtin_tools()) and the invocation gate
// (tools_call --no-builtins check) consult this.
const BUILTIN_TOOL_NAMES: &[&str] = &[
    "iii_worker_register",
    "iii_worker_stop",
    "iii_trigger_register",
    "iii_trigger_unregister",
    "iii_trigger_void",
    "iii_trigger_enqueue",
];

fn is_builtin_tool(name: &str) -> bool {
    BUILTIN_TOOL_NAMES.contains(&name)
}

fn builtin_tools() -> Vec<McpTool> {
    vec![
        McpTool::new(
            "iii_worker_register",
            Some("Register a new worker (Node.js or Python)".into()),
            json!({ "type": "object", "properties": { "language": { "type": "string", "enum": ["node", "python"] }, "code": { "type": "string" }, "function_name": { "type": "string" }, "description": { "type": "string" } }, "required": ["language", "code", "function_name"] }),
        ),
        McpTool::new(
            "iii_worker_stop",
            Some("Stop a worker".into()),
            json!({ "type": "object", "properties": { "id": { "type": "string" } }, "required": ["id"] }),
        ),
        McpTool::new(
            "iii_trigger_register",
            Some("Attach an http/cron/queue trigger to a function".into()),
            json!({ "type": "object", "properties": { "trigger_type": { "type": "string" }, "function_id": { "type": "string" }, "config": { "type": "object" } }, "required": ["trigger_type", "function_id", "config"] }),
        ),
        McpTool::new(
            "iii_trigger_unregister",
            Some("Unregister a trigger".into()),
            json!({ "type": "object", "properties": { "id": { "type": "string" } }, "required": ["id"] }),
        ),
        McpTool::new(
            "iii_trigger_void",
            Some("Fire-and-forget function invocation".into()),
            json!({ "type": "object", "properties": { "function_id": { "type": "string" }, "payload": { "type": "object" } }, "required": ["function_id", "payload"] }),
        ),
        McpTool::new(
            "iii_trigger_enqueue",
            Some("Enqueue to named queue".into()),
            json!({ "type": "object", "properties": { "function_id": { "type": "string" }, "payload": { "type": "object" }, "queue": { "type": "string" } }, "required": ["function_id", "payload"] }),
        ),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpTool {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    annotations: Option<ToolAnnotations>,
}

impl McpTool {
    fn new(name: &str, description: Option<String>, input_schema: Value) -> Self {
        Self {
            name: name.to_string(),
            title: None,
            description,
            input_schema,
            output_schema: None,
            annotations: None,
        }
    }
}

#[derive(Deserialize)]
struct CallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

fn tool_result(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": false })
}
fn tool_error(msg: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": msg }], "isError": true })
}
fn tool_json(v: &Value) -> Value {
    tool_result(&serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()))
}
fn str_field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}
fn str_field_or(v: &Value, key: &str, default: &str) -> String {
    v.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or(default)
        .to_string()
}
