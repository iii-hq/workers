use anyhow::Result;
use clap::Parser;
use iii_sdk::{
    register_worker, InitOptions, OtelConfig, RegisterFunctionMessage, RegisterTriggerInput,
};
use serde_json::json;
use std::sync::Arc;

mod config;
mod exec;
mod functions;
mod jobs;
mod manifest;

#[derive(Parser, Debug)]
#[command(name = "iii-shell", about = "Unix shell execution worker for iii agents")]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, default_value = "ws://127.0.0.1:49134")]
    url: String,

    #[arg(long)]
    manifest: bool,
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

    if cli.manifest {
        let m = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m).unwrap());
        return Ok(());
    }

    let shell_config = match config::load_config(&cli.config) {
        Ok(c) => {
            tracing::info!(
                allowlist_size = c.allowlist.len(),
                denylist_size = c.denylist_patterns.len(),
                max_timeout_ms = c.max_timeout_ms,
                max_concurrent = c.max_concurrent_jobs,
                "loaded config from {}",
                cli.config
            );
            c
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            let mut c = config::ShellConfig::default();
            c.compile_denylist()?;
            c
        }
    };
    let shared = Arc::new(shell_config);

    tracing::info!(url = %cli.url, "connecting to III engine");
    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );

    let _exec_fn = iii.register_function_with(
        RegisterFunctionMessage {
            id: "shell::exec".to_string(),
            description: Some("Execute a command and return full output".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Program name or full command line if 'args' omitted" },
                    "args": { "type": "array", "items": { "type": "string" } },
                    "timeout_ms": { "type": "integer" }
                },
                "required": ["command"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "exit_code": { "type": ["integer", "null"] },
                    "stdout": { "type": "string" },
                    "stderr": { "type": "string" },
                    "duration_ms": { "type": "integer" },
                    "timed_out": { "type": "boolean" },
                    "stdout_truncated": { "type": "boolean" },
                    "stderr_truncated": { "type": "boolean" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::exec::build_handler(shared.clone()),
    );

    let _exec_bg_fn = iii.register_function_with(
        RegisterFunctionMessage {
            id: "shell::exec_bg".to_string(),
            description: Some("Spawn a command in background, return job_id".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "args": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["command"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string" },
                    "argv": { "type": "array" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        functions::exec_bg::build_handler(shared.clone()),
    );

    let _kill_fn = iii.register_function_with(
        RegisterFunctionMessage {
            id: "shell::kill".to_string(),
            description: Some("Kill a running background job".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": { "job_id": { "type": "string" } },
                "required": ["job_id"]
            })),
            response_format: None,
            metadata: None,
            invocation: None,
        },
        functions::kill::build_handler(),
    );

    let _status_fn = iii.register_function_with(
        RegisterFunctionMessage {
            id: "shell::status".to_string(),
            description: Some("Get status of a background job".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": { "job_id": { "type": "string" } },
                "required": ["job_id"]
            })),
            response_format: None,
            metadata: None,
            invocation: None,
        },
        functions::status::build_handler(),
    );

    let _list_fn = iii.register_function_with(
        RegisterFunctionMessage {
            id: "shell::list".to_string(),
            description: Some("List all background jobs".to_string()),
            request_format: Some(json!({ "type": "object", "properties": {} })),
            response_format: None,
            metadata: None,
            invocation: None,
        },
        functions::list::build_handler(shared.clone()),
    );

    for (fn_id, path, method) in [
        ("shell::exec", "shell/exec", "POST"),
        ("shell::exec_bg", "shell/exec_bg", "POST"),
        ("shell::kill", "shell/kill", "POST"),
        ("shell::status", "shell/status", "POST"),
        ("shell::list", "shell/list", "GET"),
    ] {
        if let Err(e) = iii.register_trigger(RegisterTriggerInput {
            trigger_type: "http".to_string(),
            function_id: fn_id.to_string(),
            config: json!({ "api_path": path, "http_method": method }),
            metadata: None,
        }) {
            tracing::warn!(error = %e, "failed to register http trigger for {}", fn_id);
        }
    }

    tracing::info!("iii-shell registered 5 functions and 5 HTTP triggers, ready");

    tokio::signal::ctrl_c().await?;
    tracing::info!("iii-shell shutting down");
    iii.shutdown_async().await;
    Ok(())
}
