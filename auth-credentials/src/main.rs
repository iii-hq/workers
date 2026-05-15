use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, WorkerMetadata};

mod config;
mod manifest;

use config::{resolve_store_backend, StoreBackend};

#[derive(Parser, Debug)]
#[command(
    name = "auth-credentials",
    about = "Provider credential vault on the iii bus — API keys and OAuth tokens under auth::*."
)]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    /// Engine WebSocket URL. Falls back to `engine_url` in config when unset.
    #[arg(long, env = "III_URL")]
    url: Option<String>,

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
        let m = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m)?);
        return Ok(());
    }

    let cfg = match config::load_config(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::WorkerConfig::default()
        }
    };

    let engine_url = cli.url.unwrap_or_else(|| cfg.engine_url.clone());
    let store_kind = resolve_store_backend(&cfg);

    let iii = Arc::new(register_worker(
        &engine_url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "auth-credentials".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

    let store: Arc<dyn auth_credentials::CredentialStore> = match store_kind {
        StoreBackend::Memory => {
            tracing::info!("auth-credentials: using in-memory store");
            Arc::new(auth_credentials::InMemoryStore::new())
        }
        StoreBackend::IiiState => {
            tracing::info!("auth-credentials: using iii-state store");
            let iii_for_store: Arc<dyn auth_credentials::io::IIITrigger> = iii.clone();
            Arc::new(auth_credentials::store_iii_state::IiiStateCredentialStore::new(iii_for_store))
        }
    };

    let _refs = auth_credentials::register_with_iii(&iii, store)
        .await
        .context("auth-credentials register failed")?;
    tracing::info!("auth-credentials registered (auth::*)");

    wait_for_shutdown().await?;

    tracing::info!("auth-credentials shutting down");
    iii.shutdown_async().await;
    Ok(())
}

async fn wait_for_shutdown() -> Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).context("failed to install SIGTERM handler")?;
        tokio::select! {
            r = tokio::signal::ctrl_c() => r.context("failed to await SIGINT")?,
            _ = sigterm.recv() => {}
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .context("failed to await SIGINT")
    }
}
