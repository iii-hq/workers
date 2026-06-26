//! `harness` binary entry.
//!
//! Boot sequence (configuration.md §4c — Tier 1):
//!   1. Parse CLI. An optional `--config` YAML file only SEEDS the first
//!      registration; the authoritative config lives in the `configuration`
//!      worker.
//!   2. Connect to the local iii engine over WebSocket.
//!   3. Register the config schema (+ seed) and fetch the authoritative,
//!      env-expanded value (a required boot dependency).
//!   4. Register the trigger types the harness emits/owns (turn events + the
//!      five hook points) BEFORE functions so handlers capture subscriber sets.
//!   5. Register the `harness::*` functions.
//!   6. Bind the cron pending-sweep (retain its handle for live re-bind).
//!   7. Seed the function-registry cache and bind `engine::functions-available`
//!      so native exposure reads a live snapshot instead of listing per turn.
//!   8. LAST: bind the configuration-change trigger so its handler closes over
//!      the fully-built snapshot cell + the cron handle.
//!   9. Sleep on Ctrl+C, then `shutdown_async` cleanly.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, WorkerMetadata};
use tokio::sync::RwLock;

use harness::configuration::{self, ConfigCell, TriggerHandles};
use harness::deps::Deps;
use harness::events::TurnEvents;
use harness::hooks::HookRegistry;
use harness::{config, discovery, functions, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "harness",
    about = "Thin durable turn loop wiring session-manager, context-manager, and llm-router."
)]
struct Cli {
    /// Optional seed config.yaml for the first registration only. The
    /// authoritative config is always fetched from the configuration worker.
    #[arg(long)]
    config: Option<String>,

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

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "harness".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

    // A `--config` file only SEEDS the first registration; a failed parse
    // WARNS and falls through to None (the authoritative value comes from the
    // configuration worker).
    let seed = match cli.config.as_deref() {
        Some(path) => match config::WorkerConfig::from_file(path) {
            Ok(c) => {
                tracing::info!(path = %path, "loaded seed config for initial registration");
                Some(c)
            }
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "failed to load seed config; relying on the stored configuration entry");
                None
            }
        },
        None => None,
    };

    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering harness configuration schema")?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading harness configuration")?;

    // Trigger types first: the handlers capture the subscriber sets.
    let events = TurnEvents::register(&iii);
    let hooks = HookRegistry::register(&iii);

    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg.clone())));
    let functions_cell = discovery::new_cell();
    let deps = Arc::new(Deps::new(
        iii.clone(),
        cell.clone(),
        functions_cell.clone(),
        events,
        hooks,
    ));

    functions::register_all(&iii, &deps);

    // Bind the cron pending-sweep; retain the handle so a sweep_expression
    // change re-binds it live.
    let handles = Arc::new(TriggerHandles {
        sweep: std::sync::Mutex::new(configuration::bind_sweep(&iii, &cfg)),
    });

    discovery::seed(&iii, &functions_cell, cfg.dispatch_timeout_ms).await;
    discovery::register_functions_trigger(&iii, functions_cell, cfg.dispatch_timeout_ms);

    // LAST: bind the configuration-change trigger.
    configuration::register_config_trigger(&iii, cell, handles)
        .context("registering the configuration change trigger")?;

    tracing::info!(
        "harness ready: 8 harness::* functions + turn events + hook points + reactive function-registry cache"
    );

    tokio::signal::ctrl_c().await?;
    tracing::info!("harness shutting down");
    iii.shutdown_async().await;
    Ok(())
}
