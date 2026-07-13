//! `browser` binary entry: connect, register configuration + fetch the
//! authoritative value, register the five `browser::*` trigger types and
//! twelve functions, start the idle sweep, then sleep until Ctrl+C.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};

use browser::config::WorkerConfig;
use browser::events::{self, IiiDeliverer};
use browser::session::Sessions;
use browser::{configuration, functions, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "browser",
    about = "Interactive Chromium sessions on the iii bus (browser::*)."
)]
struct Cli {
    /// Optional YAML seed used to populate `initial_value` on first registration.
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
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest::build_manifest()).unwrap()
        );
        return Ok(());
    }

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "browser".to_string(),
                os: std::env::consts::OS.to_string(),
                description: Some(
                    "Interactive Chromium sessions on the iii bus (browser::*).".to_string(),
                ),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

    // Optional YAML seed → initial_value on first registration.
    let seed = cli.config.as_deref().and_then(|path| {
        let contents = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(path, error = %e, "failed to read seed config; ignoring");
                return None;
            }
        };
        match serde_yaml::from_str::<serde_json::Value>(&contents) {
            Ok(v) => match WorkerConfig::from_json(&v) {
                Ok(cfg) => Some(cfg),
                Err(e) => {
                    tracing::warn!(path, error = %e, "failed to parse seed config; ignoring");
                    None
                }
            },
            Err(e) => {
                tracing::warn!(path, error = %e, "failed to parse YAML seed config; ignoring");
                None
            }
        }
    });

    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering browser configuration schema")?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading browser configuration")?;
    tracing::info!(
        headless = cfg.headless,
        max_sessions = cfg.max_sessions,
        console_buffer = cfg.console_buffer,
        "loaded browser configuration"
    );
    let shared = cfg.into_shared();

    // Trigger types before functions, so handlers capture live subscriber sets.
    let sets = events::register_trigger_types(&iii);
    let emitter = Arc::new(events::Emitter::new(
        sets,
        Arc::new(IiiDeliverer::new(iii.clone())),
    ));

    let sessions = Sessions::new(shared.clone(), emitter);
    functions::register_all(&iii, &sessions);

    configuration::register_config_trigger(
        &iii,
        configuration::SharedState {
            config: shared.clone(),
        },
    )
    .context("registering configuration change trigger")?;

    // Idle sweep: stop sessions nobody has touched for idle_stop_ms.
    let sweep_sessions = sessions.clone();
    let sweep = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            sweep_sessions.sweep_idle().await;
        }
    });

    tracing::info!("browser ready: browser::* sessions + console capture + pick");
    tokio::signal::ctrl_c().await?;
    tracing::info!("browser shutting down");
    sweep.abort();
    sessions.stop_all().await;
    iii.shutdown_async().await;
    Ok(())
}
