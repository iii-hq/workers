use iii_sdk::{III, RegisterFunctionMessage};
use serde_json::json;
use std::sync::Arc;

use crate::config::SandboxConfig;
use crate::kvm::template::Template;
use crate::VmRegistry;

mod exec;
mod sandbox;

pub fn register_all(
    iii: &III,
    url: &str,
    template: Arc<Template>,
    config: Arc<SandboxConfig>,
    registry: VmRegistry,
) {
    let handler = sandbox::build_create_handler(
        url.to_string(),
        template.clone(),
        config.clone(),
        registry.clone(),
    );
    let _fn_create = iii.register_function(
        RegisterFunctionMessage {
            id: "sandbox::create".to_string(),
            description: Some("Fork a new KVM sandbox from the pre-loaded template".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "language": {
                        "type": "string",
                        "enum": ["python", "node", "javascript", "ruby", "bash"],
                        "description": "Runtime language for the sandbox"
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Sandbox lifetime in seconds"
                    }
                }
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "language": { "type": "string" },
                    "status": { "type": "string" },
                    "created_at": { "type": "integer" },
                    "expires_at": { "type": "integer" },
                    "fork_time_us": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        handler,
    );

    let handler = sandbox::build_get_handler(url.to_string());
    let _fn_get = iii.register_function(
        RegisterFunctionMessage {
            id: "sandbox::get".to_string(),
            description: Some("Get sandbox metadata by ID".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Sandbox ID" }
                },
                "required": ["id"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "language": { "type": "string" },
                    "status": { "type": "string" },
                    "created_at": { "type": "integer" },
                    "expires_at": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        handler,
    );

    let handler = sandbox::build_list_handler(url.to_string());
    let _fn_list = iii.register_function(
        RegisterFunctionMessage {
            id: "sandbox::list".to_string(),
            description: Some("List all active sandboxes".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {}
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "sandboxes": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "language": { "type": "string" },
                                "status": { "type": "string" }
                            }
                        }
                    }
                }
            })),
            metadata: None,
            invocation: None,
        },
        handler,
    );

    let handler = sandbox::build_kill_handler(url.to_string(), registry.clone());
    let _fn_kill = iii.register_function(
        RegisterFunctionMessage {
            id: "sandbox::kill".to_string(),
            description: Some("Kill and remove a sandbox".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Sandbox ID to kill" }
                },
                "required": ["id"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "killed": { "type": "boolean" },
                    "id": { "type": "string" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        handler,
    );

    let handler = exec::build_run_handler(url.to_string(), config.clone(), registry.clone());
    let _fn_run = iii.register_function(
        RegisterFunctionMessage {
            id: "exec::run".to_string(),
            description: Some("Run a shell command in a sandbox".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "sandbox_id": { "type": "string", "description": "Target sandbox ID" },
                    "command": { "type": "string", "description": "Shell command to execute" },
                    "timeout": { "type": "integer", "description": "Command timeout in seconds" }
                },
                "required": ["sandbox_id", "command"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "exit_code": { "type": "integer" },
                    "stdout": { "type": "string" },
                    "stderr": { "type": "string" },
                    "duration_us": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        handler,
    );

    let handler = exec::build_code_handler(url.to_string(), config.clone(), registry.clone());
    let _fn_code = iii.register_function(
        RegisterFunctionMessage {
            id: "exec::code".to_string(),
            description: Some("Run code in a sandbox using the configured language interpreter".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "sandbox_id": { "type": "string", "description": "Target sandbox ID" },
                    "code": { "type": "string", "description": "Source code to execute" },
                    "language": { "type": "string", "description": "Language override (defaults to sandbox language)" },
                    "timeout": { "type": "integer", "description": "Execution timeout in seconds" }
                },
                "required": ["sandbox_id", "code"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "exit_code": { "type": "integer" },
                    "stdout": { "type": "string" },
                    "stderr": { "type": "string" },
                    "duration_us": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        handler,
    );

    tracing::info!(
        "registered 6 functions: sandbox::create, sandbox::get, sandbox::list, sandbox::kill, exec::run, exec::code"
    );
}
