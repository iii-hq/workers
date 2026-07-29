use anyhow::Result;
use clap::Parser;
use editor::{bus::Bus, config, configuration, functions, manifest, ui};
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(
    name = "editor",
    about = "Editor surface for code — diffs, git marks, fuzzy find, conflict-safe saves"
)]
struct Cli {
    /// Optional one-time seed for the configuration worker on first
    /// registration. The configuration worker is the source of truth after
    /// that, so this never overwrites a stored value.
    #[arg(long)]
    config: Option<String>,

    /// Engine WebSocket. `III_URL` is what the worker manager injects when the
    /// worker runs managed — inside a sandbox the engine is on the VM's
    /// gateway, never on the VM's own loopback, so the default is only right
    /// for a worker started by hand on the host.
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
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest::build_manifest()).unwrap()
        );
        return Ok(());
    }

    // A seed that will not parse is a warning, not a failure: the stored
    // value (or the built-in default) still applies.
    let seed = cli
        .config
        .as_deref()
        .and_then(|path| match config::WorkerConfig::from_file(path) {
            Ok(cfg) => Some(cfg),
            Err(e) => {
                tracing::warn!(error = %e, path, "failed to read config seed; ignoring it");
                None
            }
        });

    let iii = register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "editor".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    // The configuration worker is a required boot dependency: without it
    // there is no authoritative config, and guessing one silently would mean
    // running with limits nobody chose.
    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(|e| anyhow::anyhow!("registering editor configuration: {e}"))?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(|e| anyhow::anyhow!("loading editor configuration: {e}"))?;
    let git_timeout_ms = cfg.git_timeout_ms;
    let cfg = configuration::cell(cfg);

    // The bus carries the engine URL because `shell::fs::read` answers with a
    // channel reference that has to be dialled separately.
    let bus = Arc::new(Bus::new(iii.clone(), cli.url.clone(), git_timeout_ms));

    functions::register_all(&iii, &cfg, &bus);
    ui::register(&iii);

    // Bound last, so the handler closes over fully-built state.
    if let Err(e) = configuration::register_config_trigger(&iii, cfg.clone()) {
        tracing::warn!(error = %e, "failed to bind the configuration trigger");
    }

    tracing::info!("editor ready, waiting for invocations");
    tokio::signal::ctrl_c().await?;
    tracing::info!("editor shutting down");
    iii.shutdown_async().await;
    Ok(())
}
