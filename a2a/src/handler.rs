use std::sync::Arc;

use iii_sdk::{
    III, RegisterFunctionMessage, RegisterTriggerInput, TriggerAction, TriggerRequest, Value,
};
use serde_json::json;

use crate::streaming::{
    StreamRegistry, broadcast_artifact, broadcast_status, dispatch_resubscribe, dispatch_stream,
    writer_ref_from_input,
};
use crate::types::*;

// Blocks self-invocation of the A2A handler and cross-protocol bridging
// through the sibling MCP entry point. Pure structural guard — not access
// control. RBAC lives at iii-worker-manager.
pub fn is_protocol_loop(function_id: &str) -> bool {
    function_id.starts_with("mcp::") || function_id.starts_with("a2a::")
}

#[derive(Debug, Clone)]
pub struct AgentIdentity {
    pub name: String,
    pub description: String,
    pub provider_org: String,
    pub provider_url: String,
    pub docs_url: String,
}

// Single source of truth for AgentIdentity defaults — also referenced by
// clap's #[arg(default_value = ...)] in main.rs so the two stay in sync.
pub const DEFAULT_AGENT_NAME: &str = "iii-engine";
pub const DEFAULT_AGENT_DESCRIPTION: &str =
    "iii-engine agent — invoke any registered function via A2A";
pub const DEFAULT_PROVIDER_ORG: &str = "iii";
pub const DEFAULT_PROVIDER_URL: &str = "https://github.com/iii-hq/iii";
pub const DEFAULT_DOCS_URL: &str = "https://github.com/iii-hq/workers/tree/main/a2a";

impl Default for AgentIdentity {
    fn default() -> Self {
        Self {
            name: DEFAULT_AGENT_NAME.to_string(),
            description: DEFAULT_AGENT_DESCRIPTION.to_string(),
            provider_org: DEFAULT_PROVIDER_ORG.to_string(),
            provider_url: DEFAULT_PROVIDER_URL.to_string(),
            docs_url: DEFAULT_DOCS_URL.to_string(),
        }
    }
}

pub fn register(iii: &III, base_url: String, identity: AgentIdentity) {
    let iii_card = iii.clone();
    let card_base_url = base_url.clone();
    let card_identity = identity.clone();
    iii.register_function_with(
        RegisterFunctionMessage {
            id: "a2a::agent_card".to_string(),
            description: Some("A2A Agent Card".to_string()),
            request_format: None,
            response_format: None,
            metadata: None,
            invocation: None,
        },
        move |_input: Value| {
            let iii_inner = iii_card.clone();
            let base = card_base_url.clone();
            let ident = card_identity.clone();
            async move {
                let card = build_agent_card(&iii_inner, &base, &ident).await;
                Ok(json!({
                    "status_code": 200,
                    "headers": { "content-type": "application/json" },
                    "body": card
                }))
            }
        },
    );

    let iii_rpc = iii.clone();
    let registry = Arc::new(StreamRegistry::new());
    let registry_rpc = registry.clone();
    iii.register_function_with(
        RegisterFunctionMessage {
            id: "a2a::jsonrpc".to_string(),
            description: Some("A2A JSON-RPC endpoint".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": { "body": { "type": "object" } }
            })),
            response_format: None,
            metadata: None,
            invocation: None,
        },
        move |input: Value| {
            let iii_inner = iii_rpc.clone();
            let registry = registry_rpc.clone();
            async move {
                // Snapshot the writer ref BEFORE stripping the body — once
                // we narrow `input` down to the JSON-RPC body the channel
                // ref disappears.
                let writer_ref = writer_ref_from_input(&input);

                let body = if let Some(b) = input.get("body") {
                    b.clone()
                } else {
                    input
                };

                let request: A2ARequest = match serde_json::from_value(body) {
                    Ok(r) => r,
                    Err(e) => {
                        return Ok(json!({
                            "status_code": 200,
                            "headers": { "content-type": "application/json" },
                            "body": A2AResponse::error(None, -32600, format!("Invalid request: {}", e))
                        }));
                    }
                };

                // Streaming methods take the writer ref directly and emit
                // SSE; the JSON-RPC envelope is unused for the response.
                match request.method.as_str() {
                    "message/stream" | "SendStreamingMessage" => {
                        let Some(writer_ref) = writer_ref else {
                            return Ok(json!({
                                "status_code": 200,
                                "headers": { "content-type": "application/json" },
                                "body": A2AResponse::error(
                                    request.id,
                                    -32004,
                                    "Streaming not supported on this transport (no writable channel)"
                                )
                            }));
                        };
                        dispatch_stream(&iii_inner, request.params, writer_ref, registry).await;
                        // The SSE response was written directly to the
                        // channel; return an empty value so the engine
                        // doesn't try to write a second response body.
                        return Ok(Value::Null);
                    }
                    "tasks/resubscribe" | "SubscribeToTask" => {
                        let Some(writer_ref) = writer_ref else {
                            return Ok(json!({
                                "status_code": 200,
                                "headers": { "content-type": "application/json" },
                                "body": A2AResponse::error(
                                    request.id,
                                    -32004,
                                    "Streaming not supported on this transport (no writable channel)"
                                )
                            }));
                        };
                        dispatch_resubscribe(&iii_inner, request.params, writer_ref, registry)
                            .await;
                        return Ok(Value::Null);
                    }
                    _ => {}
                }

                let response = handle_a2a_request(&iii_inner, request, &registry).await;

                Ok(json!({
                    "status_code": 200,
                    "headers": { "content-type": "application/json" },
                    "body": response
                }))
            }
        },
    );

    if let Err(e) = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "a2a::agent_card".to_string(),
        config: json!({ "api_path": ".well-known/agent-card.json", "http_method": "GET" }),
        metadata: None,
    }) {
        tracing::error!(error = %e, "Failed to register a2a::agent_card trigger");
    }

    if let Err(e) = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "a2a::jsonrpc".to_string(),
        config: json!({ "api_path": "a2a", "http_method": "POST" }),
        metadata: None,
    }) {
        tracing::error!(error = %e, "Failed to register a2a::jsonrpc trigger");
    }

    tracing::info!("A2A registered: GET /.well-known/agent-card.json, POST /a2a");
}

pub async fn build_agent_card(iii: &III, base_url: &str, identity: &AgentIdentity) -> AgentCard {
    let skills = match iii.list_functions().await {
        Ok(fns) => fns
            .iter()
            .filter(|f| !is_protocol_loop(&f.function_id))
            .map(|f| AgentSkill {
                id: f.function_id.clone(),
                name: f
                    .description
                    .clone()
                    .unwrap_or_else(|| f.function_id.replace("::", " ")),
                description: f
                    .description
                    .clone()
                    .unwrap_or_else(|| f.function_id.clone()),
                tags: f.function_id.split("::").map(|s| s.to_string()).collect(),
                examples: None,
            })
            .collect(),
        Err(_) => vec![],
    };

    let base = base_url.trim().trim_end_matches('/');
    // A2A v0.3 AgentProvider requires BOTH organization and url. Omit the
    // provider object if either is empty rather than emit a half-populated
    // record that violates the spec.
    let provider = if identity.provider_org.is_empty() || identity.provider_url.is_empty() {
        None
    } else {
        Some(AgentProvider {
            organization: identity.provider_org.clone(),
            url: identity.provider_url.clone(),
        })
    };
    let documentation_url = if identity.docs_url.is_empty() {
        None
    } else {
        Some(identity.docs_url.clone())
    };
    AgentCard {
        name: identity.name.clone(),
        description: identity.description.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        supported_interfaces: vec![AgentInterface {
            url: format!("{}/a2a", base),
            protocol_binding: "JSONRPC".to_string(),
            protocol_version: "0.3".to_string(),
        }],
        provider,
        documentation_url,
        capabilities: AgentCapabilities {
            streaming: true,
            push_notifications: false,
            state_transition_history: true,
        },
        default_input_modes: vec!["text/plain".to_string(), "application/json".to_string()],
        default_output_modes: vec!["text/plain".to_string(), "application/json".to_string()],
        skills,
    }
}

pub async fn handle_a2a_request(
    iii: &III,
    request: A2ARequest,
    registry: &Arc<StreamRegistry>,
) -> A2AResponse {
    let id = request.id.clone();
    match request.method.as_str() {
        "message/send" | "SendMessage" => handle_send(iii, id, request.params, registry).await,
        "tasks/get" | "GetTask" => handle_get(iii, id, request.params).await,
        "tasks/cancel" | "CancelTask" => handle_cancel(iii, id, request.params, registry).await,
        "tasks/list" | "ListTasks" => handle_list(iii, id).await,
        m if m.contains("pushNotification") || m.contains("PushNotification") => {
            A2AResponse::error(id, -32003, "Push notifications not supported")
        }
        _ => A2AResponse::error(id, -32601, format!("Unknown method: {}", request.method)),
    }
}

const TASK_SCOPE: &str = "a2a:tasks";

pub async fn store_task(iii: &III, task: &Task) {
    if let Err(e) = iii
        .trigger(TriggerRequest {
            function_id: "state::set".to_string(),
            payload: json!({ "scope": TASK_SCOPE, "key": task.id, "data": task }),
            action: Some(TriggerAction::Void),
            timeout_ms: None,
        })
        .await
    {
        tracing::error!(task_id = %task.id, error = %e, "Failed to store task");
    }
}

pub async fn load_task(iii: &III, task_id: &str) -> Option<Task> {
    iii.trigger(TriggerRequest {
        function_id: "state::get".to_string(),
        payload: json!({ "scope": TASK_SCOPE, "key": task_id }),
        action: None,
        timeout_ms: Some(5000),
    })
    .await
    .ok()
    .and_then(|v| serde_json::from_value(v).ok())
}

pub fn msg_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn text_part(s: impl Into<String>) -> Part {
    Part {
        text: Some(s.into()),
        data: None,
        url: None,
        raw: None,
        media_type: None,
    }
}

pub fn iso_now() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    let millis = d.subsec_millis();
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let h = time_secs / 3600;
    let m = (time_secs % 3600) / 60;
    let s = time_secs % 60;

    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let year_days = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            366
        } else {
            365
        };
        if remaining < year_days {
            break;
        }
        remaining -= year_days;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut mo = 0;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining < md as i64 {
            mo = i + 1;
            break;
        }
        remaining -= md as i64;
    }
    let day = remaining + 1;

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        y, mo, day, h, m, s, millis
    )
}

async fn handle_send(
    iii: &III,
    id: Option<Value>,
    params: Option<Value>,
    registry: &Arc<StreamRegistry>,
) -> A2AResponse {
    let params: SendMessageParams = match params {
        Some(p) => match serde_json::from_value(p) {
            Ok(p) => p,
            Err(e) => return A2AResponse::error(id, -32602, format!("Invalid params: {}", e)),
        },
        None => return A2AResponse::error(id, -32602, "Missing params"),
    };

    let task_id = params
        .message
        .task_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let context_id = params.message.context_id.clone();

    // Resolve and structurally validate the function id BEFORE building or
    // storing the task. RBAC is enforced at iii-worker-manager — we only
    // reject the empty case and the protocol-loop case here. This avoids
    // the spurious Working→Failed double-write the older code did.
    let (function_id, payload) = resolve_function(&params.message);
    let validation_failure = if function_id.is_empty() {
        Some(
            "No function_id found. Send a data part with {\"function_id\": \"...\", \"payload\": {...}} or use :: notation in text."
                .to_string(),
        )
    } else if is_protocol_loop(&function_id) {
        Some(format!(
            "Function '{}' is a protocol entry point, not a callable tool",
            function_id
        ))
    } else {
        None
    };

    if let Some(reason) = validation_failure {
        // Terminal-task idempotency: if the same task_id was already
        // resolved (good or bad), return the stored copy rather than
        // overwriting with a fresh failure record.
        if let Some(existing) = load_task(iii, &task_id).await
            && matches!(
                existing.status.state,
                TaskState::Completed
                    | TaskState::Canceled
                    | TaskState::Failed
                    | TaskState::Rejected
            )
        {
            return A2AResponse::success(id, json!({ "task": existing }));
        }
        let task = build_failed_task(task_id, context_id, &params, &reason);
        store_task(iii, &task).await;
        return A2AResponse::success(id, json!({ "task": task }));
    }

    let mut task = if let Some(existing) = load_task(iii, &task_id).await {
        if matches!(
            existing.status.state,
            TaskState::Completed | TaskState::Canceled | TaskState::Failed | TaskState::Rejected
        ) {
            return A2AResponse::success(id, json!({ "task": existing }));
        }
        let mut t = existing;
        if let Some(ref mut history) = t.history {
            history.push(params.message.clone());
        }
        t.status = TaskStatus {
            state: TaskState::Working,
            message: Some(Message {
                message_id: msg_id(),
                role: MessageRole::Agent,
                parts: vec![text_part("Processing...")],
                task_id: None,
                context_id: None,
                metadata: None,
            }),
            timestamp: Some(iso_now()),
        };
        t
    } else {
        Task {
            id: task_id.clone(),
            context_id,
            status: TaskStatus {
                state: TaskState::Working,
                message: Some(Message {
                    message_id: msg_id(),
                    role: MessageRole::Agent,
                    parts: vec![text_part("Processing...")],
                    task_id: None,
                    context_id: None,
                    metadata: None,
                }),
                timestamp: Some(iso_now()),
            },
            artifacts: None,
            history: Some(vec![params.message.clone()]),
            metadata: params.metadata.clone(),
        }
    };
    store_task(iii, &task).await;
    // Broadcast Working transition so concurrent stream subscribers see
    // the live update instead of needing to poll.
    broadcast_status(registry, &task, false).await;

    let fn_name = function_id.clone();

    match iii
        .trigger(TriggerRequest {
            function_id,
            payload,
            action: None,
            timeout_ms: Some(30000),
        })
        .await
    {
        Ok(result) => {
            let fresh = load_task(iii, &task_id).await;
            if let Some(ref t) = fresh
                && matches!(t.status.state, TaskState::Canceled)
            {
                // Cancel path already broadcast its own final frame.
                return A2AResponse::success(id, json!({ "task": t }));
            }
            let result_text =
                serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string());
            let artifact = Artifact {
                artifact_id: uuid::Uuid::new_v4().to_string(),
                parts: vec![Part {
                    text: Some(result_text),
                    data: None,
                    url: None,
                    raw: None,
                    media_type: Some("application/json".to_string()),
                }],
                name: Some(fn_name),
                metadata: None,
            };
            task.status = TaskStatus {
                state: TaskState::Completed,
                message: None,
                timestamp: Some(iso_now()),
            };
            task.artifacts = Some(vec![artifact.clone()]);
            broadcast_artifact(registry, &task, &artifact).await;
        }
        Err(err) => {
            task.status = TaskStatus {
                state: TaskState::Failed,
                message: Some(Message {
                    message_id: msg_id(),
                    role: MessageRole::Agent,
                    parts: vec![text_part(format!("Error: {}", err))],
                    task_id: None,
                    context_id: None,
                    metadata: None,
                }),
                timestamp: Some(iso_now()),
            };
        }
    }

    store_task(iii, &task).await;
    broadcast_status(registry, &task, true).await;
    registry.close_task(&task.id).await;
    A2AResponse::success(id, json!({ "task": task }))
}

// Build a Failed task in one shot for early validation rejections. Avoids
// the older Working→Failed double-write when the function id can't be
// dispatched (empty / protocol-loop).
fn build_failed_task(
    task_id: String,
    context_id: Option<String>,
    params: &SendMessageParams,
    reason: &str,
) -> Task {
    Task {
        id: task_id,
        context_id,
        status: TaskStatus {
            state: TaskState::Failed,
            message: Some(Message {
                message_id: msg_id(),
                role: MessageRole::Agent,
                parts: vec![text_part(reason)],
                task_id: None,
                context_id: None,
                metadata: None,
            }),
            timestamp: Some(iso_now()),
        },
        artifacts: None,
        history: Some(vec![params.message.clone()]),
        metadata: params.metadata.clone(),
    }
}

async fn handle_get(iii: &III, id: Option<Value>, params: Option<Value>) -> A2AResponse {
    let params: GetTaskParams = match params {
        Some(p) => match serde_json::from_value(p) {
            Ok(p) => p,
            Err(e) => return A2AResponse::error(id, -32602, format!("Invalid params: {}", e)),
        },
        None => return A2AResponse::error(id, -32602, "Missing params"),
    };
    match load_task(iii, &params.id).await {
        Some(task) => A2AResponse::success(id, json!({ "task": task })),
        None => A2AResponse::error(id, -32001, format!("Task not found: {}", params.id)),
    }
}

async fn handle_list(iii: &III, id: Option<Value>) -> A2AResponse {
    match iii
        .trigger(TriggerRequest {
            function_id: "state::list".to_string(),
            payload: json!({ "scope": TASK_SCOPE }),
            action: None,
            timeout_ms: Some(5000),
        })
        .await
    {
        Ok(value) => {
            let tasks: Vec<Task> = value
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| serde_json::from_value(v.clone()).ok())
                        .collect()
                })
                .unwrap_or_default();
            A2AResponse::success(id, json!({ "tasks": tasks }))
        }
        Err(_) => A2AResponse::success(id, json!({ "tasks": [] })),
    }
}

async fn handle_cancel(
    iii: &III,
    id: Option<Value>,
    params: Option<Value>,
    registry: &Arc<StreamRegistry>,
) -> A2AResponse {
    let params: CancelTaskParams = match params {
        Some(p) => match serde_json::from_value(p) {
            Ok(p) => p,
            Err(e) => return A2AResponse::error(id, -32602, format!("Invalid params: {}", e)),
        },
        None => return A2AResponse::error(id, -32602, "Missing params"),
    };
    match load_task(iii, &params.id).await {
        Some(mut task) => {
            if matches!(
                task.status.state,
                TaskState::Completed
                    | TaskState::Canceled
                    | TaskState::Failed
                    | TaskState::Rejected
            ) {
                return A2AResponse::error(id, -32002, "Task not cancelable (terminal state)");
            }
            task.status = TaskStatus {
                state: TaskState::Canceled,
                message: None,
                timestamp: Some(iso_now()),
            };
            store_task(iii, &task).await;
            // Broadcast the canceled final frame so any concurrent stream
            // subscribers wake up and tear down their writer.
            broadcast_status(registry, &task, true).await;
            registry.close_task(&task.id).await;
            A2AResponse::success(id, json!({ "task": task }))
        }
        None => A2AResponse::error(id, -32001, format!("Task not found: {}", params.id)),
    }
}

pub fn resolve_function(message: &Message) -> (String, Value) {
    let text = message
        .parts
        .iter()
        .find_map(|p| p.text.as_ref())
        .cloned()
        .unwrap_or_default();
    let data_payload = message.parts.iter().find_map(|p| p.data.as_ref());

    if let Some(payload) = data_payload
        && let Some(fid) = payload.get("function_id").and_then(|v| v.as_str())
    {
        let args = payload.get("payload").cloned().unwrap_or(json!({}));
        return (fid.to_string(), args);
    }

    // Only treat the message as a direct function invocation when the very
    // first token looks like `namespace::fn_name`. Otherwise free-form
    // text like "please run orders::process" would resolve the function_id
    // to "please", then fail with a confusing not-exposed error.
    let text = text.trim();
    let first_token = text.split(char::is_whitespace).next().unwrap_or("");
    if first_token.contains("::") {
        let rest = text[first_token.len()..].trim_start();
        if !rest.is_empty() {
            let payload = serde_json::from_str(rest).unwrap_or(json!({ "input": rest }));
            return (first_token.to_string(), payload);
        }
        return (first_token.to_string(), json!({}));
    }

    (String::new(), json!({}))
}
