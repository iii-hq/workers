//! memory-consolidate worker: boot, configuration, scheduling.
//!
//! Scheduling REUSES the engine's cron trigger infrastructure (the `cron`
//! worker or the built-in `iii-cron` — whichever owns the `cron` type):
//! an hourly heartbeat binds `memory-consolidate::on-tick`, and the tick
//! decides from the persisted last run (state worker, scope
//! `memory_consolidate`) whether `interval_hours` have elapsed. A slim
//! backstop loop provides CATCH-UP-ON-BOOT — a pass missed while this
//! worker was down runs shortly after boot — and keeps the schedule alive
//! on rigs with no cron trigger owner at all.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, IIIClient, InitOptions, RegisterFunction};
use tokio::sync::RwLock;

use memory_consolidate::functions::{self, ConfigCell, Deps};
use memory_consolidate::{config, configuration, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "memory-consolidate",
    about = "Scheduled hygiene for the memory worker: deterministic dedup, supersede-only, pinned untouchable, catch-up-on-boot."
)]
struct Cli {
    /// Engine WebSocket URL.
    #[arg(long, default_value = "ws://127.0.0.1:49134")]
    url: String,

    /// Optional YAML seed config (only seeds the FIRST configuration
    /// registration; the authoritative value lives in the configuration
    /// worker).
    #[arg(long)]
    config: Option<String>,

    /// Print the registry module manifest and exit.
    #[arg(long)]
    manifest: bool,
}

fn worker_metadata() -> WorkerMetadata {
    WorkerMetadata {
        runtime: "rust".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: "memory-consolidate".to_string(),
        os: std::env::consts::OS.to_string(),
        pid: Some(std::process::id()),
        telemetry: None,
        ..WorkerMetadata::default()
    }
}

/// Backstop cadence: catch-up-on-boot plus a safety net on rigs where no
/// worker owns the `cron` trigger type. The tick's due-check makes each
/// pass idempotent, so heartbeat and backstop can coexist.
const BACKSTOP_CHECK: Duration = Duration::from_secs(1_800);
/// First check after boot: give the memory worker and siblings a moment to
/// finish their own boot wave.
const BOOT_GRACE: Duration = Duration::from_secs(45);
/// Hourly heartbeat on the engine's cron infrastructure; the tick decides
/// whether a pass is actually due.
const CRON_EXPRESSION: &str = "0 0 * * * *";

/// Register the internal tick handler and bind it to the `cron` trigger
/// type (retried: the cron owner may boot after this worker — mirrors the
/// memory worker's binding retry).
fn bind_schedule(iii: Arc<IIIClient>, deps: Arc<Deps>) {
    let deps_for_fn = deps.clone();
    iii.register_function(
        functions::TICK_ID,
        RegisterFunction::new_async(move |_payload: serde_json::Value| {
            let deps = deps_for_fn.clone();
            async move { functions::tick(&deps).await }
        })
        .description(functions::TICK_DESC)
        .metadata(serde_json::json!({ "internal": true, "trace_hidden": true })),
    );

    tokio::spawn(async move {
        for attempt in 1..=20u32 {
            match iii.register_trigger(RegisterTriggerInput {
                trigger_type: "cron".to_string(),
                function_id: functions::TICK_ID.to_string(),
                config: serde_json::json!({ "expression": CRON_EXPRESSION }),
                metadata: None,
            }) {
                Ok(_) => {
                    tracing::info!(expression = CRON_EXPRESSION, "cron heartbeat bound");
                    return;
                }
                Err(e) => {
                    tracing::debug!(attempt, error = %e, "cron binding not accepted yet; retrying");
                }
            }
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
        tracing::warn!(
            "no cron trigger owner accepted the heartbeat binding; the backstop loop keeps the schedule"
        );
    });
}

/// Catch-up-on-boot plus the no-cron safety net: call the same guarded
/// tick the heartbeat uses.
fn spawn_backstop(deps: Arc<Deps>) {
    tokio::spawn(async move {
        tokio::time::sleep(BOOT_GRACE).await;
        loop {
            if let Err(e) = functions::tick(&deps).await {
                tracing::warn!(error = %e, "scheduled pass failed; retrying next check");
            }
            tokio::time::sleep(BACKSTOP_CHECK).await;
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
        .context("registering memory-consolidate configuration schema")?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading memory-consolidate configuration")?;

    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg)));
    let deps = Arc::new(Deps {
        iii: iii.clone(),
        config: cell.clone(),
        run_gate: tokio::sync::Mutex::new(()),
    });

    functions::register_all(&iii, &deps);
    if let Err(e) = configuration::register_config_trigger(&iii, cell) {
        tracing::warn!(error = %e, "configuration change trigger binding failed; hot reload disabled");
    }

    // Reuse the http worker: the public functions double as REST routes
    // (`POST /memory-consolidate/...`). Best-effort, like memory.
    for spec in functions::catalog() {
        let api_path = spec.function_id.replace("::", "/");
        if let Err(e) = iii.register_trigger(RegisterTriggerInput {
            trigger_type: "http".to_string(),
            function_id: spec.function_id.to_string(),
            config: serde_json::json!({ "api_path": api_path, "http_method": "POST" }),
            metadata: None,
        }) {
            tracing::debug!(error = %e, function_id = spec.function_id, "http trigger registration failed");
        }
    }

    bind_schedule(iii.clone(), deps.clone());
    spawn_backstop(deps);

    tracing::info!(
        "memory-consolidate ready: run/status + cron-heartbeat dedup with catch-up-on-boot"
    );
    tokio::signal::ctrl_c().await?;
    Ok(())
}
