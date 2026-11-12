//! The MCP Streamable HTTP endpoint: one HTTP request in, one reply out.
//!
//! Order of checks: Origin (403), HTTP method (`DELETE` ends a session,
//! anything but `POST` is 405), session (`initialize` opens one and returns
//! its id; an unknown `Mcp-Session-Id` is 404 so the client re-initializes),
//! then JSON-RPC dispatch.

use serde_json::{json, Value};

use crate::bus::Bus;
use crate::config::McpConfig;
use crate::directory_bridge;
use crate::gateway;
use crate::jsonrpc::{
    is_json_rpc_request, json_rpc_error, json_rpc_success, parse_json_rpc_request,
    response_id_from_body, JsonRpcRequest, JsonRpcResponse, JsonRpcResponseId, INTERNAL_ERROR,
    INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND,
};
use crate::origin::{header, origin_allowed};
use crate::rbac_guard::RbacMode;
use crate::sessions::{Observe, Sessions};
use crate::tools::{mcp_tools_call, mcp_tools_list, McpToolsCallError};

pub const PROTOCOL_VERSION: &str = "2025-06-18";
pub const SESSION_HEADER: &str = "mcp-session-id";

/// Implementation-defined JSON-RPC code for transport-level refusals.
const SERVER_ERROR: i64 = -32000;

/// Everything one request runs against.
pub struct App<'a, B> {
    pub bus: &'a B,
    pub cfg: &'a McpConfig,
    pub sessions: &'a Sessions,
    pub mode: &'a RbacMode,
    pub sink: &'a str,
}

#[derive(Debug)]
pub struct HttpReply {
    pub status: u16,
    /// `None` for 202 (notifications) and bodiless replies.
    pub rpc: Option<JsonRpcResponse>,
    /// Set when `initialize` opened a session: the `Mcp-Session-Id` header.
    pub session_id: Option<String>,
}

impl HttpReply {
    fn rpc(status: u16, rpc: JsonRpcResponse) -> Self {
        Self {
            status,
            rpc: Some(rpc),
            session_id: None,
        }
    }
    fn empty(status: u16) -> Self {
        Self {
            status,
            rpc: None,
            session_id: None,
        }
    }
}

fn refusal(status: u16, message: &str) -> HttpReply {
    HttpReply::rpc(
        status,
        json_rpc_error(JsonRpcResponseId::Null, SERVER_ERROR, message.to_string()),
    )
}

fn initialize_result(mode: &RbacMode) -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {}, "resources": {} },
        "serverInfo": { "name": env!("CARGO_PKG_NAME"), "version": env!("CARGO_PKG_VERSION") },
        "instructions": gateway::instructions(mode),
    })
}

async fn tools_list<B: Bus>(app: &App<'_, B>) -> Result<Value, String> {
    let mut tools: Vec<Value> = Vec::new();
    if app.cfg.tools.gateway() {
        if let Some(list) = gateway::tools_list()["tools"].as_array() {
            tools.extend(list.iter().cloned());
        }
    }
    if app.cfg.tools.functions() {
        let listed = mcp_tools_list(app.bus, app.cfg)
            .await
            .map_err(|e| e.to_string())?;
        if let Some(list) = listed["tools"].as_array() {
            tools.extend(list.iter().cloned());
        }
    }
    Ok(json!({ "tools": tools }))
}

async fn tools_call<B: Bus + Observe>(
    app: &App<'_, B>,
    session_id: Option<&str>,
    params: &Value,
    id: JsonRpcResponseId,
) -> JsonRpcResponse {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    if name.is_empty() {
        return json_rpc_error(
            id,
            INVALID_PARAMS,
            "tools/call requires a non-empty string name".to_string(),
        );
    }
    let args = params
        .get("arguments")
        .filter(|a| a.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    if app.cfg.tools.gateway() {
        let ctx = gateway::Ctx {
            bus: app.bus,
            sessions: app.sessions,
            session_id,
            mode: app.mode,
            sink: app.sink,
        };
        if let Some(result) = gateway::call_tool(&ctx, name, &args).await {
            return json_rpc_success(id, result);
        }
    }
    if app.cfg.tools.functions() {
        return match mcp_tools_call(app.bus, app.cfg, params).await {
            Ok(v) => json_rpc_success(id, v),
            Err(McpToolsCallError::InvalidParams(e)) => {
                json_rpc_error(id, INVALID_PARAMS, e.to_string())
            }
        };
    }
    json_rpc_success(
        id,
        json!({ "content": [{ "type": "text", "text": format!("unknown tool: {name} (this server offers the iii_* gateway tools)") }], "isError": true }),
    )
}

async fn dispatch<B: Bus + Observe>(
    app: &App<'_, B>,
    req: &JsonRpcRequest,
    session_id: Option<&str>,
    id: JsonRpcResponseId,
) -> JsonRpcResponse {
    match req.method.as_str() {
        "initialize" => json_rpc_success(id, initialize_result(app.mode)),
        "ping" => json_rpc_success(id, json!({})),
        "tools/list" => match tools_list(app).await {
            Ok(v) => json_rpc_success(id, v),
            Err(e) => json_rpc_error(id, INTERNAL_ERROR, e),
        },
        "tools/call" => tools_call(app, session_id, &req.params, id).await,
        "resources/list" => match directory_bridge::resources_list(app.bus).await {
            Ok(v) => json_rpc_success(id, v),
            Err(e) => json_rpc_error(id, INTERNAL_ERROR, e.to_string()),
        },
        "resources/read" => match directory_bridge::resources_read(app.bus, &req.params).await {
            Ok(v) => json_rpc_success(id, v),
            Err(e) => json_rpc_error(id, INTERNAL_ERROR, e.to_string()),
        },
        "resources/templates/list" => json_rpc_success(id, directory_bridge::resource_templates()),
        // Profiles are reached through the gateway tools (and the iii Claude
        // plugin's /iii:* commands); this server offers no MCP prompts.
        "prompts/list" => json_rpc_success(id, json!({ "prompts": [] })),
        "prompts/get" => json_rpc_error(
            id,
            INVALID_PARAMS,
            "this server offers no prompts".to_string(),
        ),
        other => json_rpc_error(id, METHOD_NOT_FOUND, format!("Method not found: {other}")),
    }
}

pub async fn handle_http<B: Bus + Observe>(
    app: &App<'_, B>,
    http_method: &str,
    headers: &Value,
    body: &Value,
) -> HttpReply {
    if !origin_allowed(headers, &app.cfg.allowed_origins) {
        return refusal(
            403,
            "Forbidden: this Origin may not use the MCP server (see allowed_origins)",
        );
    }
    let session_header = header(headers, SESSION_HEADER).filter(|s| !s.is_empty());

    if http_method.eq_ignore_ascii_case("DELETE") {
        return match session_header {
            None => refusal(400, "Bad Request: DELETE needs an Mcp-Session-Id header"),
            Some(sid) if app.sessions.end(&sid).await => HttpReply::empty(200),
            Some(_) => refusal(404, "Session not found"),
        };
    }
    if !http_method.eq_ignore_ascii_case("POST") {
        return refusal(
            405,
            "Method Not Allowed: use POST (and DELETE to end a session)",
        );
    }

    if body.is_array() {
        return HttpReply::rpc(
            200,
            json_rpc_error(
                JsonRpcResponseId::Null,
                INVALID_REQUEST,
                "Batch requests are not supported".to_string(),
            ),
        );
    }
    let Some(req) = is_json_rpc_request(body)
        .then(|| parse_json_rpc_request(body))
        .flatten()
    else {
        return HttpReply::rpc(
            200,
            json_rpc_error(
                response_id_from_body(body),
                INVALID_REQUEST,
                "Invalid JSON-RPC request".to_string(),
            ),
        );
    };

    let initializing = req.method == "initialize";
    let session_id = if initializing {
        Some(app.sessions.create())
    } else {
        match session_header {
            Some(sid) if app.sessions.touch(&sid) => Some(sid),
            Some(_) => return refusal(404, "Session not found: initialize again"),
            None => None,
        }
    };

    let Some(id) = req.id.clone() else {
        return HttpReply::empty(202);
    };
    let rpc = dispatch(app, &req, session_id.as_deref(), JsonRpcResponseId::Id(id)).await;
    HttpReply {
        status: 200,
        rpc: Some(rpc),
        session_id: if initializing { session_id } else { None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::fake::FakeBus;
    use crate::config::{McpConfig, ToolMode};
    use crate::rbac_guard::{RbacMode, DANGEROUS_NOTICE};
    use crate::sessions::{SessionLimits, Sessions};
    use crate::testkit::Engine;
    use serde_json::json;
    use std::sync::atomic::Ordering;

    struct Fixture {
        engine: Engine,
        cfg: McpConfig,
        sessions: Sessions,
        mode: RbacMode,
    }

    impl Fixture {
        fn new(bus: FakeBus) -> Self {
            Self {
                engine: Engine::new(bus),
                cfg: McpConfig::default(),
                sessions: Sessions::new(SessionLimits::default()),
                mode: RbacMode::Proxied {
                    policy: json!({ "auth_function": true }),
                },
            }
        }
        fn app(&self) -> App<'_, Engine> {
            App {
                bus: &self.engine,
                cfg: &self.cfg,
                sessions: &self.sessions,
                mode: &self.mode,
                sink: "mcp::observed",
            }
        }
        async fn post(&self, headers: Value, body: Value) -> HttpReply {
            handle_http(&self.app(), "POST", &headers, &body).await
        }
    }

    fn rpc(method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
    }

    fn result(r: &HttpReply) -> Value {
        let v = serde_json::to_value(r.rpc.as_ref().expect("a JSON-RPC reply")).unwrap();
        assert!(v.get("error").is_none(), "unexpected error: {v}");
        v["result"].clone()
    }

    fn rpc_error(r: &HttpReply) -> Value {
        let v = serde_json::to_value(r.rpc.as_ref().expect("a JSON-RPC reply")).unwrap();
        v["error"].clone()
    }

    fn tool_text(r: &HttpReply) -> (bool, String) {
        let res = result(r);
        (
            res.get("isError") == Some(&json!(true)),
            res["content"][0]["text"].as_str().unwrap().to_string(),
        )
    }

    #[tokio::test]
    async fn initialize_opens_a_session_and_describes_the_gateway() {
        let f = Fixture::new(FakeBus::new());
        let r = f.post(json!({}), rpc("initialize", json!({}))).await;
        assert_eq!(r.status, 200);
        let sid = r.session_id.clone().expect("Mcp-Session-Id");
        assert!(f.sessions.touch(&sid));
        let res = result(&r);
        assert_eq!(res["protocolVersion"], PROTOCOL_VERSION);
        assert!(res["capabilities"]["tools"].is_object());
        assert!(res["capabilities"]["resources"].is_object());
        assert!(res["capabilities"].get("prompts").is_none());
        assert_eq!(res["serverInfo"]["name"], env!("CARGO_PKG_NAME"));
        assert!(res["instructions"].as_str().unwrap().contains("iii_call"));
    }

    #[tokio::test]
    async fn initialize_without_rbac_says_so_first() {
        let mut f = Fixture::new(FakeBus::new());
        f.mode = RbacMode::Dangerous;
        let r = f.post(json!({}), rpc("initialize", json!({}))).await;
        assert!(result(&r)["instructions"]
            .as_str()
            .unwrap()
            .starts_with(DANGEROUS_NOTICE));
    }

    #[tokio::test]
    async fn a_foreign_origin_is_refused_before_anything_runs() {
        let f = Fixture::new(FakeBus::new());
        let r = f
            .post(
                json!({ "origin": "https://evil.example" }),
                rpc(
                    "tools/call",
                    json!({ "name": "iii_call", "arguments": { "function_id": "a::b" } }),
                ),
            )
            .await;
        assert_eq!(r.status, 403);
        assert!(rpc_error(&r)["message"]
            .as_str()
            .unwrap()
            .contains("Origin"));
        assert!(f.engine.bus.calls().is_empty());
        assert!(r.session_id.is_none());
    }

    #[tokio::test]
    async fn a_configured_origin_is_allowed() {
        let mut f = Fixture::new(FakeBus::new());
        f.cfg.allowed_origins = vec!["https://console.example.com".into()];
        let r = f
            .post(
                json!({ "Origin": "https://console.example.com" }),
                rpc("ping", json!({})),
            )
            .await;
        assert_eq!(r.status, 200);
    }

    #[tokio::test]
    async fn an_unknown_session_is_404_so_the_client_reinitializes() {
        let f = Fixture::new(FakeBus::new());
        let r = f
            .post(
                json!({ "mcp-session-id": "gone" }),
                rpc("tools/list", json!({})),
            )
            .await;
        assert_eq!(r.status, 404);
        assert!(rpc_error(&r)["message"]
            .as_str()
            .unwrap()
            .contains("Session not found"));
    }

    #[tokio::test]
    async fn notifications_are_accepted_with_202() {
        let f = Fixture::new(FakeBus::new());
        let r = f
            .post(
                json!({}),
                json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            )
            .await;
        assert_eq!(r.status, 202);
        assert!(r.rpc.is_none());
    }

    #[tokio::test]
    async fn batches_and_garbage_are_invalid_requests() {
        let f = Fixture::new(FakeBus::new());
        assert_eq!(
            rpc_error(&f.post(json!({}), json!([1])).await)["code"],
            -32600
        );
        assert_eq!(
            rpc_error(&f.post(json!({}), json!({ "x": 1 })).await)["code"],
            -32600
        );
        assert_eq!(
            rpc_error(&f.post(json!({}), rpc("foo/bar", json!({}))).await)["code"],
            -32601
        );
    }

    #[tokio::test]
    async fn gateway_mode_lists_the_fixed_tools() {
        let f = Fixture::new(FakeBus::new());
        let tools = result(&f.post(json!({}), rpc("tools/list", json!({}))).await)["tools"].clone();
        assert_eq!(
            tools.as_array().unwrap().len(),
            crate::gateway::TOOL_NAMES.len()
        );
        assert!(
            f.engine.bus.calls().is_empty(),
            "gateway listing needs no engine call"
        );
    }

    fn functions_bus() -> FakeBus {
        FakeBus::new()
            .on("engine::functions::list", |_| {
                Ok(json!({ "functions": [
                    { "function_id": "myworker::echo", "description": "Echo" },
                    { "function_id": "state::set" }
                ]}))
            })
            .on("myworker::echo", |p| Ok(json!({ "echo": p })))
    }

    #[tokio::test]
    async fn functions_mode_lists_one_tool_per_visible_function() {
        let mut f = Fixture::new(functions_bus());
        f.cfg.tools = ToolMode::Functions;
        let tools = result(&f.post(json!({}), rpc("tools/list", json!({}))).await)["tools"].clone();
        let names: Vec<_> = tools
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].clone())
            .collect();
        assert_eq!(names, vec![json!("myworker__echo")]);
    }

    #[tokio::test]
    async fn both_mode_lists_gateway_then_function_tools() {
        let mut f = Fixture::new(functions_bus());
        f.cfg.tools = ToolMode::Both;
        let tools = result(&f.post(json!({}), rpc("tools/list", json!({}))).await)["tools"].clone();
        let names: Vec<String> = tools
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names[0], "iii_call");
        assert_eq!(names.last().unwrap(), "myworker__echo");
    }

    #[tokio::test]
    async fn gateway_mode_does_not_forward_function_style_tool_names() {
        let f = Fixture::new(functions_bus());
        let r = f
            .post(
                json!({}),
                rpc(
                    "tools/call",
                    json!({ "name": "myworker__echo", "arguments": {} }),
                ),
            )
            .await;
        let (err, text) = tool_text(&r);
        assert!(err && text.contains("unknown tool"), "{text}");
        assert!(f.engine.bus.calls().is_empty());
    }

    #[tokio::test]
    async fn functions_mode_still_calls_by_tool_name() {
        let mut f = Fixture::new(functions_bus());
        f.cfg.tools = ToolMode::Functions;
        let r = f
            .post(
                json!({}),
                rpc(
                    "tools/call",
                    json!({ "name": "myworker__echo", "arguments": { "a": 1 } }),
                ),
            )
            .await;
        assert_eq!(
            result(&r)["structuredContent"],
            json!({ "echo": { "a": 1 } })
        );
        // iii_call is not a tool in functions-only mode.
        let r = f
            .post(
                json!({}),
                rpc(
                    "tools/call",
                    json!({ "name": "iii_call", "arguments": { "function_id": "myworker::echo" } }),
                ),
            )
            .await;
        assert!(tool_text(&r).0);
    }

    #[tokio::test]
    async fn tools_call_without_a_name_is_invalid_params() {
        let f = Fixture::new(FakeBus::new());
        let r = f
            .post(json!({}), rpc("tools/call", json!({ "arguments": {} })))
            .await;
        assert_eq!(rpc_error(&r)["code"], -32602);
    }

    #[tokio::test]
    async fn observe_uses_the_session_from_the_header_and_targets_the_sink() {
        let f = Fixture::new(FakeBus::new());
        let sid = f
            .post(json!({}), rpc("initialize", json!({})))
            .await
            .session_id
            .unwrap();
        let r = f
            .post(
                json!({ "Mcp-Session-Id": sid }),
                rpc(
                    "tools/call",
                    json!({ "name": "iii_observe", "arguments": { "function_id": "state::set" } }),
                ),
            )
            .await;
        let (err, text) = tool_text(&r);
        assert!(!err, "{text}");
        assert_eq!(f.engine.observed.lock().unwrap()[0].1, "mcp::observed");
        // Without the header the session tools refuse.
        let r = f
            .post(
                json!({}),
                rpc(
                    "tools/call",
                    json!({ "name": "iii_observe", "arguments": { "function_id": "state::set" } }),
                ),
            )
            .await;
        assert!(tool_text(&r).0);
    }

    #[tokio::test]
    async fn resources_come_from_the_directory_and_prompts_are_empty() {
        let f = Fixture::new(FakeBus::new().on("directory::skills::list", |_| {
            Ok(json!({ "skills": [{ "id": "kanban" }] }))
        }));
        let r = f.post(json!({}), rpc("resources/list", json!({}))).await;
        assert_eq!(result(&r)["resources"][0]["uri"], "iii://kanban");
        let r = f
            .post(json!({}), rpc("resources/templates/list", json!({})))
            .await;
        assert_eq!(
            result(&r)["resourceTemplates"][0]["uriTemplate"],
            "iii://{id}"
        );
        assert_eq!(
            result(&f.post(json!({}), rpc("prompts/list", json!({}))).await),
            json!({ "prompts": [] })
        );
        assert_eq!(
            rpc_error(
                &f.post(json!({}), rpc("prompts/get", json!({ "name": "x" })))
                    .await
            )["code"],
            -32602
        );
        assert!(!f
            .engine
            .bus
            .calls()
            .iter()
            .any(|(id, _)| id.starts_with("skills::") || id.starts_with("prompts::")));
    }

    #[tokio::test]
    async fn delete_ends_the_session_and_disposes_its_observers() {
        let f = Fixture::new(FakeBus::new());
        let sid = f
            .post(json!({}), rpc("initialize", json!({})))
            .await
            .session_id
            .unwrap();
        f.post(
            json!({ "mcp-session-id": sid }),
            rpc(
                "tools/call",
                json!({ "name": "iii_observe", "arguments": { "function_id": "a::b" } }),
            ),
        )
        .await;
        let r = handle_http(
            &f.app(),
            "DELETE",
            &json!({ "mcp-session-id": sid }),
            &Value::Null,
        )
        .await;
        assert_eq!(r.status, 200);
        assert!(r.rpc.is_none());
        assert!(!f.sessions.touch(&sid));
        assert_eq!(f.engine.disposed.load(Ordering::SeqCst), 1);
        let r = handle_http(
            &f.app(),
            "DELETE",
            &json!({ "mcp-session-id": sid }),
            &Value::Null,
        )
        .await;
        assert_eq!(r.status, 404);
        let r = handle_http(&f.app(), "DELETE", &json!({}), &Value::Null).await;
        assert_eq!(r.status, 400);
    }

    #[tokio::test]
    async fn other_http_methods_are_405() {
        let f = Fixture::new(FakeBus::new());
        assert_eq!(
            handle_http(&f.app(), "GET", &json!({}), &Value::Null)
                .await
                .status,
            405
        );
    }
}
