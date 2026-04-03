use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig};
use std::sync::Arc;

mod config;
mod docker;
mod functions;
mod manifest;
mod types;

#[derive(Parser, Debug)]
#[command(name = "sandbox-docker", about = "III engine Docker sandbox worker")]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

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
        let manifest = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&manifest).unwrap());
        return Ok(());
    }

    let worker_config = match config::load_config(&cli.config) {
        Ok(c) => {
            tracing::info!(
                image = %c.default_image,
                timeout = c.default_timeout,
                max_sandboxes = c.max_sandboxes,
                pool_size = c.pool_size,
                "loaded config from {}",
                cli.config
            );
            c
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::SandboxWorkerConfig::default()
        }
    };

    let config = Arc::new(worker_config);

    let docker_client =
        Arc::new(docker::connect_docker().expect("failed to connect to Docker daemon"));

    tracing::info!(url = %cli.url, "connecting to III engine");

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    ));

    functions::register_all(&iii, docker_client, config);

    tracing::info!("all sandbox functions registered, waiting for invocations");

    tokio::signal::ctrl_c().await?;

    tracing::info!("sandbox-docker shutting down");
    iii.shutdown_async().await;

    Ok(())
}
