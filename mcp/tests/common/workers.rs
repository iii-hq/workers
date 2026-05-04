//! In-process registration of the `mcp` surface against the shared
//! SDK handle, plus stub implementations of the skills::* / prompts::*
//! functions the dispatcher delegates into.
//!
//! Re-uses the same entry point the production binary does
//! (`iii_mcp::functions::register_all`), so the BDD scenarios exercise
//! identical handler code paths. The skills-side stubs let us assert
//! on delegation without depending on the skills crate or running a
//! real skills binary.

use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use iii_sdk::{IIIError, RegisterFunctionMessage, III};
use serde_json::{json, Value};
use tokio::sync::OnceCell;

use iii_mcp::{config::McpConfig, functions};

pub struct Shared {
    pub cfg: Arc<McpConfig>,
}

static SHARED: OnceCell<Arc<Shared>> = OnceCell::const_new();

/// Idempotent: the first caller registers; subsequent callers reuse.
pub async fn register_all(iii: &Arc<III>) -> Result<Arc<Shared>> {
    if let Some(s) = SHARED.get() {
        return Ok(s.clone());
    }

    let cfg = Arc::new(McpConfig::default());
    let iii_arc = iii.clone();
    functions::register_all(&iii_arc, &cfg);

    register_skills_stubs(iii);
    register_prompts_stubs(iii);
    register_tools_fixtures(iii);

    // Give the SDK a beat to publish the function registrations before
    // scenarios start triggering them.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let shared = Arc::new(Shared { cfg });
    let _ = SHARED.set(shared.clone());
    Ok(shared)
}

pub fn shared() -> Option<Arc<Shared>> {
    SHARED.get().cloned()
}

/// Stub `skills::resources-*` so the dispatcher's delegation path can
/// be exercised without a real skills binary.
fn register_skills_stubs(iii: &III) {
    iii.register_function_with(
        RegisterFunctionMessage {
            id: "skills::resources-list".into(),
            description: Some("BDD stub: returns one fixture resource.".into()),
            request_format: None,
            response_format: None,
            metadata: None,
            invocation: None,
        },
        |_input: Value| -> Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            Box::pin(async move {
                Ok(json!({
                    "resources": [
                        {
                            "uri": "iii://skills",
                            "name": "skills",
                            "description": "Index of every registered skill",
                            "mimeType": "text/markdown"
                        },
                        {
                            "uri": "iii://demo",
                            "name": "demo",
                            "mimeType": "text/markdown"
                        }
                    ]
                }))
            })
        },
    );

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "skills::resources-read".into(),
            description: Some("BDD stub: echoes the uri back as content.".into()),
            request_format: None,
            response_format: None,
            metadata: None,
            invocation: None,
        },
        |input: Value| -> Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            Box::pin(async move {
                let uri = input
                    .get("uri")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(json!({
                    "contents": [
                        {
                            "uri": uri,
                            "mimeType": "text/markdown",
                            "text": format!("# stub\n\nfor {uri}\n")
                        }
                    ]
                }))
            })
        },
    );

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "skills::resources-templates".into(),
            description: Some("BDD stub: returns the two URI templates.".into()),
            request_format: None,
            response_format: None,
            metadata: None,
            invocation: None,
        },
        |_input: Value| -> Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            Box::pin(async move {
                Ok(json!({
                    "resourceTemplates": [
                        {
                            "uriTemplate": "iii://{skill_id}",
                            "name": "Skill",
                            "mimeType": "text/markdown"
                        },
                        {
                            "uriTemplate": "iii://{skill_id}/{function_id}",
                            "name": "Skill section",
                            "mimeType": "text/markdown"
                        }
                    ]
                }))
            })
        },
    );
}

/// Stub `prompts::mcp-list` and `prompts::mcp-get` for delegation tests.
fn register_prompts_stubs(iii: &III) {
    iii.register_function_with(
        RegisterFunctionMessage {
            id: "prompts::mcp-list".into(),
            description: Some("BDD stub: returns one fixture prompt.".into()),
            request_format: None,
            response_format: None,
            metadata: None,
            invocation: None,
        },
        |_input: Value| -> Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            Box::pin(async move {
                Ok(json!({
                    "prompts": [
                        {
                            "name": "demo-greet",
                            "description": "Greet someone.",
                            "arguments": [
                                { "name": "to", "required": true }
                            ]
                        }
                    ]
                }))
            })
        },
    );

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "prompts::mcp-get".into(),
            description: Some("BDD stub: returns a single user message.".into()),
            request_format: None,
            response_format: None,
            metadata: None,
            invocation: None,
        },
        |input: Value| -> Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            Box::pin(async move {
                let name = input
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let to = input
                    .get("arguments")
                    .and_then(|a| a.get("to"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("world")
                    .to_string();
                Ok(json!({
                    "description": format!("Stub render of {name}"),
                    "messages": [
                        {
                            "role": "user",
                            "content": { "type": "text", "text": format!("Hello, {to}!") }
                        }
                    ]
                }))
            })
        },
    );
}

/// Register a couple of regular user-namespace functions so tools/list
/// has something visible and tools/call has something to dispatch.
fn register_tools_fixtures(iii: &III) {
    iii.register_function_with(
        RegisterFunctionMessage {
            id: "bdd::echo".into(),
            description: Some("BDD: echo input back".into()),
            request_format: Some(json!({
                "type": "object",
                "properties": { "msg": { "type": "string" } },
                "required": ["msg"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": { "echoed": { "type": "string" } }
            })),
            metadata: None,
            invocation: None,
        },
        |input: Value| -> Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            Box::pin(async move {
                let msg = input
                    .get("msg")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| IIIError::Handler("missing required field: msg".into()))?;
                Ok(json!({ "echoed": msg }))
            })
        },
    );

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "bdd::boom".into(),
            description: Some("BDD: always errors".into()),
            request_format: None,
            response_format: None,
            metadata: None,
            invocation: None,
        },
        |_input: Value| -> Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            Box::pin(async move { Err(IIIError::Handler("kapow".into())) })
        },
    );
}
