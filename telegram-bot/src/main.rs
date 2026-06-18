use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, WorkerMetadata};
use tokio::sync::RwLock;

use telegram_bot::clients::telegram;
use telegram_bot::configuration::{self, ConfigCell};
use telegram_bot::deps::Deps;
use telegram_bot::functions;
use telegram_bot::ingress;
use telegram_bot::WorkerConfig;

#[derive(Parser, Debug)]
#[command(
    name = "telegram-bot",
    about = "Telegram bridge to the harness stack (polling or webhook ingress)."
)]
struct Cli {
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
        let m = telegram_bot::manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m).unwrap());
        return Ok(());
    }

    let seed = cli.config.as_deref().and_then(|path| {
        match WorkerConfig::from_file(path) {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!(error = %e, path, "failed to parse --config seed; continuing without seed");
                None
            }
        }
    });

    let iii = register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "telegram-bot".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering telegram-bot configuration schema")?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading telegram-bot configuration")?;
    cfg.validate()
        .map_err(anyhow::Error::msg)
        .context("telegram-bot requires a non-empty bot_token in configuration")?;

    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg)));
    let deps = Arc::new(Deps::new(iii.clone(), cell.clone()));

    functions::register_all(&iii, &deps);
    functions::bind_triggers(&iii);
    functions::bind_http_triggers(&iii);

    configuration::register_config_trigger(&iii, cell, deps.clone())
        .map_err(anyhow::Error::msg)
        .context("binding configuration trigger")?;

    ingress::start(&deps).await;

    if let Err(e) = telegram::set_my_commands(&deps).await {
        tracing::warn!(error = %e, "failed to register bot commands with Telegram");
    }

    tracing::info!("telegram-bot ready, waiting for invocations");
    tokio::signal::ctrl_c().await?;
    tracing::info!("telegram-bot shutting down");
    ingress::shutdown(&deps).await;
    iii.shutdown_async().await;
    Ok(())
}
