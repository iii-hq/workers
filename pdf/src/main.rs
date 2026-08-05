//! The pdf worker: read PDFs locally, and know which pages need OCR.
//!
//! Boot order, and why:
//!
//! 1. tracing, then the CLI
//! 2. `--manifest` prints and returns without connecting, because the registry
//!    publish pipeline calls it and must not need an engine
//! 3. materialize the CJK CMaps, before any document is parsed
//! 4. connect
//! 5. register and fetch the configuration — a required boot dependency, so a
//!    failure here aborts rather than running on guessed limits
//! 6. register the functions, then the console UI they drive
//! 7. bind the configuration trigger LAST, so its handler closes over fully
//!    built state
//! 8. wait for a signal, then shut the SDK down cleanly

use std::sync::Arc;

use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;

use pdf::config::WorkerConfig;
use pdf::configuration::ConfigCell;
use pdf::{cmaps, configuration, functions, manifest, ui};

#[derive(Parser, Debug)]
#[command(name = "pdf", about = manifest::DESCRIPTION)]
struct Cli {
    /// Optional one-time seed for the configuration entry on first
    /// registration. Never overwrites a stored value.
    #[arg(long)]
    config: Option<String>,

    /// Engine websocket URL.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,

    /// Print the registry manifest and exit.
    #[arg(long)]
    manifest: bool,
}

/// Wait for either interrupt or terminate.
///
/// A managed worker is stopped with SIGTERM, and a process that only listens
/// for ctrl-c dies without running `shutdown_async`, which leaves its
/// Message-path triggers registered against a function that no longer exists.
#[cfg(unix)]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    use tokio::signal::unix::{signal, SignalKind};
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        _ = terminate.recv() => {}
    }
    Ok(())
}

#[cfg(not(unix))]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest::build_manifest())?
        );
        return Ok(());
    }

    // Before anything parses a document: without this, CID fonts with no
    // ToUnicode table decode to empty text and nothing says so.
    cmaps::materialize();

    let iii = register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "pdf".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    // A malformed seed warns and falls through: the stored value or the
    // built-in default still applies, and refusing to boot over a seed file
    // would be worse than ignoring it.
    let seed = cli
        .config
        .as_deref()
        .and_then(|path| match WorkerConfig::from_file(path) {
            Ok(cfg) => Some(cfg),
            Err(e) => {
                tracing::warn!(error = %e, path, "failed to parse config seed; ignoring it");
                None
            }
        });

    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(|e| anyhow::anyhow!("configuration::register failed: {e}"))?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(|e| anyhow::anyhow!("configuration::get failed: {e}"))?;
    tracing::info!(
        max_input_bytes = cfg.max_input_bytes,
        max_chars = cfg.max_chars,
        classify_sample_pages = cfg.classify_sample_pages,
        "configuration loaded"
    );
    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg)));

    functions::register_all(&iii, &cell);
    ui::register(&iii);

    configuration::register_config_trigger(&iii, cell.clone())
        .map_err(|e| anyhow::anyhow!("configuration trigger registration failed: {e}"))?;

    tracing::info!(url = %cli.url, "pdf worker ready");

    wait_for_shutdown().await?;
    iii.shutdown_async().await;
    Ok(())
}
