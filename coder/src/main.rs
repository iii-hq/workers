use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_observability::OtelConfig;
use iii_sdk::{register_worker, InitOptions, WorkerMetadata};

use coder::config;
use coder::functions;
use coder::manifest;
use coder::path::PathResolver;

#[derive(Parser, Debug)]
#[command(name = "coder", about = "Path-jailed code worker for iii agents")]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
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
        println!(
            "{}",
            serde_json::to_string_pretty(&m).expect("manifest serializes")
        );
        return Ok(());
    }

    let cfg = match config::load_config(&cli.config) {
        Ok(c) => {
            tracing::info!(
                base_path = %c.base_path.display(),
                non_accessible_globs = c.non_accessible_globs.len(),
                "loaded config from {}",
                cli.config
            );
            c
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %cli.config,
                "failed to load config, using defaults"
            );
            config::CoderConfig::default()
        }
    };
    let cfg = Arc::new(cfg);

    let resolver = match PathResolver::new(&cfg) {
        Ok(r) => Arc::new(r),
        Err(e) => {
            // base_path canonicalization failure is operator config —
            // refuse to start instead of degrading silently.
            anyhow::bail!(
                "failed to initialise PathResolver: {} (check `base_path` and `non_accessible_globs`)",
                e.code()
            );
        }
    };
    tracing::info!(
        base_root = %resolver.base_root().display(),
        "path resolver ready"
    );

    tracing::info!(url = %cli.url, "connecting to III engine");
    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "coder".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );

    functions::register_all(&iii, resolver.clone(), cfg.clone());

    tracing::info!("coder ready, waiting for invocations");
    tokio::signal::ctrl_c().await?;
    tracing::info!("coder shutting down");
    iii.shutdown_async().await;
    Ok(())
}
