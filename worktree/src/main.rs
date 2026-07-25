//! `worktree` binary entry.
//!
//! Boot sequence:
//!   1. Parse CLI. An optional `--config` YAML file only SEEDS the first
//!      registration; the authoritative config lives in the `configuration`
//!      worker (Path B).
//!   2. Connect to the local iii engine over WebSocket.
//!   3. Register the config schema (+ seed) and fetch the authoritative
//!      value — a required boot dependency; failure aborts startup.
//!   4. Probe `git --version` (warn below 2.35).
//!   5. Register the six `worktree::*` trigger types FIRST so function
//!      handlers can capture the subscriber sets.
//!   6. Register the eleven public functions.
//!   7. Register the config-change handler + `configuration` trigger, then
//!      bind the prune cron.
//!   8. Sleep on Ctrl+C, then shut down cleanly.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use tokio::sync::RwLock;

use worktree::configuration::{self, ConfigCell, CronSlot};
use worktree::events::{Emitter, IiiDeliverer};
use worktree::functions::{self, Deps};
use worktree::land::test_gate::ShellExecRunner;
use worktree::land::IiiEnqueuer;
use worktree::locks::RepoLocks;
use worktree::state::IiiStateStore;
use worktree::{config, git, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "worktree",
    about = "Git worktree lifecycle for parallel agents, with an automated land queue."
)]
struct Cli {
    /// Optional seed config.yaml used to populate `initial_value` on the
    /// first registration. The authoritative config is always fetched from
    /// the `configuration` worker afterward.
    #[arg(long)]
    config: Option<String>,

    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
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

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "worktree".to_string(),
                os: std::env::consts::OS.to_string(),
                description: Some(
                    "Git worktree lifecycle for parallel agents, with an automated \
                     land queue."
                        .to_string(),
                ),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering worktree configuration schema")?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading worktree configuration")?;

    git::probe_git_version(cfg.git_timeout_ms).await;

    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg.clone())));

    let sets = worktree::events::register_trigger_types(&iii);
    let emitter = Arc::new(Emitter::new(sets, Arc::new(IiiDeliverer::new(iii.clone()))));

    let deps = Arc::new(Deps {
        cell: cell.clone(),
        state: Arc::new(IiiStateStore::new(iii.clone())),
        locks: RepoLocks::default(),
        emitter,
        enqueuer: Arc::new(IiiEnqueuer::new(iii.clone())),
        test_runner: Arc::new(ShellExecRunner::new(iii.clone())),
    });
    functions::register_all(&iii, &deps);

    // Injectable console UI — after the worktree::* functions so the console
    // can attribute the assets. `iii` is already an Arc<IIIClient> (the
    // console-ui crate clones it into its hot-reload watcher task).
    worktree::ui::register(&iii);

    // Best-effort HTTP exposure for the read-only surface. The `http`
    // trigger type may be absent (no http worker installed); a failed
    // registration warns and the worker keeps serving the bus.
    for function_id in [
        "worktree::list",
        "worktree::get",
        "worktree::status",
        "worktree::validate",
    ] {
        let api_path = function_id.replace("::", "/");
        match iii.register_trigger(iii_sdk::protocol::RegisterTriggerInput {
            trigger_type: "http".to_string(),
            function_id: function_id.to_string(),
            config: serde_json::json!({ "api_path": api_path, "http_method": "POST" }),
            metadata: None,
            namespace: iii.namespace(),
        }) {
            Ok(_) => tracing::info!(function_id, api_path, "http trigger registered"),
            Err(e) => {
                tracing::warn!(error = %e, function_id, "http trigger registration failed")
            }
        }
    }

    let cron = CronSlot::default();
    if let Err(e) = configuration::register_config_trigger(&iii, cell, cron.clone()) {
        tracing::warn!(error = %e, "failed to bind the configuration trigger");
    }
    cron.rebind(&iii, &cfg.prune_schedule);

    tracing::info!("worktree ready, waiting for invocations");
    tokio::signal::ctrl_c().await?;
    tracing::info!("worktree shutting down");
    iii.shutdown_async().await;
    Ok(())
}
