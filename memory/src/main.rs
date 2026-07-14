//! `memory` binary entry.
//!
//! Boot sequence:
//!   1. Parse CLI. An optional `--config` YAML file is only a SEED for the
//!      first registration; the authoritative config lives in the
//!      `configuration` worker.
//!   2. Connect to the local iii engine over WebSocket.
//!   3. Register the config schema (+ seed) and fetch the authoritative
//!      value. `configuration` is a required boot dependency.
//!   4. Open the store. An unwritable data_dir is BOOT-FATAL: a memory
//!      worker that silently runs in RAM and prints "persisted" is the
//!      worst failure mode a memory product can have.
//!   5. Register the two custom trigger types, then the `memory::*`
//!      functions and the internal hook handlers.
//!   6. Bind the harness seams best-effort (`harness::hook::pre-generate`
//!      fail-OPEN — memory must never block a turn — and
//!      `harness::turn-completed` for extraction). In a deployment without
//!      the harness the worker still boots and serves its RPCs.
//!   7. Bind the `configuration` change trigger LAST.
//!   8. Sleep on Ctrl+C, then `shutdown_async` cleanly. There is no
//!      shutdown flush: every write was already fsynced at commit time.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, IIIClient, InitOptions, RegisterFunction};
use serde_json::json;
use tokio::sync::RwLock;

use memory::configuration::{self, ConfigCell, StoreCell};
use memory::deps::Deps;
use memory::events::{self, Emitter};
use memory::store::Store;
use memory::{config, functions, hooks, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "memory",
    about = "Durable cross-session agent memory: named banks, always-injected blocks, auto-extracted facts, hybrid recall."
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
        name: "memory".to_string(),
        os: std::env::consts::OS.to_string(),
        pid: Some(std::process::id()),
        telemetry: None,
        ..WorkerMetadata::default()
    }
}

/// Best-effort binding: in standalone deployments the harness's trigger
/// types may not exist (yet); a failed binding must not prevent boot.
/// Restart the worker after the sibling appears to re-bind.
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

    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering memory configuration schema")?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading memory configuration")?;

    // Boot-fatal on an unwritable data_dir: never silently run in RAM.
    let store = Store::open(cfg.resolved_data_dir())
        .map_err(|e| anyhow::anyhow!("opening the memory store: {e}"))?;
    let store_cell: StoreCell = Arc::new(RwLock::new(Arc::new(store)));

    // Trigger types first: the handlers capture the subscriber sets.
    let sets = events::register_trigger_types(&iii);
    let emitter = Arc::new(Emitter::new(sets, iii.clone()));

    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg.clone())));
    let deps = Arc::new(Deps {
        iii: iii.clone(),
        store: store_cell.clone(),
        config: cell.clone(),
        emitter,
    });

    functions::register_all(&iii, &deps);
    register_hook_functions(&iii, &deps);

    // Injection: fail-OPEN and generously time-bounded — a memory failure
    // must never block or fail a turn.
    bind_best_effort(
        &iii,
        "harness::hook::pre-generate",
        hooks::PRE_GENERATE_FN,
        json!({ "priority": 100, "timeout_ms": 3_000, "on_error": "fail_open" }),
    );
    // Extraction: async observation of completed turns.
    bind_best_effort(
        &iii,
        "harness::turn-completed",
        hooks::TURN_COMPLETED_FN,
        json!({}),
    );

    // LAST: bind the configuration-change trigger so its handler closes
    // over the fully-built cells.
    configuration::register_config_trigger(&iii, cell, store_cell)
        .context("registering the configuration change trigger")?;

    tracing::info!("memory ready: 14 memory::* functions + 2 custom trigger types");

    tokio::signal::ctrl_c().await?;
    tracing::info!("memory shutting down (all writes already fsynced)");
    iii.shutdown_async().await;
    Ok(())
}

/// The two internal harness-facing handlers. Registered like every other
/// function so hook invocations show up in traces, but marked internal.
fn register_hook_functions(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    let d = deps.clone();
    iii.register_function(
        hooks::PRE_GENERATE_FN,
        RegisterFunction::new_async(move |input: hooks::PreGenerateInput| {
            let d = d.clone();
            async move { hooks::pre_generate(&d, input).await }
        })
        .description(
            "Internal: harness pre-generate hook — injects the session bank's blocks into the \
             system prompt and recalled facts as one appended message. Never denies.",
        )
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    let d = deps.clone();
    iii.register_function(
        hooks::TURN_COMPLETED_FN,
        RegisterFunction::new_async(move |input: hooks::TurnCompletedInput| {
            let d = d.clone();
            async move { hooks::turn_completed(&d, input).await }
        })
        .description(
            "Internal: harness turn-completed handler — spawns one background extraction pass \
             (a single router::complete call) for the finished turn.",
        )
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );
}
