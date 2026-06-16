use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, WorkerMetadata};
use tokio::sync::RwLock;

use coder::config::CoderConfig;
use coder::configuration::{self, ConfigCell};
use coder::functions;
use coder::manifest;
use coder::path::PathResolver;

#[derive(Parser, Debug)]
#[command(name = "coder", about = "Path-jailed code worker for iii agents")]
struct Cli {
    /// Optional seed config.yaml used to populate `initial_value` on first
    /// register. The AUTHORITATIVE config is always fetched from the
    /// `configuration` worker afterward; this file only seeds it. Keeps
    /// `--config ./config.yaml` working for deployments that pass it.
    #[arg(long)]
    config: Option<String>,

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

    // 2. register_worker FIRST so the configuration round-trip below runs over a
    //    live connection.
    tracing::info!(url = %cli.url, "connecting to III engine");
    let iii = register_worker(
        &cli.url,
        InitOptions {
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

    // 3. Best-effort seed: a failed parse WARNS and falls through to None (does
    //    NOT abort) — the authoritative value comes from fetch_config. The seed
    //    file IS env-expanded (`${VAR}`) via CoderConfig::from_file.
    let seed = match &cli.config {
        Some(path) => match CoderConfig::from_file(path) {
            Ok(cfg) => {
                tracing::info!(path = %path, "loaded seed config for initial registration");
                Some(cfg)
            }
            Err(e) => {
                tracing::warn!(
                    path = %path,
                    error = %e,
                    "failed to load seed config; relying on existing configuration entry"
                );
                None
            }
        },
        None => None,
    };

    // 4. Register the schema + (optional) seed with the configuration worker.
    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering coder configuration schema")?;

    // 5. Fetch the AUTHORITATIVE config (env-expanded by the configuration
    //    worker; falls back to default inside fetch_config when unset).
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading coder configuration")?;
    tracing::info!(
        base_path = ?cfg.base_path,
        base_paths = ?cfg.base_paths,
        non_accessible_globs = cfg.non_accessible_globs.len(),
        "coder configuration loaded"
    );

    // 6. Build the PathResolver from the authoritative cfg — SAME as today. The
    //    resolver is the security jail: built once here and NEVER rebuilt at
    //    runtime. Zero reachable roots / conflicting root config is an operator
    //    error — refuse to start instead of degrading silently. Capture the
    //    boot jail signature BEFORE moving cfg into the snapshot cell.
    let resolver = match PathResolver::new(&cfg) {
        Ok(r) => Arc::new(r),
        Err(e) => {
            anyhow::bail!(
                "failed to initialise PathResolver: {} (check `base_paths`/`base_path` and `non_accessible_globs`)",
                e
            );
        }
    };
    tracing::info!(roots = ?resolver.roots(), "path resolver ready");
    let boot_sig = cfg.jail_signature();

    // 7. The hot-swappable config snapshot shared with every cfg-taking handler.
    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg)));

    // 8. Register the RPC functions (handlers read the live snapshot per call).
    functions::register_all(&iii, resolver, cell.clone());

    // 9. LAST: bind the configuration-change trigger so its handler closes over
    //    the fully-built snapshot cell + the boot jail signature.
    configuration::register_config_trigger(&iii, cell, boot_sig)
        .context("registering configuration change trigger")?;

    tracing::info!("coder ready, waiting for invocations");
    tokio::signal::ctrl_c().await?;
    tracing::info!("coder shutting down");
    iii.shutdown_async().await;
    Ok(())
}
