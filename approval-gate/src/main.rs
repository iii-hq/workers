//! `approval-gate` binary entry.
//!
//! Boot sequence:
//!   1. Parse CLI. An optional `--config` YAML file is only a SEED for the
//!      first registration; the authoritative config lives in the
//!      `configuration` worker.
//!   2. Connect to the local iii engine over WebSocket.
//!   3. Register the config schema (+ seed) with the `configuration` worker
//!      and fetch the authoritative, env-expanded value. `configuration`
//!      is a required boot dependency: a failed register/fetch aborts
//!      startup — the gate must run on a known, authoritative policy
//!      surface, never a guessed one.
//!   4. Register the two event functions (`approval::on-pending-created`,
//!      `approval::on-pending-resolved`) — first, because the function
//!      handlers call them through the emitter.
//!   5. Register the 13 `approval::*` functions (each reads the live config
//!      snapshot per call).
//!   6. Observe the harness hook functions (gate + filesystem-access-watch),
//!      `session::on-deleted` and `harness::on-turn-completed`. An observer
//!      of a sibling that is not running (yet) stays pending and becomes
//!      active when the sibling registers its function; the worker boots and
//!      serves its RPCs either way.
//!   7. Observe `configuration::on-updated` LAST so its handler closes over
//!      the fully-built snapshot cell.
//!   8. Sleep on Ctrl+C, then `shutdown_async` cleanly.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, Deliver, InitOptions, Source};
use tokio::sync::RwLock;

use approval_gate::configuration::{self, ConfigCell};
use approval_gate::events::{self, Emitter};
use approval_gate::functions::{self, Deps};
use approval_gate::{config, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "approval-gate",
    about = "Policy and decision surface for human-held function calls."
)]
struct Cli {
    /// Optional seed config.yaml used to populate `initial_value` on the
    /// first registration. The AUTHORITATIVE config is always fetched from
    /// the `configuration` worker afterward; this file only seeds it.
    #[arg(long)]
    config: Option<String>,

    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,

    #[arg(long)]
    manifest: bool,
}

fn worker_metadata() -> WorkerMetadata {
    WorkerMetadata {
        runtime: "rust".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: "approval-gate".to_string(),
        os: std::env::consts::OS.to_string(),
        pid: Some(std::process::id()),
        telemetry: None,
        ..WorkerMetadata::default()
    }
}

/// The siblings' events this worker cleans up after.
const SESSION_ON_DELETED_FN: &str = "session::on-deleted";
const HARNESS_ON_TURN_COMPLETED_FN: &str = "harness::on-turn-completed";

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
            metadata: Some(worker_metadata()),
            ..InitOptions::default()
        },
    ));

    // A `--config` file only SEEDS the first registration; a failed parse
    // WARNS and falls through to None (the authoritative value comes from
    // the configuration worker). The seed IS env-expanded (`${VAR}`).
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
        .context("registering approval-gate configuration schema")?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading approval-gate configuration")?;

    // Event functions first: the handlers call them through the emitter.
    events::register_functions(&iii);
    let sink = Arc::new(Emitter::new(iii.clone()));

    // The hot-swappable snapshot shared with every handler.
    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg.clone())));
    let deps = Arc::new(Deps {
        iii: iii.clone(),
        sink,
        config: cell.clone(),
    });

    functions::register_all(&iii, &deps);

    // Observers live as long as the worker's connection; dropping a handle
    // does not dispose it.
    configuration::observe_hook(&iii);
    configuration::observe_filesystem_access_watch_hook(&iii);
    iii.observe(
        Source::function(SESSION_ON_DELETED_FN).deliver(Deliver::Output),
        functions::ON_SESSION_DELETED_ID,
    );
    iii.observe(
        Source::function(HARNESS_ON_TURN_COMPLETED_FN).deliver(Deliver::Output),
        functions::ON_TURN_COMPLETED_ID,
    );

    // LAST: observe configuration changes so the handler closes over the
    // snapshot cell.
    configuration::observe_config_changes(&iii, cell);

    tracing::info!("approval-gate ready: 13 approval::* functions + 2 event functions");

    tokio::signal::ctrl_c().await?;
    tracing::info!("approval-gate shutting down");
    iii.shutdown_async().await;
    Ok(())
}
