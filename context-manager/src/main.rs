//! `context-manager` binary entry.
//!
//! Boot sequence:
//!   1. Parse CLI / load YAML config (a missing file falls back to
//!      defaults; a file that exists but doesn't parse is fatal — a
//!      typo'd budget knob must never silently run the defaults).
//!   2. Connect to the local iii engine over WebSocket.
//!   3. Wire production adapters behind the ports: model limits via
//!      `router::models::get`, the summariser via `router::chat`, and
//!      compaction leases via `state::*` (all soft — the worker runs
//!      without `llm-router`; only compaction degrades).
//!   4. Register the four `context::*` functions.
//!   5. Sleep on Ctrl+C, then `shutdown_async` cleanly.

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, WorkerMetadata};

use context_manager::adapters::router::{RouterModelResolver, RouterSummarizer};
use context_manager::adapters::state_lease::IiiLeaseStore;
use context_manager::ports::{Deps, SystemClock};
use context_manager::{config, functions, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "context-manager",
    about = "Model-ready context assembly: token counting, pruning, and history compaction."
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
        Ok(c) => c,
        Err(e)
            if e.downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound) =>
        {
            tracing::warn!(path = %cli.config, "config file not found, using defaults");
            config::WorkerConfig::default()
        }
        Err(e) => {
            return Err(e.context(format!("invalid config {} — refusing to start", cli.config)));
        }
    };

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "context-manager".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

    let deps = Arc::new(Deps {
        resolver: Arc::new(RouterModelResolver::new(iii.clone())),
        summarizer: Arc::new(RouterSummarizer::new(
            iii.clone(),
            cfg.summarizer_timeout_ms,
        )),
        leases: Arc::new(IiiLeaseStore::new(iii.clone())),
        clock: Arc::new(SystemClock),
        config: cfg,
    });

    functions::register_all(&iii, &deps);

    tracing::info!("context-manager ready: 4 context::* functions");

    tokio::signal::ctrl_c().await?;
    tracing::info!("context-manager shutting down");
    iii.shutdown_async().await;
    Ok(())
}
