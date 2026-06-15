use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_observability::OtelConfig;
use iii_sdk::{register_worker, InitOptions};

use codex::config::Config;
use codex::functions::register_all;
use codex::manifest;

#[derive(Parser, Debug)]
#[command(name = "codex", about = "OpenAI Codex worker for iii agents")]
struct Cli {
    /// Seed config loaded at boot. Defaults to ./config.yaml.
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    /// Engine WebSocket URL. Unset falls back to `engine_url` in config.yaml.
    #[arg(long, env = "III_URL", default_value = "")]
    url: String,

    /// Print the registry manifest as JSON and exit.
    #[arg(long)]
    manifest: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.manifest {
        let m = manifest::build_manifest();
        println!(
            "{}",
            serde_json::to_string_pretty(&m).expect("manifest serializes")
        );
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cfg = Arc::new(Config::load(&cli.config)?);
    let url = if cli.url.is_empty() {
        cfg.engine_url.clone()
    } else {
        cli.url.clone()
    };
    tracing::info!(url = %url, config = %cli.config, "connecting to III engine");

    let iii = register_worker(
        &url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );

    register_all(&iii, cfg);
    tracing::info!("codex worker registered all functions, ready");

    wait_for_shutdown_signal().await?;
    tracing::info!("codex worker shutting down");
    iii.shutdown_async().await;
    Ok(())
}

async fn wait_for_shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate())?;
        tokio::select! {
            r = tokio::signal::ctrl_c() => r,
            _ = sigterm.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}
