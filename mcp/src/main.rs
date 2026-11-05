//! `mcp` binary entry. Connects to the iii engine, registers the
//! [`mcp::handler`](mcp::functions::FUNCTION_ID) function, observes
//! `http::on-request` with it for `POST <api_path>`, then sleeps on Ctrl+C.
//!
//! All real logic lives under [`mcp`]; tests under `tests/` reuse the
//! same modules to drive the bridge in-process.

use anyhow::Result;
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

use iii_mcp::{config, functions, manifest};

use crate::config::McpConfig;

#[derive(Parser, Debug)]
#[command(
    name = "mcp",
    about = "MCP 2025-06-18 Streamable HTTP bridge for the iii engine."
)]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
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

    // The route lives as long as the connection; dropping the handle keeps
    // it. Report whether it became active without blocking startup.
    let route = functions::observe_route(&iii, &cfg.api_path);
    let api_path = cfg.api_path.clone();
    tokio::spawn(async move {
        match tokio::time::timeout(Duration::from_secs(30), route.ready()).await {
            Ok(Ok(())) => tracing::info!(
                function_id = functions::FUNCTION_ID,
                api_path = %api_path,
                "http::on-request route active"
            ),
            Ok(Err(e)) => tracing::warn!(
                error = %e,
                function_id = functions::FUNCTION_ID,
                api_path = %api_path,
                "http::on-request route rejected"
            ),
            Err(_) => tracing::warn!(
                function_id = functions::FUNCTION_ID,
                api_path = %api_path,
                "http::on-request route still pending after 30s; is the http worker running?"
            ),
        }
    });

    tracing::info!("mcp ready, waiting for invocations");
    tokio::signal::ctrl_c().await?;
    tracing::info!("mcp shutting down");
    iii.shutdown_async().await;
    Ok(())
}
