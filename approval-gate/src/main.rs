//! `approval-gate` binary entry.
//!
//! Boot sequence:
//!   1. Parse CLI / load YAML config (a missing file falls back to
//!      defaults; a file that exists but doesn't parse is fatal).
//!   2. Connect to the local iii engine over WebSocket.
//!   3. Register the two custom trigger types (`approval::pending-created`,
//!      `approval::pending-resolved`) — first, because the function
//!      handlers capture the subscriber sets they fan out to.
//!   4. Register the 14 `approval::*` functions.
//!   5. Bind triggers, all best-effort (`harness::hook::pre-trigger`,
//!      `configuration`, `session::deleted`, `harness::turn-completed`,
//!      `cron`) — in a standalone deployment some of these trigger types
//!      don't exist yet; the worker still boots and serves its RPCs.
//!   6. Register the `approval-gate` configuration entry and read the
//!      deployment defaults once (the configuration trigger keeps them
//!      fresh from then on).
//!   7. Sleep on Ctrl+C, then `shutdown_async` cleanly.

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, RegisterTriggerInput, WorkerMetadata, III};
use serde_json::json;

use approval_gate::events::{self, Emitter};
use approval_gate::functions::{self, Deps};
use approval_gate::gate_config::{self, replace, shared_defaults};
use approval_gate::{config, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "approval-gate",
    about = "Policy and decision surface for human-held function calls."
)]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

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
    iii: &Arc<III>,
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
    let cfg = Arc::new(cfg);

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(worker_metadata()),
            ..InitOptions::default()
        },
    ));

    // Trigger types first: the handlers capture the subscriber sets.
    let sets = events::register_trigger_types(&iii);

    let sink = Arc::new(Emitter::new(sets, iii.clone()));
    let defaults = shared_defaults();
    let deps = Arc::new(Deps {
        iii: iii.clone(),
        sink,
        defaults: defaults.clone(),
        cfg: cfg.clone(),
    });

    functions::register_all(&iii, &deps);

    // The gate's own hook binding — installing the worker is installing
    // the hook (approval-gate.md § The approval::gate hook).
    bind_best_effort(
        &iii,
        "harness::hook::pre-trigger",
        "approval::gate",
        json!({
            "functions": cfg.hook.functions,
            "timeout_ms": cfg.hook.timeout_ms,
            "on_error": cfg.hook.on_error,
        }),
    );
    bind_best_effort(
        &iii,
        "configuration",
        "approval::on-config-change",
        json!({
            "configuration_id": gate_config::ENTRY_ID,
            "event_types": ["configuration:registered", "configuration:updated"],
        }),
    );
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
    bind_best_effort(
        &iii,
        "cron",
        "approval::sweep",
        json!({ "expression": cfg.sweep_expression }),
    );

    // Configuration entry: register (no initial_value — operator values
    // survive re-register), then one initial read; the configuration
    // trigger above keeps the defaults fresh reactively.
    if let Err(e) = gate_config::register_entry(&iii).await {
        tracing::info!(error = %e, "configuration worker unavailable; running on built-in defaults");
    }
    replace(&defaults, gate_config::read_defaults(&iii).await);

    tracing::info!("approval-gate ready: 14 approval::* functions + 2 custom trigger types");

    tokio::signal::ctrl_c().await?;
    tracing::info!("approval-gate shutting down");
    iii.shutdown_async().await;
    Ok(())
}
