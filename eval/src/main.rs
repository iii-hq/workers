use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, RegisterTriggerInput};
use serde_json::json;
use std::sync::Arc;

mod config;
mod functions;
mod manifest;

#[derive(Parser, Debug)]
#[command(name = "iii-eval", about = "III engine OTel-native evaluation worker")]
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

    let eval_config = match config::load_config(&cli.config) {
        Ok(c) => {
            tracing::info!(
                retention_hours = c.retention_hours,
                drift_threshold = c.drift_threshold,
                max_spans = c.max_spans_per_function,
                "loaded config from {}",
                cli.config
            );
            c
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::EvalConfig::default()
        }
    };

    let config = Arc::new(eval_config);

    tracing::info!(url = %cli.url, "connecting to III engine");

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );

    let iii_arc = Arc::new(iii);
    functions::register_all(&iii_arc, &config);

    let cron_expression = config.cron_drift_check.clone();

    // Aggregate trigger registration errors and fail startup if ANY of them
    // failed. Continuing on warnings hides a non-functional worker (no
    // ingest/drift/analyze pipeline) behind a "ready" log line.
    let mut trigger_errors: Vec<String> = Vec::new();
    let triggers = [
        (
            "cron",
            "eval::drift",
            json!({ "expression": cron_expression }),
        ),
        (
            "subscribe",
            "eval::ingest",
            json!({ "topic": "telemetry.spans" }),
        ),
        (
            "http",
            "eval::analyze_traces",
            json!({ "api_path": "eval/analyze", "http_method": "POST" }),
        ),
    ];
    for (ttype, fn_id, cfg) in triggers {
        match iii_arc.register_trigger(RegisterTriggerInput {
            trigger_type: ttype.to_string(),
            function_id: fn_id.to_string(),
            config: cfg,
            metadata: None,
        }) {
            Ok(_) => tracing::info!(kind = ttype, function = fn_id, "trigger registered"),
            Err(e) => {
                tracing::error!(error = %e, kind = ttype, function = fn_id, "trigger registration failed");
                trigger_errors.push(format!("{ttype}:{fn_id}: {e}"));
            }
        }
    }
    if !trigger_errors.is_empty() {
        anyhow::bail!(
            "iii-eval startup aborted — {} trigger registration(s) failed: {}",
            trigger_errors.len(),
            trigger_errors.join(", ")
        );
    }

    tracing::info!("iii-eval worker ready, waiting for invocations");

    tokio::signal::ctrl_c().await?;

    tracing::info!("iii-eval shutting down");
    iii_arc.shutdown_async().await;

    Ok(())
}
