use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, RegisterFunctionMessage};
use serde_json::json;

mod config;
mod functions;

#[derive(Parser, Debug)]
#[command(name = "iii-introspection", about = "Slim engine introspection worker")]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

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
    let cfg = Arc::new(config::load(&cli.config).unwrap_or_default());

    tracing::info!(url = %cli.url, "connecting to iii engine");

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );
    let iii = Arc::new(iii.clone());

    register_fn(
        &iii,
        "introspection::workers::list",
        "Slim list of connected workers (name, status, function_count). Pass {\"include\":\"full\"} for full graph.",
        json!({
            "type": "object",
            "properties": {
                "include": {"type": "string", "enum": ["slim", "full"], "default": "slim"},
                "filter": {"type": "string", "description": "Substring match on worker name"}
            }
        }),
        {
            let iii = iii.clone();
            move |payload| {
                let iii = iii.clone();
                Box::pin(async move { functions::workers::list(iii, payload).await })
            }
        },
    );

    register_fn(
        &iii,
        "introspection::workers::describe",
        "Full worker detail: function ids, status, kind, function count.",
        json!({
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"]
        }),
        {
            let iii = iii.clone();
            move |payload| {
                let iii = iii.clone();
                Box::pin(async move { functions::workers::describe(iii, payload).await })
            }
        },
    );

    register_fn(
        &iii,
        "introspection::functions::list",
        "Slim list of registered functions (id + description only). Optional ?worker filter.",
        json!({
            "type": "object",
            "properties": {
                "worker": {"type": "string"},
                "filter": {"type": "string"}
            }
        }),
        {
            let iii = iii.clone();
            move |payload| {
                let iii = iii.clone();
                Box::pin(async move { functions::functions_mod::list(iii, payload).await })
            }
        },
    );

    register_fn(
        &iii,
        "introspection::functions::describe",
        "Just-in-time full schema for one function id (request_format + response_format).",
        json!({
            "type": "object",
            "properties": {"id": {"type": "string"}},
            "required": ["id"]
        }),
        {
            let iii = iii.clone();
            move |payload| {
                let iii = iii.clone();
                Box::pin(async move { functions::functions_mod::describe(iii, payload).await })
            }
        },
    );

    register_fn(
        &iii,
        "introspection::stream::subscribe",
        "Snapshot of recent worker/function registration events. Streaming subscriber is wired through the engine pubsub channel introspection.registrations.",
        json!({
            "type": "object",
            "properties": {"since_ms": {"type": "integer"}}
        }),
        {
            let iii = iii.clone();
            move |payload| {
                let iii = iii.clone();
                Box::pin(async move { functions::stream::subscribe(iii, payload).await })
            }
        },
    );

    register_fn(
        &iii,
        "introspection::context::bootstrap",
        "Slim per-session context: connected workers, disabled engine-builtins with activation hints (so the agent stops calling sandbox::* when iii-sandbox is off), root skill index ids only, canonical discovery flow. Call once at session start instead of dumping every SKILL.md body.",
        json!({"type": "object"}),
        {
            let iii = iii.clone();
            move |payload| {
                let iii = iii.clone();
                Box::pin(async move { functions::context::bootstrap(iii, payload).await })
            }
        },
    );

    register_fn(
        &iii,
        "introspection::context::worker_status",
        "Probe one worker: connected vs not_registered, plus activation_hint when it is an engine builtin (iii-sandbox / iii-http / iii-cron) that needs a config block.",
        json!({
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"]
        }),
        {
            let iii = iii.clone();
            move |payload| {
                let iii = iii.clone();
                Box::pin(async move { functions::context::worker_status(iii, payload).await })
            }
        },
    );

    let cfg_q = cfg.clone();
    register_fn(
        &iii,
        "introspection::registry::query",
        "Search the workers.iii.dev registry for capabilities matching a query.",
        json!({
            "type": "object",
            "properties": {
                "q": {"type": "string"},
                "limit": {"type": "integer", "default": 20}
            },
            "required": ["q"]
        }),
        move |payload| {
            let cfg = cfg_q.clone();
            Box::pin(async move { functions::registry::query(cfg, payload).await })
        },
    );

    tracing::info!("iii-introspection registered 6 functions, awaiting calls");

    tokio::signal::ctrl_c().await?;
    Ok(())
}

fn register_fn<F>(
    iii: &Arc<iii_sdk::III>,
    id: &str,
    description: &str,
    request_format: serde_json::Value,
    handler: F,
) where
    F: Fn(
            serde_json::Value,
        )
            -> std::pin::Pin<
                Box<
                    dyn std::future::Future<
                            Output = Result<serde_json::Value, iii_sdk::IIIError>,
                        > + Send,
                >,
            > + Send
        + Sync
        + 'static,
{
    iii.register_function((
        RegisterFunctionMessage {
            id: id.to_string(),
            description: Some(description.to_string()),
            request_format: Some(request_format),
            response_format: None,
            metadata: Some(json!({"mcp.expose": true})),
            invocation: None,
        },
        handler,
    ));
}
