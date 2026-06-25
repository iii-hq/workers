//! `web` binary entry: connect, register configuration + fetch the
//! authoritative value, register `web::fetch`, bind the config-change
//! trigger for hot-reload, then sleep until Ctrl+C.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};

use web::config::WebConfig;
use web::{configuration, functions, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "web",
    about = "Outbound HTTP client on the iii bus (web::fetch)."
)]
struct Cli {
    /// Optional YAML seed used to populate `initial_value` on first registration.
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
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest::build_manifest()).unwrap()
        );
        return Ok(());
    }

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "web".to_string(),
                os: std::env::consts::OS.to_string(),
                description: Some("Outbound HTTP client on the iii bus (web::fetch).".to_string()),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

    // Optional YAML seed → initial_value on first registration.
    let seed = cli.config.as_deref().and_then(|path| {
        let contents = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(path, error = %e, "failed to read seed config; ignoring");
                return None;
            }
        };
        match serde_yaml::from_str::<serde_json::Value>(&contents) {
            Ok(v) => match WebConfig::from_json(&v) {
                Ok(cfg) => Some(cfg),
                Err(e) => {
                    tracing::warn!(path, error = %e, "failed to parse seed config; ignoring");
                    None
                }
            },
            Err(e) => {
                tracing::warn!(path, error = %e, "failed to parse YAML seed config; ignoring");
                None
            }
        }
    });

    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering web configuration schema")?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading web configuration")?;
    tracing::info!(
        max_timeout_ms = cfg.max_timeout_ms,
        max_response_bytes = cfg.max_response_bytes,
        allow_loopback = cfg.allow_loopback,
        "loaded web configuration"
    );

    let shared = cfg.into_shared();
    functions::register_all(&iii, &shared);
    configuration::register_config_trigger(
        &iii,
        configuration::SharedState {
            config: shared.clone(),
        },
    )
    .context("registering configuration change trigger")?;

    tracing::info!("web ready: web::fetch + configuration hot-reload");
    tokio::signal::ctrl_c().await?;
    tracing::info!("web shutting down");
    iii.shutdown_async().await;
    Ok(())
}
