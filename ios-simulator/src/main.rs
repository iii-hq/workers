//! `ios-simulator` binary entry: connect, register configuration + fetch the
//! authoritative value, register the trigger types and functions, start the
//! device watcher and sweep, then sleep until Ctrl+C / SIGTERM.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};

use ios_simulator::bridge::Bridges;
use ios_simulator::config::WorkerConfig;
use ios_simulator::events::Events;
use ios_simulator::sim::Sim;
use ios_simulator::{configuration, functions, manifest, simctl, ui};

const DESCRIPTION: &str = "iOS Simulators on the iii bus (ios-simulator::*).";

#[derive(Parser, Debug)]
#[command(name = "ios-simulator", about = DESCRIPTION)]
struct Cli {
    /// Optional YAML seed used as `initial_value` on first registration.
    #[arg(long)]
    config: Option<String>,
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    #[arg(long)]
    manifest: bool,
}

async fn wait_for_shutdown_signal() -> Result<()> {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        r = tokio::signal::ctrl_c() => r?,
        _ = sigterm.recv() => {}
    }
    Ok(())
}

fn read_seed(path: &str) -> Option<WorkerConfig> {
    let parsed = std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_yaml::from_str::<serde_json::Value>(&s).map_err(|e| e.to_string()))
        .and_then(|v| WorkerConfig::from_json(&v));
    match parsed {
        Ok(cfg) => Some(cfg),
        Err(error) => {
            tracing::warn!(path, %error, "ignoring unreadable seed config");
            None
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
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest::build_manifest())?
        );
        return Ok(());
    }

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "ios-simulator".to_string(),
                os: std::env::consts::OS.to_string(),
                description: Some(DESCRIPTION.to_string()),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

    let seed = cli.config.as_deref().and_then(read_seed);
    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering ios-simulator configuration schema")?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading ios-simulator configuration")?;
    // Without Xcode there is nothing to drive; fail at boot, not per call.
    let developer_dir = simctl::default_developer_dir()
        .await
        .map_err(anyhow::Error::msg)?;
    tracing::info!(
        data_dir = %cfg.data_root().display(),
        developer_dir = %developer_dir.display(),
        share_system_devices = cfg.share_system_devices,
        require_tenant = cfg.require_tenant,
        "loaded ios-simulator configuration"
    );
    let shared = cfg.into_shared();

    // Trigger types before functions, so every emit has its binding tables.
    let events = Events::register(&iii);
    let sim = Sim::new(
        shared.clone(),
        developer_dir,
        Bridges::new(events.clone()),
        events,
    );
    functions::register_all(&iii, &sim);
    iii_console_ui::register_configuration_identity(
        &iii,
        "ios-simulator",
        configuration::config_id(),
    );
    configuration::register_config_trigger(&iii, shared)
        .context("registering configuration change trigger")?;
    ui::register(&iii);

    let watcher = tokio::spawn(sim.clone().watch_loop());
    let sweep = tokio::spawn(sim.clone().sweep_loop());

    tracing::info!("ios-simulator ready");
    wait_for_shutdown_signal().await?;
    tracing::info!("ios-simulator shutting down");
    watcher.abort();
    sweep.abort();
    // Simulators keep running; only our bridges and recordings stop (the
    // recordings are finalized, not lost).
    sim.shutdown_all().await;
    iii.shutdown_async().await;
    Ok(())
}
