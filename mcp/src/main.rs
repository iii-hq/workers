//! `mcp` binary entry.
//!
//! Boot sequence:
//!   1. Parse CLI / load YAML config (with fallback to defaults).
//!   2. Connect to the iii engine over WebSocket.
//!   3. Register the `mcp::handler` function and bind it to `POST /mcp`.
//!   4. Sleep on Ctrl+C, then `shutdown_async` cleanly.
//!
//! All real work happens inside `mcp::handler`, dispatched per
//! [`iii_mcp::functions::handler`].

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig};

use iii_mcp::{config, functions, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "mcp",
    about = "Model Context Protocol bridge. Exposes iii functions as MCP tools and the skills worker as MCP resources/prompts over POST /mcp."
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

    let cfg = match config::load_config(&cli.config) {
        Ok(c) => {
            tracing::info!(
                api_path = %c.api_path,
                state_timeout_ms = c.state_timeout_ms,
                "loaded config from {}",
                cli.config
            );
            c
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::McpConfig::default()
        }
    };
    let cfg = Arc::new(cfg);

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );
    let iii = Arc::new(iii);

    functions::register_all(&iii, &cfg);

    tracing::info!(api_path = %cfg.api_path, "mcp ready: POST /{} bound", cfg.api_path);

    tokio::signal::ctrl_c().await?;
    tracing::info!("mcp shutting down");
    iii.shutdown_async().await;
    Ok(())
}
