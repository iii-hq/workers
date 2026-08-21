//! A2UI v0.9.1 generative UI worker for iii Harness and Console.

use std::sync::Arc;

use a2ui::composer::Composer;
use a2ui::config::WorkerConfig;
use a2ui::configuration::ConfigCell;
use a2ui::functions::Deps;
use a2ui::store::Store;
use a2ui::{configuration, functions, hook, manifest, ui};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "a2ui", about = manifest::DESCRIPTION)]
struct Cli {
    /// Optional one-time configuration seed. Stored configuration wins on
    /// subsequent boots.
    #[arg(long)]
    config: Option<String>,

    /// Engine websocket URL.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,

    /// Print the registry manifest and exit without connecting.
    #[arg(long)]
    manifest: bool,
}

#[cfg(unix)]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    use tokio::signal::unix::{signal, SignalKind};
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        _ = terminate.recv() => {}
    }
    Ok(())
}

#[cfg(not(unix))]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
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
                runtime: "rust".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "a2ui".into(),
                os: std::env::consts::OS.into(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

    let seed = cli
        .config
        .as_deref()
        .and_then(|path| match WorkerConfig::from_file(path) {
            Ok(config) => Some(config),
            Err(error) => {
                tracing::warn!(%error, path, "failed to parse configuration seed; ignoring it");
                None
            }
        });
    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(|error| anyhow::anyhow!("configuration::register failed: {error}"))?;
    let config = configuration::fetch_config(&iii)
        .await
        .map_err(|error| anyhow::anyhow!("configuration::get failed: {error}"))?;
    let config: ConfigCell = Arc::new(RwLock::new(Arc::new(config)));
    let store = Arc::new(Store::new(iii.clone()));
    let deps = Deps {
        iii: iii.clone(),
        config: config.clone(),
        store,
        composer: Arc::new(Composer::new(iii.clone())),
    };

    functions::register_all(&iii, deps);
    hook::register(&iii);
    hook::bind(&iii)
        .map_err(|error| anyhow::anyhow!("Harness pre-trigger binding failed: {error}"))?;
    ui::register(&iii);
    configuration::register_config_trigger(&iii, config)
        .map_err(|error| anyhow::anyhow!("configuration trigger registration failed: {error}"))?;

    tracing::info!(url = %cli.url, protocol = %a2ui::protocol::PROTOCOL_VERSION, "A2UI worker ready");
    wait_for_shutdown().await?;
    iii.shutdown_async().await;
    Ok(())
}
