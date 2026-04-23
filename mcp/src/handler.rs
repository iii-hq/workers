use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use iii_sdk::{
    FunctionInfo, FunctionsAvailableGuard, III, RegisterFunctionMessage, RegisterTriggerInput,
    Trigger, TriggerAction, TriggerRequest, Value,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Mutex, mpsc};

use crate::prompts;
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
    "engine::",
    "state::",
    "stream::",
    "iii.",
    // Protocol-worker entry points are stateless RPC dispatchers. Routing
    // an MCP tools/call through `mcp::handler` recurses into ourselves;
    // through `a2a::jsonrpc` double-envelopes an A2A request inside MCP.
    // Neither is a useful tool surface. Hide both even under --expose-all.
    "mcp::",
    "a2a::",
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

pub struct McpHandler {
    initialized: AtomicBool,
    iii: III,
    exposure: ExposureConfig,
    worker_manager: WorkerManager,
    triggers: Mutex<HashMap<String, Trigger>>,
    notification_rx: tokio::sync::Mutex<mpsc::Receiver<String>>,
    _functions_guard: FunctionsAvailableGuard,
}

impl McpHandler {
    pub fn new(iii: III, engine_url: String, exposure: ExposureConfig) -> Self {
        let (tx, notification_rx) = mpsc::channel(16);

        let guard = iii.on_functions_available(move |_| {
            let n = json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" });
            if let Ok(json) = serde_json::to_string(&n) {
                if tx.try_send(json).is_err() {
                    tracing::warn!("Notification channel full, tools/list_changed dropped");
                }
            }
        });

        Self {
            initialized: AtomicBool::new(false),
            iii,
            exposure,
            worker_manager: WorkerManager::new(engine_url),
            triggers: Mutex::new(HashMap::new()),
            notification_rx: tokio::sync::Mutex::new(notification_rx),
            _functions_guard: guard,
        }
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

        let result = match method {
            "initialize" => Ok(initialize_result()),
            "ping" => Ok(json!({})),
            "tools/list" => self.tools_list().await.map_err(|e| (INTERNAL_ERROR, e)),
            "tools/call" => self
                .tools_call(params)
                .await
                .map_err(|e| (INVALID_PARAMS, e)),
            "resources/list" => Ok(self.resources_list()),
            "resources/read" => self
                .resources_read(params)
                .await
                .map_err(|e| (INVALID_PARAMS, e)),
            "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
            "prompts/list" => Ok(prompts::list()),
            "prompts/get" => Ok(prompts::get(params)),
            _ => Err((METHOD_NOT_FOUND, format!("Unknown method: {}", method))),
        };

        json!(match result {
            Ok(value) => JsonRpcResponse::success(id, value),
            Err((code, msg)) => JsonRpcResponse::error(id, code, msg),
        })
    }

    async fn tools_list(&self) -> Result<Value, String> {
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
        Ok(json!({ "tools": tools }))
    }

    async fn tools_call(&self, params: Option<Value>) -> Result<Value, String> {
        let params: CallParams = parse(params)?;

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
        if let Ok(fns) = self.iii.list_functions().await {
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
        }
        match self
            .iii
            .trigger(TriggerRequest {
                function_id: function_id.clone(),
                payload: params.arguments,
                action: None,
                timeout_ms: None,
            })
            .await
        {
            Ok(result) => Ok(tool_json(&result)),
            Err(err) => {
                tracing::error!(function_id = %function_id, error = %err, "Trigger failed");
                Ok(tool_error(&format!("Error: {}", err)))
            }
        }
    }

    fn resources_list(&self) -> Value {
        json!({ "resources": [
            { "uri": "iii://functions", "name": "Functions", "mimeType": "application/json" },
            { "uri": "iii://workers", "name": "Workers", "mimeType": "application/json" },
            { "uri": "iii://triggers", "name": "Triggers", "mimeType": "application/json" },
            { "uri": "iii://context", "name": "Context", "mimeType": "application/json" },
        ]})
    }

    async fn resources_read(&self, params: Option<Value>) -> Result<Value, String> {
        #[derive(Deserialize)]
        struct P {
            uri: String,
        }
        let p: P = parse(params)?;
        let (text, mime) = match p.uri.as_str() {
            "iii://functions" => {
                let v = self
                    .iii
                    .list_functions()
                    .await
                    .map_err(|e| format!("{}", e))?;
                (
                    serde_json::to_string_pretty(&v).unwrap_or_else(|_| "[]".into()),
                    "application/json",
                )
            }
            "iii://workers" => {
                let v = self
                    .iii
                    .list_workers()
                    .await
                    .map_err(|e| format!("{}", e))?;
                (
                    serde_json::to_string_pretty(&v).unwrap_or_else(|_| "[]".into()),
                    "application/json",
                )
            }
            "iii://triggers" => {
                let v = self
                    .iii
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
                    "sdk_version": "0.10.0",
                    "function_id_delimiter": "::",
                    "metadata_filtering": { "mcp.expose": true, "a2a.expose": true }
                }))
                .unwrap(),
                "application/json",
            ),
            _ => return Err(format!("Resource not found: {}", p.uri)),
        };
        Ok(json!({ "contents": [{ "uri": p.uri, "mimeType": mime, "text": text }] }))
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
            async move {
                let body = input.get("body").cloned().unwrap_or(input);
                let response = dispatch_http(&iii_inner, &body, &cfg_inner).await;
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
        "capabilities": { "tools": { "listChanged": true }, "resources": { "subscribe": false, "listChanged": true }, "prompts": { "listChanged": false } },
        "serverInfo": { "name": "iii-mcp", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "iii-engine MCP server. Only functions with metadata `mcp.expose: true` are exposed as tools. Infra namespaces (engine::*, state::*, stream::*, iii.*, mcp::*) are always hidden. Optional `mcp.tier` metadata is filtered by the server's --tier flag."
    })
}

async fn dispatch_http(iii: &III, body: &Value, cfg: &ExposureConfig) -> Value {
    let method = body.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let id = body.get("id").cloned();
    let params = body.get("params").cloned();

    if method.starts_with("notifications/") {
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
            Ok(json!({ "tools": tools }))
        }
        "tools/call" => {
            let p: CallParams = match params {
                Some(p) => match serde_json::from_value(p) {
                    Ok(p) => p,
                    Err(e) => {
                        return json!(JsonRpcResponse::error(id, INVALID_PARAMS, format!("{}", e)));
                    }
                },
                None => return json!(JsonRpcResponse::error(id, INVALID_PARAMS, "Missing params")),
            };
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
                    if let Ok(fns) = iii.list_functions().await {
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
        "resources/list" => Ok(json!({ "resources": [
            { "uri": "iii://functions", "name": "Functions", "mimeType": "application/json" },
            { "uri": "iii://workers", "name": "Workers", "mimeType": "application/json" },
            { "uri": "iii://triggers", "name": "Triggers", "mimeType": "application/json" },
        ]})),
        "prompts/list" => Ok(prompts::list()),
        "prompts/get" => Ok(prompts::get(params)),
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

fn function_to_tool(f: &FunctionInfo) -> McpTool {
    McpTool {
        name: f.function_id.replace("::", "__"),
        description: f.description.clone(),
        input_schema: f
            .request_format
            .clone()
            .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
    }
}

fn builtin_tools() -> Vec<McpTool> {
    vec![
        McpTool {
            name: "iii_worker_register".into(),
            description: Some("Register a new worker (Node.js or Python)".into()),
            input_schema: json!({ "type": "object", "properties": { "language": { "type": "string", "enum": ["node", "python"] }, "code": { "type": "string" }, "function_name": { "type": "string" }, "description": { "type": "string" } }, "required": ["language", "code", "function_name"] }),
        },
        McpTool {
            name: "iii_worker_stop".into(),
            description: Some("Stop a worker".into()),
            input_schema: json!({ "type": "object", "properties": { "id": { "type": "string" } }, "required": ["id"] }),
        },
        McpTool {
            name: "iii_trigger_register".into(),
            description: Some("Attach an http/cron/queue trigger to a function".into()),
            input_schema: json!({ "type": "object", "properties": { "trigger_type": { "type": "string" }, "function_id": { "type": "string" }, "config": { "type": "object" } }, "required": ["trigger_type", "function_id", "config"] }),
        },
        McpTool {
            name: "iii_trigger_unregister".into(),
            description: Some("Unregister a trigger".into()),
            input_schema: json!({ "type": "object", "properties": { "id": { "type": "string" } }, "required": ["id"] }),
        },
        McpTool {
            name: "iii_trigger_void".into(),
            description: Some("Fire-and-forget function invocation".into()),
            input_schema: json!({ "type": "object", "properties": { "function_id": { "type": "string" }, "payload": { "type": "object" } }, "required": ["function_id", "payload"] }),
        },
        McpTool {
            name: "iii_trigger_enqueue".into(),
            description: Some("Enqueue to named queue".into()),
            input_schema: json!({ "type": "object", "properties": { "function_id": { "type": "string" }, "payload": { "type": "object" }, "queue": { "type": "string" } }, "required": ["function_id", "payload"] }),
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpTool {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    input_schema: Value,
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
