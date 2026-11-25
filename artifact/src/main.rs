//! The artifact worker: versioned single-file artifacts and expiring share
//! links.
//!
//! Boot order:
//!
//! 1. tracing, then the CLI; `--manifest` prints and returns without
//!    connecting
//! 2. connect, then register and fetch the configuration (a required boot
//!    dependency)
//! 3. bind the loopback share listener — a bind failure aborts, because
//!    every share link would be dead
//! 4. register the functions and the console UI, start the expiry sweeper, observe
//!    configuration changes last
//! 5. wait for a signal, then stop the listener and shut the SDK down

use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{InitOptions, register_worker};
use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;

use artifact::config::WorkerConfig;
use artifact::configuration::{self, ConfigCell};
use artifact::functions::{self, Ctx, sharing};
use artifact::store::Store;
use artifact::tunnel::Tunnel;
use artifact::{manifest, server, ui};

/// How often expired shares are dropped and an idle tunnel released.
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Parser, Debug)]
#[command(name = "artifact", about = manifest::DESCRIPTION)]
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

#[cfg(unix)]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};
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

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "artifact".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

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

    let listener = tokio::net::TcpListener::bind(
        cfg.listen_socket()
            .map_err(|e| anyhow::anyhow!("invalid listen_addr: {e}"))?,
    )
    .await
    .map_err(|e| anyhow::anyhow!("cannot bind the share listener {}: {e}", cfg.listen_addr))?;
    let bound = listener.local_addr()?;
    tracing::info!(%bound, tunnel_id = %cfg.tunnel_id, "share listener bound");

    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg)));
    let store = Arc::new(Store::new(iii.clone()));
    let ctx = Arc::new(Ctx {
        store: store.clone(),
        tunnel: Arc::new(Tunnel::Bus(iii.clone())),
        local_base: format!("http://{bound}"),
    });

    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        let result = axum::serve(listener, server::router(store))
            .with_graceful_shutdown(async {
                let _ = stop_rx.await;
            })
            .await;
        if let Err(e) = result {
            tracing::error!(error = %e, "share listener stopped");
        }
    });

    functions::register_all(&iii, &ctx, &cell);
    ui::register(&iii);

    let sweeper_ctx = ctx.clone();
    let sweeper = tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            tick.tick().await;
            match sharing::sweep(&sweeper_ctx).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(expired = n, "expired shares dropped"),
                Err(e) => tracing::warn!(error = %e, "share sweep failed"),
            }
        }
    });

    configuration::register_config_observer(&iii, cell.clone());

    tracing::info!(url = %cli.url, "artifact worker ready");

    wait_for_shutdown().await?;
    sweeper.abort();
    let _ = stop_tx.send(());
    let _ = server_task.await;
    iii.shutdown_async().await;
    Ok(())
}
