use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, IIIError, InitOptions, OtelConfig, RegisterFunctionMessage, III};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use iii_conductor::dispatch::{dispatch, merge_run};
use iii_conductor::state::{list_runs, read_run};
use iii_conductor::types::{DispatchInput, MergeResult};

#[derive(Parser, Debug)]
#[command(name = "iii-conductor")]
#[command(version)]
#[command(about = "Multi-agent fan-out + verifier-gated merge worker for iii-engine")]
struct Args {
    #[arg(long, default_value = "ws://localhost:49134")]
    engine_url: String,

    #[arg(long)]
    debug: bool,
}

#[derive(Debug, Deserialize)]
struct RunIdPayload {
    run_id: String,
}

type HandlerFut = Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>;

fn dispatch_handler(iii: Arc<III>) -> impl Fn(Value) -> HandlerFut + Send + Sync + 'static {
    move |payload: Value| {
        let iii = iii.clone();
        Box::pin(async move {
            let input: DispatchInput = serde_json::from_value(payload)
                .map_err(|e| IIIError::Handler(format!("invalid DispatchInput: {e}")))?;
            let (summary, _run) = dispatch(iii, input)
                .await
                .map_err(|e| IIIError::Handler(format!("dispatch failed: {e}")))?;
            serde_json::to_value(summary).map_err(|e| IIIError::Handler(format!("serialize: {e}")))
        })
    }
}

fn status_handler(iii: Arc<III>) -> impl Fn(Value) -> HandlerFut + Send + Sync + 'static {
    move |payload: Value| {
        let iii = iii.clone();
        Box::pin(async move {
            let p: RunIdPayload = serde_json::from_value(payload)
                .map_err(|e| IIIError::Handler(format!("invalid run_id payload: {e}")))?;
            let run = read_run(iii.as_ref(), &p.run_id).await?;
            Ok(serde_json::to_value(run).unwrap_or(Value::Null))
        })
    }
}

fn list_handler(iii: Arc<III>) -> impl Fn(Value) -> HandlerFut + Send + Sync + 'static {
    move |_payload: Value| {
        let iii = iii.clone();
        Box::pin(async move {
            let runs = list_runs(iii.as_ref()).await?;
            Ok(serde_json::to_value(runs).unwrap_or(Value::Null))
        })
    }
}

fn merge_handler(iii: Arc<III>) -> impl Fn(Value) -> HandlerFut + Send + Sync + 'static {
    move |payload: Value| {
        let iii = iii.clone();
        Box::pin(async move {
            let p: RunIdPayload = serde_json::from_value(payload)
                .map_err(|e| IIIError::Handler(format!("invalid run_id payload: {e}")))?;
            let Some(run) = read_run(iii.as_ref(), &p.run_id).await? else {
                let result = MergeResult {
                    ok: false,
                    reason: Some("run not found".to_string()),
                    run_id: p.run_id,
                    winner: None,
                    losers: Vec::new(),
                };
                return Ok(serde_json::to_value(result).unwrap_or(Value::Null));
            };
            let result = merge_run(iii, run).await;
            Ok(serde_json::to_value(result).unwrap_or(Value::Null))
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let filter = if args.debug {
        EnvFilter::new("iii_conductor=debug,iii_sdk=debug")
    } else {
        EnvFilter::new("iii_conductor=info,iii_sdk=warn")
    };
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .init();

    tracing::info!(url = %args.engine_url, "connecting to iii engine");

    let iii = register_worker(
        &args.engine_url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );
    let iii_arc = Arc::new(iii.clone());

    let public_meta = json!({ "public": true });

    let _fn_dispatch = iii.register_function_with(
        RegisterFunctionMessage {
            id: "conductor::dispatch".to_string(),
            description: Some(
                "Fan a task across N agents in parallel. Each gets its own worktree, runs verifier gates, and is recorded under a run_id."
                    .to_string(),
            ),
            request_format: Some(json!({
                "type": "object",
                "required": ["task", "agents", "cwd"],
                "properties": {
                    "task": { "type": "string" },
                    "cwd": { "type": "string" },
                    "timeout_ms": { "type": "integer" },
                    "agents": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["kind"],
                            "properties": {
                                "kind": { "type": "string", "enum": ["claude", "codex", "gemini", "aider", "cursor", "amp", "opencode", "qwen", "remote"] },
                                "bin": { "type": "string" },
                                "args": { "type": "array", "items": { "type": "string" } },
                                "function_id": { "type": "string" },
                                "prompt": { "type": "string" },
                                "worktree": { "type": "boolean" }
                            }
                        }
                    },
                    "gates": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["function_id"],
                            "properties": {
                                "function_id": { "type": "string" },
                                "description": { "type": "string" }
                            }
                        }
                    }
                }
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "run_id": { "type": "string" },
                    "agents": { "type": "integer" },
                    "gates": { "type": "integer" }
                }
            })),
            metadata: Some(public_meta.clone()),
            invocation: None,
        },
        dispatch_handler(iii_arc.clone()),
    );

    let _fn_status = iii.register_function_with(
        RegisterFunctionMessage {
            id: "conductor::status".to_string(),
            description: Some("Read the current state of a dispatch run.".to_string()),
            request_format: Some(json!({
                "type": "object",
                "required": ["run_id"],
                "properties": { "run_id": { "type": "string" } }
            })),
            response_format: None,
            metadata: Some(public_meta.clone()),
            invocation: None,
        },
        status_handler(iii_arc.clone()),
    );

    let _fn_list = iii.register_function_with(
        RegisterFunctionMessage {
            id: "conductor::list".to_string(),
            description: Some("List all dispatch runs.".to_string()),
            request_format: Some(json!({ "type": "object", "properties": {} })),
            response_format: None,
            metadata: Some(public_meta.clone()),
            invocation: None,
        },
        list_handler(iii_arc.clone()),
    );

    let _fn_merge = iii.register_function_with(
        RegisterFunctionMessage {
            id: "conductor::merge".to_string(),
            description: Some(
                "Pick the winning agent for a run (first to pass all gates with a non-empty diff). Cleans up loser worktrees."
                    .to_string(),
            ),
            request_format: Some(json!({
                "type": "object",
                "required": ["run_id"],
                "properties": { "run_id": { "type": "string" } }
            })),
            response_format: None,
            metadata: Some(public_meta),
            invocation: None,
        },
        merge_handler(iii_arc.clone()),
    );

    tracing::info!("conductor functions registered: dispatch, status, list, merge");

    tokio::signal::ctrl_c().await?;
    tracing::info!("shutdown signal received");
    Ok(())
}
