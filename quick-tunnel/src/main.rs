use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{register_worker, runtime::WorkerMetadata, InitOptions};
use quick_tunnel::{
    config::{self, Config},
    manager::Manager,
    service,
};
use serde_json::json;

#[derive(Parser)]
#[command(
    name = "quick-tunnel",
    about = "Lease-managed cloudflared Quick Tunnels on iii"
)]
struct Cli {
    /// Optional YAML seed; central configuration is authoritative after first boot.
    #[arg(long)]
    config: Option<String>,
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    /// Explicit isolated development mode, without the configuration worker.
    #[arg(long)]
    local_config: bool,
    /// Print package information without connecting or starting any child.
    #[arg(long)]
    manifest: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let cli = Cli::parse();
    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "name": "quick-tunnel", "version": env!("CARGO_PKG_VERSION"),
                "description": env!("CARGO_PKG_DESCRIPTION"),
                "default_config": Config::default(),
                "prerequisites": ["operator-provided cloudflared with --output json support"],
                "supported_targets": ["aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-unknown-linux-gnu", "x86_64-unknown-linux-musl", "aarch64-unknown-linux-gnu", "armv7-unknown-linux-gnueabihf"]
            }))?
        );
        return Ok(());
    }
    let seed = cli
        .config
        .map(|p| -> Result<Config> {
            let text = std::fs::read_to_string(p).context("reading config seed")?;
            let config: Config = serde_yaml::from_str(&text).context("parsing config seed")?;
            config.validate().map_err(anyhow::Error::msg)?;
            Ok(config)
        })
        .transpose()?;
    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                name: "quick-tunnel".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    let result = run(&cli.local_config, seed, iii.clone()).await;
    tokio::task::spawn_blocking(move || iii.shutdown()).await?;
    result
}

async fn run(
    local_config: &bool,
    seed: Option<Config>,
    iii: Arc<iii_sdk::IIIClient>,
) -> Result<()> {
    let config = if *local_config {
        seed.unwrap_or_default()
    } else {
        let spec = iii_config_client::EntrySpec {
            id: config::config_id(),
            form_id: config::DEFAULT_CONFIG_ID,
            name: "Quick Tunnel",
            description: "Authorized origins and cloudflared lifecycle limits (restart to apply)",
            schema: serde_json::to_value(schemars::schema_for!(Config))?,
            default_value: serde_json::to_value(Config::default())?,
        };
        // `configuration::ensure` seeds the candidate only when nothing is
        // stored yet and preserves an operator/Compose value atomically; on an
        // engine without it the client falls back to the warned legacy path.
        iii_config_client::ensure(&iii, &spec, seed.map(serde_json::to_value).transpose()?)
            .await
            .map_err(anyhow::Error::msg)?;
        let value = iii_config_client::fetch(&iii, spec.id)
            .await
            .map_err(anyhow::Error::msg)?
            .context("missing quick-tunnel config")?;
        serde_json::from_value(value)?
    };
    let targets = config.targets.keys().cloned().collect();
    // Restored leases can spawn children immediately, before we start waiting.
    #[cfg(unix)]
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    #[cfg(unix)]
    let mut int = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let manager = Manager::open(config)?;
    let delivery = service::register(iii.clone(), manager.clone(), targets);
    #[cfg(unix)]
    let signal_result: std::io::Result<()> = {
        tokio::select! { _ = term.recv() => {}, _ = int.recv() => {} }
        Ok(())
    };
    #[cfg(not(unix))]
    let signal_result = tokio::signal::ctrl_c().await;
    manager.shutdown().await;
    delivery.abort();
    let _ = delivery.await;
    signal_result.context("waiting for shutdown signal")
}
