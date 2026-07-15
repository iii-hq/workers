//! memory-consolidate worker: boot, configuration, scheduling.
//!
//! Scheduling is a self-owned timer with CATCH-UP-ON-BOOT semantics rather
//! than a cron-worker binding: the last completed pass lives in the state
//! worker (scope `memory_consolidate`), the loop checks every few minutes
//! whether `interval_hours` have elapsed, and a pass missed while this
//! worker was down therefore runs shortly after boot. No dependency on a
//! scheduler worker being installed; nothing to miss.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
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

/// How often the scheduler re-checks whether a pass is due. Small enough
/// that catch-up-on-boot feels immediate, large enough to cost nothing.
const SCHEDULE_CHECK: Duration = Duration::from_secs(300);
/// First check after boot: give the memory worker and siblings a moment to
/// finish their own boot wave.
const BOOT_GRACE: Duration = Duration::from_secs(45);

fn spawn_scheduler(deps: Arc<Deps>) {
    tokio::spawn(async move {
        tokio::time::sleep(BOOT_GRACE).await;
        loop {
            let cfg = deps.config().await;
            if cfg.enabled {
                let last = functions::last_run_ms(&deps.iii).await;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let interval_ms = cfg.interval_hours.saturating_mul(3_600_000);
                if now.saturating_sub(last) >= interval_ms {
                    tracing::info!(
                        last_run_ms = last,
                        interval_hours = cfg.interval_hours,
                        "scheduled consolidation pass due (catch-up-on-boot semantics)"
                    );
                    match functions::run(&deps, Default::default()).await {
                        Ok(res) => tracing::info!(
                            superseded = res.superseded,
                            dry_run = res.dry_run,
                            "scheduled pass complete"
                        ),
                        Err(e) => {
                            tracing::warn!(error = %e, "scheduled pass failed; retrying next check")
                        }
                    }
                }
            }
            tokio::time::sleep(SCHEDULE_CHECK).await;
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
    });

    functions::register_all(&iii, &deps);
    if let Err(e) = configuration::register_config_trigger(&iii, cell) {
        tracing::warn!(error = %e, "configuration change trigger binding failed; hot reload disabled");
    }

    spawn_scheduler(deps);

    tracing::info!("memory-consolidate ready: run/status + scheduled dedup with catch-up-on-boot");
    tokio::signal::ctrl_c().await?;
    Ok(())
}
