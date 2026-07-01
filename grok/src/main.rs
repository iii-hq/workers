use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_helpers::observability::OtelConfig;
use iii_sdk::{register_worker, InitOptions};
use tokio::sync::RwLock;

use grok::config::Config;
use grok::configuration;
use grok::functions::register_all;
use grok::manifest;

#[derive(Parser, Debug)]
#[command(name = "grok", about = "xAI Grok CLI worker for iii agents")]
struct Cli {
    /// Seed config registered as `initial_value` with the `configuration`
    /// worker on first registration. Defaults to ./config.yaml. The live value
    /// from the configuration worker is authoritative once an entry exists.
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
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

    tracing::info!(url = %cli.url, seed_config = %cli.config, "connecting to III engine");

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );

    // Seed from config.yaml when present; a parse error fails fast.
    let seed = match Config::load(&cli.config) {
        Ok(cfg) => Some(cfg),
        Err(e) => {
            tracing::warn!(path = %cli.config, error = %e, "failed to load seed config; relying on the configuration worker");
            None
        }
    };

    // Config registration is best-effort: grok has no security policy, so if
    // the configuration worker is unreachable (or absent, as in interface
    // collection on a bare engine) the worker still serves with the seed /
    // built-in defaults. Never fatal — registering grok::* must not depend on
    // the configuration worker being up.
    if let Err(e) = configuration::register_config(&iii, seed.as_ref()).await {
        tracing::warn!(error = %e, "configuration::register failed; continuing with the seed");
    }
    let cfg = match configuration::fetch_config(&iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!(error = %e, "configuration fetch failed; using seed/default config");
            seed.clone().unwrap_or_default()
        }
    };

    let cell: configuration::ConfigCell = Arc::new(RwLock::new(Arc::new(cfg)));

    // Bind the config-change trigger and reconcile so a value that landed
    // during boot is applied before the first turn. Best-effort.
    if let Err(e) = configuration::register_config_trigger(&iii, cell.clone()) {
        tracing::warn!(error = %e, "configuration change trigger registration failed");
    }
    configuration::reconcile(&iii, &cell).await;

    register_all(&iii, cell);
    tracing::info!("grok worker registered all functions, ready");

    wait_for_shutdown_signal().await?;
    tracing::info!("grok worker shutting down");
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
