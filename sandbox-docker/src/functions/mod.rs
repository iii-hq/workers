pub mod exec;
pub mod fs;
pub mod sandbox;

use iii_sdk::{RegisterFunctionMessage, III};
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::config::SandboxWorkerConfig;

pub fn register_all(
    iii: &Arc<III>,
    docker: Arc<bollard::Docker>,
    config: Arc<SandboxWorkerConfig>,
) {
    register_sandbox_create(iii, docker.clone(), config.clone());
    register_sandbox_get(iii);
    register_sandbox_list(iii);
    register_sandbox_kill(iii, docker.clone());
    register_exec_run(iii, docker.clone(), config.clone());
    register_exec_code(iii, docker.clone(), config.clone());
    register_fs_read(iii, docker.clone(), config.clone());
    register_fs_write(iii, docker.clone(), config.clone());
    register_fs_list(iii, docker, config);
}

fn register_sandbox_create(
    iii: &Arc<III>,
    docker: Arc<bollard::Docker>,
    config: Arc<SandboxWorkerConfig>,
) {
    let iii_clone = iii.clone();
    let _fn_ref = iii.register_function(
        RegisterFunctionMessage {
            id: "sandbox::create".to_string(),
            description: Some("Create a Docker sandbox container".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "image": { "type": "string", "description": "Docker image to use" },
                    "timeout": { "type": "integer", "description": "TTL in seconds" },
                    "memory": { "type": "integer", "description": "Memory limit in MB" },
                    "cpu": { "type": "number", "description": "CPU cores" },
                    "network": { "type": "boolean", "description": "Enable network access" },
                    "env": { "type": "object", "description": "Environment variables" },
                    "workdir": { "type": "string", "description": "Working directory" }
                }
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "image": { "type": "string" },
                    "status": { "type": "string" },
                    "created_at": { "type": "integer" },
                    "expires_at": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: serde_json::Value| -> Pin<
            Box<dyn Future<Output = Result<serde_json::Value, iii_sdk::IIIError>> + Send>,
        > {
            let iii = iii_clone.clone();
            let docker = docker.clone();
            let config = config.clone();
            Box::pin(async move { sandbox::handle_create(&iii, &docker, &config, payload).await })
        },
    );
}

fn register_sandbox_get(iii: &Arc<III>) {
    let iii_clone = iii.clone();
    let _fn_ref = iii.register_function(
        RegisterFunctionMessage {
            id: "sandbox::get".to_string(),
            description: Some("Get sandbox details by ID".to_string()),
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
                    "image": { "type": "string" },
                    "status": { "type": "string" },
                    "created_at": { "type": "integer" },
                    "expires_at": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: serde_json::Value| -> Pin<
            Box<dyn Future<Output = Result<serde_json::Value, iii_sdk::IIIError>> + Send>,
        > {
            let iii = iii_clone.clone();
            Box::pin(async move { sandbox::handle_get(&iii, payload).await })
        },
    );
}

fn register_sandbox_list(iii: &Arc<III>) {
    let iii_clone = iii.clone();
    let _fn_ref = iii.register_function(
        RegisterFunctionMessage {
            id: "sandbox::list".to_string(),
            description: Some("List all active sandboxes".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {}
            })),
            response_format: Some(json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "image": { "type": "string" },
                        "status": { "type": "string" }
                    }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: serde_json::Value| -> Pin<
            Box<dyn Future<Output = Result<serde_json::Value, iii_sdk::IIIError>> + Send>,
        > {
            let iii = iii_clone.clone();
            Box::pin(async move { sandbox::handle_list(&iii, payload).await })
        },
    );
}

fn register_sandbox_kill(iii: &Arc<III>, docker: Arc<bollard::Docker>) {
    let iii_clone = iii.clone();
    let _fn_ref = iii.register_function(
        RegisterFunctionMessage {
            id: "sandbox::kill".to_string(),
            description: Some("Stop and remove a sandbox container".to_string()),
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
                    "id": { "type": "string" },
                    "status": { "type": "string" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: serde_json::Value| -> Pin<
            Box<dyn Future<Output = Result<serde_json::Value, iii_sdk::IIIError>> + Send>,
        > {
            let iii = iii_clone.clone();
            let docker = docker.clone();
            Box::pin(async move { sandbox::handle_kill(&iii, &docker, payload).await })
        },
    );
}

fn register_exec_run(
    iii: &Arc<III>,
    docker: Arc<bollard::Docker>,
    config: Arc<SandboxWorkerConfig>,
) {
    let iii_clone = iii.clone();
    let _fn_ref = iii.register_function(
        RegisterFunctionMessage {
            id: "exec::run".to_string(),
            description: Some("Execute a shell command in a sandbox".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Sandbox ID" },
                    "command": { "type": "string", "description": "Shell command to execute" },
                    "timeout_ms": { "type": "integer", "description": "Timeout in milliseconds" }
                },
                "required": ["id", "command"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "exit_code": { "type": "integer" },
                    "stdout": { "type": "string" },
                    "stderr": { "type": "string" },
                    "duration_ms": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: serde_json::Value| -> Pin<
            Box<dyn Future<Output = Result<serde_json::Value, iii_sdk::IIIError>> + Send>,
        > {
            let iii = iii_clone.clone();
            let docker = docker.clone();
            let config = config.clone();
            Box::pin(async move { exec::handle_run(&iii, &docker, &config, payload).await })
        },
    );
}

fn register_exec_code(
    iii: &Arc<III>,
    docker: Arc<bollard::Docker>,
    config: Arc<SandboxWorkerConfig>,
) {
    let iii_clone = iii.clone();
    let _fn_ref = iii.register_function(
        RegisterFunctionMessage {
            id: "exec::code".to_string(),
            description: Some("Write and execute code in a sandbox".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Sandbox ID" },
                    "code": { "type": "string", "description": "Source code to execute" },
                    "language": { "type": "string", "enum": ["python", "javascript", "bash"], "description": "Programming language" },
                    "timeout_ms": { "type": "integer", "description": "Timeout in milliseconds" }
                },
                "required": ["id", "code"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "exit_code": { "type": "integer" },
                    "stdout": { "type": "string" },
                    "stderr": { "type": "string" },
                    "duration_ms": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: serde_json::Value| -> Pin<Box<dyn Future<Output = Result<serde_json::Value, iii_sdk::IIIError>> + Send>> {
            let iii = iii_clone.clone();
            let docker = docker.clone();
            let config = config.clone();
            Box::pin(async move {
                exec::handle_code(&iii, &docker, &config, payload).await
            })
        },
    );
}

fn register_fs_read(
    iii: &Arc<III>,
    docker: Arc<bollard::Docker>,
    config: Arc<SandboxWorkerConfig>,
) {
    let iii_clone = iii.clone();
    let _fn_ref = iii.register_function(
        RegisterFunctionMessage {
            id: "fs::read".to_string(),
            description: Some("Read a file from a sandbox".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Sandbox ID" },
                    "path": { "type": "string", "description": "File path to read" }
                },
                "required": ["id", "path"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: serde_json::Value| -> Pin<
            Box<dyn Future<Output = Result<serde_json::Value, iii_sdk::IIIError>> + Send>,
        > {
            let iii = iii_clone.clone();
            let docker = docker.clone();
            let config = config.clone();
            Box::pin(async move { fs::handle_read(&iii, &docker, &config, payload).await })
        },
    );
}

fn register_fs_write(
    iii: &Arc<III>,
    docker: Arc<bollard::Docker>,
    config: Arc<SandboxWorkerConfig>,
) {
    let iii_clone = iii.clone();
    let _fn_ref = iii.register_function(
        RegisterFunctionMessage {
            id: "fs::write".to_string(),
            description: Some("Write a file to a sandbox".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Sandbox ID" },
                    "path": { "type": "string", "description": "File path to write" },
                    "content": { "type": "string", "description": "File content" }
                },
                "required": ["id", "path", "content"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "size": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: serde_json::Value| -> Pin<
            Box<dyn Future<Output = Result<serde_json::Value, iii_sdk::IIIError>> + Send>,
        > {
            let iii = iii_clone.clone();
            let docker = docker.clone();
            let config = config.clone();
            Box::pin(async move { fs::handle_write(&iii, &docker, &config, payload).await })
        },
    );
}

fn register_fs_list(
    iii: &Arc<III>,
    docker: Arc<bollard::Docker>,
    config: Arc<SandboxWorkerConfig>,
) {
    let iii_clone = iii.clone();
    let _fn_ref = iii.register_function(
        RegisterFunctionMessage {
            id: "fs::list".to_string(),
            description: Some("List files in a sandbox directory".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Sandbox ID" },
                    "path": { "type": "string", "description": "Directory path to list" }
                },
                "required": ["id"]
            })),
            response_format: Some(json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "path": { "type": "string" },
                        "size": { "type": "integer" },
                        "is_directory": { "type": "boolean" }
                    }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: serde_json::Value| -> Pin<
            Box<dyn Future<Output = Result<serde_json::Value, iii_sdk::IIIError>> + Send>,
        > {
            let iii = iii_clone.clone();
            let docker = docker.clone();
            let config = config.clone();
            Box::pin(async move { fs::handle_list(&iii, &docker, &config, payload).await })
        },
    );
}
