//! `mcp` binary entry. Connects to the iii engine, registers the
//! [`mcp::handler`](mcp::functions::FUNCTION_ID) function plus an HTTP
//! trigger at the configured `api_path`, then sleeps on Ctrl+C.
//!
//! All real logic lives under [`mcp`]; tests under `tests/` reuse the
//! same modules to drive the bridge in-process.

use anyhow::Result;
use clap::Parser;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use serde_json::json;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

use mcp::{config, functions, manifest};

use crate::config::McpConfig;

#[derive(Parser, Debug)]
#[command(
    name = "mcp",
    about = "MCP 2025-06-18 Streamable HTTP bridge for the iii engine."
)]
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
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    if cli.manifest {
        let m = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m).unwrap());
        return Ok(());
    }

    let cfg = match config::load_config(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            McpConfig::default()
        }
    };
    let cfg = Arc::new(cfg);

    let iii = register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "mcp".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    functions::register_all(&iii, &cfg);

    let api_path = cfg.api_path.clone();
    match iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: functions::FUNCTION_ID.to_string(),
        config: json!({
            "api_path": api_path,
            "http_method": "POST",
        }),
        metadata: None,
    }) {
        Ok(_) => tracing::info!(
            function_id = functions::FUNCTION_ID,
            api_path = %api_path,
            "http trigger registered"
        ),
        Err(e) => tracing::warn!(
            error = %e,
            function_id = functions::FUNCTION_ID,
            api_path = %api_path,
            "failed to register http trigger"
        ),
    }

    tracing::info!("mcp ready, waiting for invocations");
    tokio::signal::ctrl_c().await?;
    tracing::info!("mcp shutting down");
    iii.shutdown_async().await;
    Ok(())
}
