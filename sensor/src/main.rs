use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, RegisterFunctionMessage};
use std::sync::Arc;

mod analysis;
mod config;
mod functions;
mod manifest;
mod state;

#[derive(Parser, Debug)]
#[command(name = "iii-sensor", about = "III engine code quality sensor")]
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
        let manifest = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&manifest).unwrap());
        return Ok(());
    }

    let sensor_config = match config::load_config(&cli.config) {
        Ok(c) => {
            tracing::info!(
                extensions = ?c.scan_extensions,
                max_file_size_kb = c.max_file_size_kb,
                "loaded config from {}",
                cli.config
            );
            c
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::SensorConfig::default()
        }
    };

    let cfg = Arc::new(sensor_config);

    tracing::info!(url = %cli.url, "connecting to III engine");

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );

    let iii_arc = Arc::new(iii.clone());

    {
        let iii_c = iii_arc.clone();
        let cfg_c = cfg.clone();
        iii.register_function((
            RegisterFunctionMessage {
                id: "sensor::scan".to_string(),
                description: Some(
                    "Scan a directory and compute per-file code quality metrics".to_string(),
                ),
                request_format: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Directory path to scan" },
                        "extensions": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "File extensions to include (defaults to config)"
                        }
                    },
                    "required": ["path"]
                })),
                response_format: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "files": { "type": "array" },
                        "summary": { "type": "object" }
                    }
                })),
                metadata: None,
                invocation: None,
            },
            move |payload: serde_json::Value| {
                let iii_c = iii_c.clone();
                let cfg_c = cfg_c.clone();
                Box::pin(async move { functions::scan::handle(iii_c, cfg_c, payload).await })
                    as std::pin::Pin<
                        Box<
                            dyn std::future::Future<
                                    Output = Result<serde_json::Value, iii_sdk::IIIError>,
                                > + Send,
                        >,
                    >
            },
        ));
    }

    {
        let iii_c = iii_arc.clone();
        let cfg_c = cfg.clone();
        iii.register_function((
            RegisterFunctionMessage {
                id: "sensor::score".to_string(),
                description: Some(
                    "Compute aggregate quality score from scan results".to_string(),
                ),
                request_format: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Directory path to scan and score" },
                        "scan_result": { "type": "object", "description": "Pre-computed scan result" }
                    }
                })),
                response_format: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "score": { "type": "number" },
                        "dimensions": { "type": "object" },
                        "grade": { "type": "string" },
                        "file_count": { "type": "integer" },
                        "timestamp": { "type": "string" }
                    }
                })),
                metadata: None,
                invocation: None,
            },
            move |payload: serde_json::Value| {
                let iii_c = iii_c.clone();
                let cfg_c = cfg_c.clone();
                Box::pin(async move { functions::score::handle(iii_c, cfg_c, payload).await })
                    as std::pin::Pin<
                        Box<dyn std::future::Future<Output = Result<serde_json::Value, iii_sdk::IIIError>> + Send>,
                    >
            },
        ));
    }

    {
        let iii_c = iii_arc.clone();
        let cfg_c = cfg.clone();
        iii.register_function((
            RegisterFunctionMessage {
                id: "sensor::baseline".to_string(),
                description: Some(
                    "Save current quality score as a named baseline snapshot".to_string(),
                ),
                request_format: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Directory path to baseline" },
                        "label": { "type": "string", "description": "Baseline label (default: 'default')" }
                    },
                    "required": ["path"]
                })),
                response_format: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "baseline_id": { "type": "string" },
                        "score": { "type": "number" },
                        "dimensions": { "type": "object" },
                        "timestamp": { "type": "string" },
                        "label": { "type": "string" }
                    }
                })),
                metadata: None,
                invocation: None,
            },
            move |payload: serde_json::Value| {
                let iii_c = iii_c.clone();
                let cfg_c = cfg_c.clone();
                Box::pin(async move { functions::baseline::handle(iii_c, cfg_c, payload).await })
                    as std::pin::Pin<
                        Box<dyn std::future::Future<Output = Result<serde_json::Value, iii_sdk::IIIError>> + Send>,
                    >
            },
        ));
    }

    {
        let iii_c = iii_arc.clone();
        let cfg_c = cfg.clone();
        iii.register_function((
            RegisterFunctionMessage {
                id: "sensor::compare".to_string(),
                description: Some(
                    "Compare current quality against a saved baseline and detect degradation"
                        .to_string(),
                ),
                request_format: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Directory path to compare" },
                        "baseline_id": { "type": "string", "description": "Specific baseline ID" },
                        "label": { "type": "string", "description": "Baseline label to compare against" }
                    },
                    "required": ["path"]
                })),
                response_format: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "degraded": { "type": "boolean" },
                        "overall_delta": { "type": "number" },
                        "dimension_deltas": { "type": "object" },
                        "baseline_score": { "type": "number" },
                        "current_score": { "type": "number" },
                        "degraded_dimensions": { "type": "array" },
                        "timestamp": { "type": "string" }
                    }
                })),
                metadata: None,
                invocation: None,
            },
            move |payload: serde_json::Value| {
                let iii_c = iii_c.clone();
                let cfg_c = cfg_c.clone();
                Box::pin(async move { functions::compare::handle(iii_c, cfg_c, payload).await })
                    as std::pin::Pin<
                        Box<dyn std::future::Future<Output = Result<serde_json::Value, iii_sdk::IIIError>> + Send>,
                    >
            },
        ));
    }

    {
        let iii_c = iii_arc.clone();
        let cfg_c = cfg.clone();
        iii.register_function((
            RegisterFunctionMessage {
                id: "sensor::gate".to_string(),
                description: Some(
                    "CI quality gate — pass/fail based on score thresholds and degradation limits"
                        .to_string(),
                ),
                request_format: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Directory path to gate" },
                        "min_score": { "type": "number", "description": "Minimum passing score (default: 60)" },
                        "max_degradation_pct": { "type": "number", "description": "Max allowed degradation % (default: 10)" }
                    },
                    "required": ["path"]
                })),
                response_format: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "passed": { "type": "boolean" },
                        "score": { "type": "number" },
                        "grade": { "type": "string" },
                        "reason": { "type": "string" },
                        "details": { "type": "object" }
                    }
                })),
                metadata: None,
                invocation: None,
            },
            move |payload: serde_json::Value| {
                let iii_c = iii_c.clone();
                let cfg_c = cfg_c.clone();
                Box::pin(async move { functions::gate::handle(iii_c, cfg_c, payload).await })
                    as std::pin::Pin<
                        Box<dyn std::future::Future<Output = Result<serde_json::Value, iii_sdk::IIIError>> + Send>,
                    >
            },
        ));
    }

    {
        let iii_c = iii_arc.clone();
        let cfg_c = cfg.clone();
        iii.register_function((
            RegisterFunctionMessage {
                id: "sensor::history".to_string(),
                description: Some(
                    "Retrieve historical quality scores and detect trend direction".to_string(),
                ),
                request_format: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Directory path to get history for" },
                        "limit": { "type": "integer", "description": "Max entries to return (default: 20)" }
                    },
                    "required": ["path"]
                })),
                response_format: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "scores": { "type": "array" },
                        "total_entries": { "type": "integer" },
                        "trend": { "type": "string", "enum": ["improving", "stable", "degrading"] }
                    }
                })),
                metadata: None,
                invocation: None,
            },
            move |payload: serde_json::Value| {
                let iii_c = iii_c.clone();
                let cfg_c = cfg_c.clone();
                Box::pin(async move { functions::history::handle(iii_c, cfg_c, payload).await })
                    as std::pin::Pin<
                        Box<dyn std::future::Future<Output = Result<serde_json::Value, iii_sdk::IIIError>> + Send>,
                    >
            },
        ));
    }

    tracing::info!("all sensor functions registered, waiting for invocations");

    tokio::signal::ctrl_c().await?;

    tracing::info!("iii-sensor shutting down");
    iii.shutdown_async().await;

    Ok(())
}
