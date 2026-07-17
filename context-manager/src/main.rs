//! `context-manager` binary entry.
//!
//! Boot sequence:
//!   1. Parse CLI. An optional `--config` YAML file is only a SEED for the
//!      first registration; the authoritative config lives in the
//!      `configuration` worker.
//!   2. Connect to the local iii engine over WebSocket.
//!   3. Register the config schema (+ seed) with the `configuration`
//!      worker and fetch the authoritative, env-expanded value.
//!      `configuration` is a required boot dependency: a failed
//!      register/fetch aborts startup.
//!   4. Wire production adapters behind the ports: model limits via
//!      `router::models::budget` and the summariser via `router::chat`
//!      (both soft — only compaction degrades without `llm-router`), plus
//!      compaction leases on the local filesystem (`lease_dir`). The
//!      summariser reads `summarizer_timeout_ms` from the live snapshot per
//!      call; the lease store is rebuilt + swapped on a `lease_dir` change.
//!   5. Register the four `context::*` functions, then bind a
//!      `configuration` trigger that hot-reloads every field (snapshot swap,
//!      lease-store rebuild) — nothing requires a restart.
//!   6. Sleep on Ctrl+C, then `shutdown_async` cleanly.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use tokio::sync::RwLock;

use context_manager::adapters::cache::CachingModelResolver;
use context_manager::adapters::fs_lease::FsLeaseStore;
use context_manager::adapters::router::{RouterModelResolver, RouterSummarizer};
use context_manager::configuration::{self, ConfigCell};
use context_manager::ports::{lease_cell, Deps, SystemClock};
use context_manager::{config, functions, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "context-manager",
    about = "Model-ready context assembly: token counting, pruning, and history compaction."
)]
struct Cli {
    /// Optional seed config.yaml used to populate `initial_value` on the
    /// first registration. The AUTHORITATIVE config is always fetched from
    /// the `configuration` worker afterward; this file only seeds it.
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

    // Connect first so the configuration round-trip below runs over a live
    // connection.
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

    // A `--config` file only SEEDS the first registration; a failed parse
    // WARNS and falls through to None (the authoritative value comes from the
    // configuration worker). The seed IS env-expanded (`${VAR}`).
    let seed = match cli.config.as_deref() {
        Some(path) => match config::WorkerConfig::from_file(path) {
            Ok(c) => {
                tracing::info!(path = %path, "loaded seed config for initial registration");
                Some(c)
            }
            Err(e) => {
                tracing::warn!(
                    path = %path,
                    error = %e,
                    "failed to load seed config; relying on the stored configuration entry"
                );
                None
            }
        },
        None => None,
    };

    // Register the schema (+ optional seed) and fetch the authoritative,
    // env-expanded config. `configuration` is a required boot dependency.
    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering context-manager configuration schema")?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading context-manager configuration")?;

    // The hot-swappable snapshot shared with every handler.
    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg.clone())));

    // The summariser reads its timeout from the live snapshot per call, so a
    // summarizer_timeout_ms change hot-applies with no rebuild.
    let summarizer = Arc::new(RouterSummarizer::new(iii.clone(), cell.clone()));

    // The lease store is the one structural adapter: rebuilt + swapped on a
    // lease_dir change (see configuration::register_config_trigger). A lease
    // dir we can't create is boot-fatal — leasing must never silently no-op.
    let leases = lease_cell(Arc::new(
        FsLeaseStore::new(cfg.resolved_lease_dir())
            .map_err(|e| anyhow::anyhow!("opening the compaction lease directory: {e}"))?,
    ));

    // Budget lookups are TTL-cached and flushed on router::models::changed;
    // the decorator drops into Deps.resolver with no handler changes.
    let clock: Arc<SystemClock> = Arc::new(SystemClock);
    let resolver = Arc::new(CachingModelResolver::new(
        Arc::new(RouterModelResolver::new(iii.clone())),
        clock.clone(),
    ));

    let deps = Arc::new(Deps {
        config: cell.clone(),
        resolver: resolver.clone(),
        summarizer,
        leases: leases.clone(),
        clock,
    });

    functions::register_all(&iii, &deps);
    context_manager::adapters::cache::register_models_changed_flush(&iii, resolver);

    // LAST: bind the configuration-change trigger so its handler closes over
    // the snapshot cell + the lease cell it rebuilds on a lease_dir change.
    configuration::register_config_trigger(&iii, cell, leases)
        .context("registering the configuration change trigger")?;

    tracing::info!("context-manager ready: 4 context::* functions");

    tokio::signal::ctrl_c().await?;
    tracing::info!("context-manager shutting down");
    iii.shutdown_async().await;
    Ok(())
}
