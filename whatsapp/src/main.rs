use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{InitOptions, register_worker, runtime::WorkerMetadata};
use serde_json::json;
use whatsapp::{
    config::{self, Config},
    service,
    sessions::SessionManager,
};

#[derive(Parser)]
#[command(name = "whatsapp", version, about)]
struct Cli {
    /// Optional YAML seed; existing central configuration remains authoritative.
    #[arg(long)]
    config: Option<std::path::PathBuf>,
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    /// Use only local configuration (isolated development / interface capture).
    #[arg(long)]
    local_config: bool,
    /// Print package information without connections or storage access.
    #[arg(long, conflicts_with = "catalog")]
    manifest: bool,
    /// Print function schemas without connections or storage access.
    #[arg(long)]
    catalog: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    whatsapp::logging::init();
    let cli = Cli::parse();
    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "name": "whatsapp", "version": env!("CARGO_PKG_VERSION"),
                "description": env!("CARGO_PKG_DESCRIPTION"), "default_config": Config::default(),
                "native_client": {"repository":"https://github.com/oxidezap/whatsapp-rust", "revision":"dd0763c4635b353c4ed52def39d030fb2f375f9a"},
                "runtime": "native binary; no Node, browser or sandbox required",
                "trigger_types": [service::EVENTS]
            }))?
        );
        return Ok(());
    }
    if cli.catalog {
        println!(
            "{}",
            serde_json::to_string_pretty(&whatsapp::registry().catalog())?
        );
        return Ok(());
    }
    let seed = cli
        .config
        .as_ref()
        .map(|path| -> Result<Config> {
            Config::from_yaml(&std::fs::read_to_string(path).context("reading config seed")?)
        })
        .transpose()?;
    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                name: "whatsapp".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    let result = run(cli.local_config, seed, iii.clone()).await;
    tokio::task::spawn_blocking(move || iii.shutdown()).await?;
    result
}

async fn run(local: bool, seed: Option<Config>, iii: Arc<iii_sdk::IIIClient>) -> Result<()> {
    let config: Config = if local {
        seed.unwrap_or_default()
    } else {
        let spec = iii_config_client::EntrySpec {
            id: config::config_id(),
            form_id: config::DEFAULT_CONFIG_ID,
            name: "WhatsApp",
            description: "Native session storage and resource limits (restart to apply)",
            schema: serde_json::to_value(schemars::schema_for!(Config))?,
            default_value: serde_json::to_value(Config::default())?,
        };
        iii_config_client::ensure(&iii, &spec, seed.map(serde_json::to_value).transpose()?)
            .await
            .map_err(anyhow::Error::msg)?;
        serde_json::from_value(
            iii_config_client::fetch(&iii, spec.id)
                .await
                .map_err(anyhow::Error::msg)?
                .context("missing whatsapp configuration")?,
        )?
    };
    config.validate()?;
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let manager = SessionManager::open(config)?;
    Arc::new(whatsapp::registry()).install(&iii, manager.clone());
    let delivery = service::register(iii, manager.clone());
    tracing::info!("WhatsApp functions registered; sessions start only on explicit request");
    #[cfg(unix)]
    let signal = tokio::select! { result = tokio::signal::ctrl_c() => result, _ = terminate.recv() => Ok(()) };
    #[cfg(not(unix))]
    let signal = tokio::signal::ctrl_c().await;
    manager.shutdown().await;
    delivery.abort();
    let _ = delivery.await;
    signal.context("waiting for shutdown signal")
}
