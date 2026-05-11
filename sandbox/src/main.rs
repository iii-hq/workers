use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions};

use sandbox_router::config::Config;
use sandbox_router::register::{register_all, Ctx};

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

#[derive(Parser, Debug)]
#[command(name = "iii-sandbox")]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: PathBuf,
    #[arg(long, default_value = DEFAULT_ENGINE_URL)]
    url: String,
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
    let cfg = Config::load(&cli.config).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "failed to load config, using defaults");
        Config::default()
    });
    let ctx = Ctx::new(Arc::new(cfg));

    let iii = register_worker(&cli.url, InitOptions::default());
    register_all(&iii, ctx);

    tracing::info!("sandbox router registered, awaiting invocations");
    tokio::signal::ctrl_c().await?;
    iii.shutdown_async().await;
    Ok(())
}
