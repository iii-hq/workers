use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, WorkerMetadata};

use turn_orchestrator::{config, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "turn-orchestrator",
    about = "Durable run::start state machine driving each agent turn through provisioning, assistant, tools, steering, and tearing-down."
)]
struct Cli {
    #[arg(
        long,
        env = "TURN_ORCHESTRATOR_CONFIG",
        default_value = "./config.yaml"
    )]
    config: String,

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
        let m = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m).unwrap());
        return Ok(());
    }

    let cfg = match config::load_config(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::TurnOrchestratorConfig::default()
        }
    };
    let cfg = Arc::new(cfg);

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "turn-orchestrator".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    turn_orchestrator::register_with_iii(&iii, &cfg)
        .await
        .expect("turn-orchestrator register failed");
    tracing::info!("turn-orchestrator registered (run::start, run::start_and_wait, turn::step)");

    tokio::signal::ctrl_c().await?;
    tracing::info!("turn-orchestrator shutting down");
    iii.shutdown_async().await;
    Ok(())
}
