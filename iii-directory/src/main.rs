//! `iii-directory` binary entry.
//!
//! Boot sequence:
//!   1. Parse CLI / load YAML config (with fallback to defaults).
//!   2. Connect to the iii engine over WebSocket.
//!   3. Register the custom trigger types `skills::on-change` /
//!      `prompts::on-change` (fan-out targets for `skills::download`).
//!   4. Register every public function against the engine
//!      (`skills::*`, `prompts::*`, `skill::fetch`, `directory::*`,
//!      `registry::*`).
//!   5. Sleep on Ctrl+C, then `shutdown_async` cleanly.
//!
//! `skills::download` is the only write path. Read-side surfaces
//! (`iii://`, `skill::fetch`, `prompts::*`, `directory::*`,
//! `registry::*`) source from the configured `skills_folder` on disk
//! or proxy to the public registry over HTTP.

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, WorkerMetadata};

use iii_directory::{config, functions, manifest, trigger_types};

#[derive(Parser, Debug)]
#[command(
    name = "iii-directory",
    about = "Engine introspection (functions / triggers / workers), workers registry proxy, and filesystem-backed skill + prompt reader."
)]
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
        let m = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m).unwrap());
        return Ok(());
    }

    let cfg = match config::load_config(&cli.config) {
        Ok(c) => {
            tracing::info!(
                skills_folder = %c.resolved_skills_folder().display(),
                registry_url = %c.registry_base(),
                "loaded config from {}",
                cli.config
            );
            c
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::SkillsConfig::default()
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
                name: "iii-directory".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    // Custom trigger types come first because the download function
    // captures the subscriber sets it'll fan out to on success.
    let registered = trigger_types::register_all(&iii);
    functions::register_all(&iii, &cfg, &registered);
    functions::log_fs_health(&cfg);

    tracing::info!("iii-directory ready: 21 functions + 2 custom trigger types");

    tokio::signal::ctrl_c().await?;
    tracing::info!("iii-directory shutting down");
    iii.shutdown_async().await;
    Ok(())
}
