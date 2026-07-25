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
    about = "Durable cross-session agent memory: named banks, always-injected rules, auto-extracted memories, hybrid recall."
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
        name: "memory".to_string(),
        os: std::env::consts::OS.to_string(),
        pid: Some(std::process::id()),
        telemetry: None,
        ..WorkerMetadata::default()
    }
}

/// Best-effort binding: in standalone deployments the harness's trigger
/// types may not exist (yet); a failed binding must not prevent boot.
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
        namespace: iii.namespace(),
    });
    match res {
        Ok(_) => tracing::info!(trigger_type, function_id, "trigger binding requested"),
        Err(e) => {
            tracing::warn!(trigger_type, function_id, error = %e, "trigger binding failed (sibling absent?)")
        }
    }
}

/// The bindings this worker needs, as (trigger_type, function_id, config).
fn required_bindings() -> Vec<(&'static str, &'static str, serde_json::Value)> {
    vec![
        (
            "harness::hook::pre-generate",
            hooks::PRE_GENERATE_FN,
            json!({ "priority": 100, "timeout_ms": 3_000, "on_error": "fail_open" }),
        ),
        (
            "harness::turn-completed",
            hooks::TURN_COMPLETED_FN,
            json!({}),
        ),
        ("session::deleted", hooks::SESSION_DELETED_FN, json!({})),
        (
            "durable:subscriber",
            hooks::EXTRACT_JOB_FN,
            json!({ "queue": hooks::EXTRACTION_QUEUE, "max_retries": 3, "backoff_ms": 2_000 }),
        ),
    ]
}

/// Ensure the extraction queue exists. Trigger parking covers the binds
/// (recoverable triggers, iii #1962), but `queue::define` is an RPC the
/// queue worker must be up to serve — enqueue rejects an undefined queue —
/// so poll until the (idempotent) definition lands.
fn ensure_extraction_queue(iii: Arc<IIIClient>) {
    tokio::spawn(async move {
        loop {
            let defined = iii
                .trigger(iii_sdk::protocol::TriggerRequest {
                    function_id: "queue::define".into(),
                    payload: json!({ "queue": hooks::EXTRACTION_QUEUE, "config": {} }),
                    action: None,
                    timeout_ms: Some(5_000),
                })
                .await
                .is_ok();
            if defined {
                tracing::info!("memory extraction queue defined");
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        }
    });
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

    // Bind the harness/session/queue seams (injection fail-OPEN — a
    // memory failure must never block a turn — plus turn-completed
    // extraction, session GC, and the durable extraction consumer).
    // One shot: if an owning sibling boots after this worker, the engine
    // parks the binding and activates it when the trigger type registers
    // (recoverable triggers, iii #1962).
    for (trigger_type, function_id, config) in required_bindings() {
        bind_best_effort(&iii, trigger_type, function_id, config);
    }
    ensure_extraction_queue(iii.clone());

    // Catch-up vector backfill for memories saved while embeddings were
    // unavailable (delayed so the router and providers finish booting).
    memory::embed_client::backfill_all(deps.clone());

    // Reuse the http worker: every public function doubles as a REST
    // route (`POST /memory/...`). Best-effort — without the http worker
    // the bus surface still serves everything.
    for spec in functions::catalog() {
        let api_path = spec.function_id.replace("::", "/");
        match iii.register_trigger(RegisterTriggerInput {
            trigger_type: "http".to_string(),
            function_id: spec.function_id.to_string(),
            config: json!({ "api_path": api_path, "http_method": "POST" }),
            metadata: None,
            namespace: iii.namespace(),
        }) {
            Ok(_) => tracing::debug!(
                function_id = spec.function_id,
                api_path,
                "http trigger registered"
            ),
            Err(e) => {
                tracing::debug!(error = %e, function_id = spec.function_id, "http trigger registration failed")
            }
        }
    }

    // LAST: bind the configuration-change trigger so its handler closes
    // over the fully-built cells.
    configuration::register_config_trigger(&iii, cell, store_cell)
        .context("registering the configuration change trigger")?;

    tracing::info!(
        "memory ready: 14 memory::* functions + 2 custom trigger types + REST via http worker"
    );

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
            "Internal: harness pre-generate hook — injects the session bank's rules into the \
             system prompt and recalled memories as one appended message. Never denies.",
        )
        // Deliberately NOT trace_hidden: seeing memory feed each turn (which
        // bank, which memories) in the trace timeline is a product requirement,
        // not plumbing noise.
        .metadata(json!({ "internal": true })),
    );

    let d = deps.clone();
    iii.register_function(
        hooks::TURN_COMPLETED_FN,
        RegisterFunction::new_async(move |input: hooks::TurnCompletedInput| {
            let d = d.clone();
            async move { hooks::turn_completed(&d, input).await }
        })
        .description(
            "Internal: harness turn-completed handler — enqueues one durable extraction job \
             (a single router::complete call) for the finished turn; inline fallback without \
             a queue.",
        )
        .metadata(json!({ "internal": true })),
    );

    let d = deps.clone();
    iii.register_function(
        hooks::EXTRACT_JOB_FN,
        RegisterFunction::new_async(move |input: hooks::ExtractJobInput| {
            let d = d.clone();
            async move { hooks::extract_job(&d, input).await }
        })
        .description(
            "Internal: queue-delivered extraction job — errors propagate so the queue retries \
             and dead-letters instead of losing the pass.",
        )
        .metadata(json!({ "internal": true })),
    );

    let d = deps.clone();
    iii.register_function(
        hooks::SESSION_DELETED_FN,
        RegisterFunction::new_async(move |input: hooks::SessionDeletedInput| {
            let d = d.clone();
            async move { hooks::session_deleted(&d, input).await }
        })
        .description("Internal: session::deleted handler — drops the session's extraction cursor.")
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );
}
