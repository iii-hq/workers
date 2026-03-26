use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, RegisterFunctionMessage};
use std::sync::Arc;
use tokio::sync::Mutex;

mod adapter;
mod docker;
mod functions;
mod state;

#[derive(Parser, Debug)]
#[command(name = "iii-launcher", about = "III engine launcher - manages worker containers via Docker")]
struct Cli {
    /// WebSocket URL of the III engine
    #[arg(long, default_value = "ws://127.0.0.1:49134")]
    url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    let adapter: Arc<dyn adapter::RuntimeAdapter> = Arc::new(docker::DockerAdapter::new());
    let launcher_state = Arc::new(Mutex::new(state::LauncherState::load().unwrap_or_default()));

    tracing::info!(url = %cli.url, "connecting to III engine");

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );

    // Register: pull
    let _pull = iii.register_function(
        RegisterFunctionMessage {
            id: "launcher::pull".to_string(),
            description: Some("Pull an OCI image and extract its worker manifest".to_string()),
            request_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "image": { "type": "string", "description": "OCI image reference (e.g. ghcr.io/org/worker:latest)" }
                },
                "required": ["image"]
            })),
            response_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "image": { "type": "string" },
                    "manifest": { "type": "object" },
                    "size_bytes": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::pull::build_pull_handler(adapter.clone()),
    );

    // Register: start
    let _start = iii.register_function(
        RegisterFunctionMessage {
            id: "launcher::start".to_string(),
            description: Some("Start a worker container from a pulled image".to_string()),
            request_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Unique name for this worker instance" },
                    "image": { "type": "string", "description": "OCI image reference" },
                    "engine_url": { "type": "string", "description": "WebSocket URL the worker should connect to" },
                    "auth_token": { "type": "string", "description": "Authentication token for the worker" },
                    "config": { "type": "object", "description": "Worker-specific configuration" }
                },
                "required": ["name", "image", "engine_url"]
            })),
            response_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "container_id": { "type": "string" },
                    "status": { "type": "string" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::start::build_start_handler(adapter.clone(), launcher_state.clone()),
    );

    // Register: stop
    let _stop = iii.register_function(
        RegisterFunctionMessage {
            id: "launcher::stop".to_string(),
            description: Some("Stop and remove a managed worker container".to_string()),
            request_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the worker to stop" }
                },
                "required": ["name"]
            })),
            response_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "stopped": { "type": "boolean" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::stop::build_stop_handler(adapter.clone(), launcher_state.clone()),
    );

    // Register: status
    let _status = iii.register_function(
        RegisterFunctionMessage {
            id: "launcher::status".to_string(),
            description: Some("Get status of all managed worker containers".to_string()),
            request_format: Some(serde_json::json!({
                "type": "object",
                "properties": {}
            })),
            response_format: Some(serde_json::json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "image": { "type": "string" },
                        "runtime": { "type": "string" },
                        "running": { "type": "boolean" },
                        "started_at": { "type": "string" }
                    }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::status::build_status_handler(adapter.clone(), launcher_state.clone()),
    );

    // Register: logs
    let _logs = iii.register_function(
        RegisterFunctionMessage {
            id: "launcher::logs".to_string(),
            description: Some("Get logs from a managed worker container".to_string()),
            request_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the worker" },
                    "follow": { "type": "boolean", "description": "Whether to follow logs (currently returns last 100 lines)" }
                },
                "required": ["name"]
            })),
            response_format: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "logs": { "type": "string" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::logs::build_logs_handler(adapter.clone(), launcher_state.clone()),
    );

    tracing::info!("all launcher functions registered, waiting for invocations");

    tokio::signal::ctrl_c().await?;

    tracing::info!("iii-launcher shutting down");
    iii.shutdown_async().await;

    Ok(())
}
