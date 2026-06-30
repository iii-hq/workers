//! `workflow` binary entry.
//!
//! Boot sequence:
//!   1. Init tracing.
//!   2. Parse CLI. `--manifest` short-circuits to print manifest and exit.
//!   3. Connect to the local iii engine.
//!   4. Register config schema (seed = None for MVP) and fetch authoritative value.
//!   5. Build ConfigCell + Deps.
//!   6. Register all functions.
//!   7. Bind the cron sweep trigger (retain handle for hot-reload).
//!   8. Set up the harness hooks: react to `engine::workers-available` and bind the
//!      turn-completed / pre-trigger / pre-generate hooks once the harness is up (no poll).
//!   9. Resume-scan: re-enqueue a tick for every non-terminal run (crash recovery).
//!  10. LAST: bind the configuration-change trigger so its handler closes over
//!      the fully-built snapshot cell + the cron handle.
//!  11. Sleep on Ctrl+C, then `shutdown_async` cleanly.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_helpers::observability::OtelConfig;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use tokio::sync::RwLock;

use workflow::configuration::{self, TriggerHandles};
use workflow::functions::{self, ConfigCell};
use workflow::locks::WorkflowLocks;
use workflow::{manifest, state};

#[derive(Parser, Debug)]
#[command(
    name = "workflow",
    about = "Durable workflow orchestrator: fan-out, barrier, and sequential DAG execution."
)]
struct Cli {
    /// Optional seed config for the first registration only.
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
                name: "workflow".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            otel: Some(OtelConfig::default()),
            ..InitOptions::default()
        },
    ));

    // MVP: --config seeding not wired; authoritative value comes from the
    // configuration worker.
    if cli.config.is_some() {
        tracing::warn!("--config seeding not wired in MVP; using stored/default config");
    }

    configuration::register_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("registering workflow configuration schema")?;
    // Don't brick boot on a transient config-worker hiccup: warn and come up
    // inert on defaults, then recover on the next config-change hot-reload (same
    // resilience stance as on_config_change keeping the previous config on a
    // fetch failure). Matches the warn-and-default convention of the other
    // worker binaries in this repo.
    let cfg = match configuration::fetch_config(&iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!(error = %e, "loading workflow configuration failed; using defaults");
            workflow::config::WorkerConfig::default()
        }
    };

    // Wire the state-layer RPC timeout from the authoritative config at boot
    // (kept in sync afterwards by configuration::apply_config on hot-reload).
    state::set_dispatch_timeout_ms(cfg.dispatch_timeout_ms);

    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg.clone())));
    let deps = functions::Deps {
        iii: iii.clone(),
        cfg: cell.clone(),
        locks: WorkflowLocks::default(),
    };

    functions::register_all(&iii, &deps);

    // Bind the cron sweep; retain the handle so a sweep_expression change
    // re-binds it live.
    let handles = Arc::new(TriggerHandles {
        sweep: std::sync::Mutex::new(configuration::bind_sweep(&iii, &cfg)),
    });

    // Bind the three HARNESS-provided hooks (turn-completed → wake, hook::pre-trigger
    // → reply stamping, hook::pre-generate → guidance injection). register_trigger
    // is fire-and-forget — the engine silently drops a bind to a trigger type it does
    // not know yet — so on a cold start where this worker boots before the harness has
    // registered them the binds would be lost. setup_harness_hooks reacts to
    // engine::workers-available (no polling) and binds once the harness's types appear.
    configuration::setup_harness_hooks(&iii).await;

    // Crash recovery: a parked AwaitingNodes run has no enqueued tick.
    // Re-drive each non-terminal run so it can make progress.
    match state::list_runs(&iii).await {
        Ok(runs) => {
            for r in runs.iter().filter(|r| !r.status.is_terminal()) {
                if let Err(e) =
                    workflow::functions::start::enqueue_tick(&iii, &r.run_id, r.step + 1).await
                {
                    tracing::warn!(run_id = %r.run_id, error = %e, "resume-scan: re-enqueue failed");
                }
            }
        }
        Err(e) => tracing::error!(error = %e, "resume-scan: list_runs failed"),
    }

    // LAST: bind the configuration-change trigger.
    configuration::register_config_trigger(&iii, cell, handles)
        .context("registering the configuration change trigger")?;

    tracing::info!("workflow ready");

    tokio::signal::ctrl_c().await?;
    tracing::info!("workflow shutting down");
    iii.shutdown_async().await;
    Ok(())
}
