//! The artifact worker: versioned single-file artifacts and expiring share
//! links.
//!
//! Boot order:
//!
//! 1. tracing, then the CLI; `--manifest` prints and returns without
//!    connecting
//! 2. connect, then register and fetch the configuration (a required boot
//!    dependency)
//! 3. bind the two loopback share listeners (public: the tunnel target,
//!    public shares only; local: every share) — a bind failure aborts,
//!    because every share link would be dead
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
use artifact::dns::Dns;
use artifact::functions::{self, Ctx, sharing};
use artifact::server::Exposure;
use artifact::store::Store;
use artifact::tunnel::Tunnel;
use artifact::{manifest, server, ui};

/// How often expired shares are dropped and an idle tunnel released.
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// How long boot waits for the configuration observer to turn active; past
/// it, startup fails and the supervisor restarts the worker.
const OBSERVER_READY_TIMEOUT: Duration = Duration::from_secs(30);

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

    // An explicit seed that does not load is an operator error: falling back
    // would persist the built-in defaults on first boot, and a corrected
    // seed file never replaces a stored value afterwards.
    let seed = match cli.config.as_deref() {
        Some(path) => Some(
            WorkerConfig::from_file(path)
                .map_err(|e| anyhow::anyhow!("--config {path} is not a valid seed: {e}"))?,
        ),
        None => None,
    };

    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(|e| anyhow::anyhow!("configuration::register failed: {e}"))?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(|e| anyhow::anyhow!("configuration::get failed: {e}"))?;

    let public_listener = bind("listen_addr", cfg.listen_socket()).await?;
    let local_listener = bind("local_listen_addr", cfg.local_listen_socket()).await?;
    let local_bound = local_listener.local_addr()?;
    tracing::info!(
        public = %public_listener.local_addr()?,
        local = %local_bound,
        tunnel_id = %cfg.tunnel_id,
        "share listeners bound"
    );

    let dns = Arc::new(
        Dns::new(&cfg.dns_check_resolver)
            .map_err(|e| anyhow::anyhow!("invalid dns_check_resolver: {e}"))?,
    );

    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg)));
    let store = Arc::new(Store::new(iii.clone()));
    let ctx = Arc::new(Ctx {
        store: store.clone(),
        tunnel: Arc::new(Tunnel::Bus(iii.clone())),
        dns,
        local_base: format!("http://{local_bound}"),
    });

    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let servers = [
        (public_listener, Exposure::Public),
        (local_listener, Exposure::Local),
    ]
    .map(|(listener, exposure)| {
        let mut stop = stop_rx.clone();
        let app = server::router(store.clone(), exposure);
        tokio::spawn(async move {
            let result = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = stop.wait_for(|stopped| *stopped).await;
                })
                .await;
            if let Err(e) = result {
                tracing::error!(error = %e, ?exposure, "share listener stopped");
            }
        })
    });

    functions::register_all(&iii, &ctx, &cell);
    ui::register(&iii);

    let sweeper_ctx = ctx.clone();
    let sweeper_cell = cell.clone();
    let sweeper = tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            tick.tick().await;
            let cfg = sweeper_cell.read().await.clone();
            match sharing::sweep(&sweeper_ctx, &cfg).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(expired = n, "ended shares dropped"),
                Err(e) => tracing::warn!(error = %e, "share sweep failed"),
            }
        }
    });

    // A change made between the boot read and the observer turning active
    // would be missed. Startup therefore waits for the observer, then reads
    // the configuration once more through the same serialized path. Either
    // step failing is a startup error: running on a snapshot that may be
    // stale, with no observer to correct it, is worse than restarting.
    let observer = configuration::register_config_observer(&iii, cell.clone());
    match tokio::time::timeout(OBSERVER_READY_TIMEOUT, observer.ready()).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => anyhow::bail!("configuration observer did not activate: {e}"),
        Err(_) => anyhow::bail!(
            "configuration observer still pending after {}s",
            OBSERVER_READY_TIMEOUT.as_secs()
        ),
    }
    configuration::reload(&iii, &cell)
        .await
        .map_err(|e| anyhow::anyhow!("configuration catch-up read failed: {e}"))?;

    tracing::info!(url = %cli.url, "artifact worker ready");

    wait_for_shutdown().await?;
    sweeper.abort();
    let _ = stop_tx.send(true);
    for server in servers {
        let _ = server.await;
    }
    iii.shutdown_async().await;
    Ok(())
}

async fn bind(
    field: &str,
    addr: Result<std::net::SocketAddr, String>,
) -> anyhow::Result<tokio::net::TcpListener> {
    let addr = addr.map_err(|e| anyhow::anyhow!("invalid {field}: {e}"))?;
    tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("cannot bind the share listener {addr} ({field}): {e}"))
}
