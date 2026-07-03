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
//!   4. Register the two custom trigger types (`approval::pending-created`,
//!      `approval::pending-resolved`) — first, because the function
//!      handlers capture the subscriber sets they fan out to.
//!   5. Register the 13 `approval::*` functions (each reads the live config
//!      snapshot per call).
//!   6. Bind the fixed gate + grant-watch hooks + the session/turn
//!      triggers, all best-effort (in a standalone deployment some of these
//!      trigger types don't exist yet; the worker still boots and serves
//!      its RPCs).
//!   7. Bind the `configuration` change trigger LAST so its handler closes
//!      over the fully-built snapshot cell.
//!   8. Sleep on Ctrl+C, then `shutdown_async` cleanly.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::json;
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

    #[arg(long, default_value = "ws://127.0.0.1:49134")]
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

/// Best-effort binding: in standalone deployments the harness's and
/// session-manager's trigger types may not exist (yet); a failed binding
/// must not prevent boot. Registration is acked asynchronously — a
/// missing trigger type surfaces later as an SDK-level
/// `trigger_type_not_found` error log, never as an `Err` here. Restart
/// the worker after the sibling appears to re-bind.
fn bind_best_effort(
    iii: &Arc<IIIClient>,
    trigger_type: &str,
    function_id: &str,
    config: serde_json::Value,
) {
    let res = iii.register_trigger(RegisterTriggerInput {
        trigger_type: trigger_type.to_string(),
        function_id: function_id.to_string(),
        config,
        metadata: None,
    });
    match res {
        Ok(_) => tracing::info!(trigger_type, function_id, "trigger binding requested"),
        Err(e) => {
            tracing::warn!(trigger_type, function_id, error = %e, "trigger binding failed (sibling absent?)")
        }
    }
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

    // Trigger types first: the handlers capture the subscriber sets.
    let sets = events::register_trigger_types(&iii);
    let sink = Arc::new(Emitter::new(sets, iii.clone()));

    // The hot-swappable snapshot shared with every handler.
    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg.clone())));
    let deps = Arc::new(Deps {
        iii: iii.clone(),
        sink,
        config: cell.clone(),
    });

    functions::register_all(&iii, &deps);

    configuration::bind_hook(&iii);
    configuration::bind_grant_watch_hook(&iii);

    // These two carry no config and are never re-bound — best-effort only.
    bind_best_effort(
        &iii,
        "session::deleted",
        "approval::on-session-deleted",
        json!({}),
    );
    bind_best_effort(
        &iii,
        "harness::turn-completed",
        "approval::on-turn-completed",
        json!({}),
    );

    // LAST: bind the configuration-change trigger so its handler closes over
    // the snapshot cell.
    configuration::register_config_trigger(&iii, cell)
        .context("registering the configuration change trigger")?;

    tracing::info!("approval-gate ready: 13 approval::* functions + 2 custom trigger types");

    tokio::signal::ctrl_c().await?;
    tracing::info!("approval-gate shutting down");
    iii.shutdown_async().await;
    Ok(())
}
