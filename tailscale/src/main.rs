use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};

use tailscale::config::WorkerConfig;
use tailscale::{configuration, functions, manifest, ui};

#[derive(Parser, Debug)]
#[command(
    name = "tailscale",
    about = "Share the iii Console over Tailscale Serve or explicitly enabled Funnel."
)]
struct Cli {
    /// YAML seed applied only when the `tailscale` configuration entry is first created.
    #[arg(long)]
    config: Option<String>,
    /// iii engine WebSocket address.
    #[arg(long, default_value = "ws://127.0.0.1:49134")]
    url: String,
    /// Print the registry manifest as JSON and exit without connecting.
    #[arg(long)]
    manifest: bool,
}

async fn wait_for_shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result?,
            _ = sigterm.recv() => {}
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
        Ok(())
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
                name: "tailscale".to_string(),
                os: std::env::consts::OS.to_string(),
                description: Some(
                    "Share the local iii Console through Tailscale with QR links and exact route controls."
                        .to_string(),
                ),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

    let seed = cli.config.as_deref().and_then(|path| {
        let contents = std::fs::read_to_string(path)
            .map_err(|error| tracing::warn!(path, %error, "ignoring unreadable seed config"))
            .ok()?;
        let value: serde_json::Value = serde_yaml::from_str(&contents)
            .map_err(|error| tracing::warn!(path, %error, "ignoring invalid seed YAML"))
            .ok()?;
        WorkerConfig::from_json(&value)
            .map_err(|error| tracing::warn!(path, %error, "ignoring invalid seed config"))
            .ok()
    });

    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering tailscale configuration")?;
    let config = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading tailscale configuration")?
        .into_shared();

    functions::register_all(&iii, config.clone());
    configuration::register_config_trigger(&iii, config)
        .context("registering tailscale configuration trigger")?;
    ui::register(&iii);

    tracing::info!("tailscale worker ready: status, configuration, share, and exact-route stop");
    wait_for_shutdown_signal().await?;
    tracing::info!("tailscale worker shutting down");
    iii.shutdown_async().await;
    Ok(())
}
