use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, RegisterTriggerInput};
use serde_json::json;
use std::sync::Arc;

mod config;
mod functions;
mod manifest;
mod state;

#[derive(Parser, Debug)]
#[command(name = "iii-experiment", about = "III engine generic optimization loop worker")]
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

    let experiment_config = match config::load_config(&cli.config) {
        Ok(c) => {
            tracing::info!(
                default_budget = c.default_budget,
                max_budget = c.max_budget,
                timeout_per_run_ms = c.timeout_per_run_ms,
                "loaded config from {}",
                cli.config
            );
            c
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::ExperimentConfig::default()
        }
    };

    let config = Arc::new(experiment_config);

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

    let triggers = [
        ("experiment::create", "experiment/create", "POST"),
        ("experiment::propose", "experiment/propose", "POST"),
        ("experiment::run", "experiment/run", "POST"),
        ("experiment::decide", "experiment/decide", "POST"),
        ("experiment::loop", "experiment/loop", "POST"),
        ("experiment::status", "experiment/status", "POST"),
        ("experiment::stop", "experiment/stop", "POST"),
    ];

    for (function_id, api_path, http_method) in &triggers {
        match iii_arc.register_trigger(RegisterTriggerInput {
            trigger_type: "http".to_string(),
            function_id: function_id.to_string(),
            config: json!({
                "api_path": api_path,
                "http_method": http_method,
            }),
            metadata: None,
        }) {
            Ok(_) => tracing::info!(function_id, api_path, "http trigger registered"),
            Err(e) => tracing::warn!(error = %e, function_id, "failed to register http trigger"),
        }
    }

    tracing::info!("iii-experiment registered 7 functions and 7 http triggers, waiting for invocations");

    tokio::signal::ctrl_c().await?;

    tracing::info!("iii-experiment shutting down");
    iii_arc.shutdown_async().await;

    Ok(())
}
