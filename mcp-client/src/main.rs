use std::sync::Arc;

use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{InitOptions, register_worker};
use mcp_client::host::EngineHost;
use mcp_client::server::{Manager, ManagerOptions, Timings};
use mcp_client::translate::ScrubWriter;
use mcp_client::{configuration, functions, log_directives, stdio_allowed};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "mcp-client",
    about = "MCP client: the tools of configured MCP servers as iii functions."
)]
struct Cli {
    /// Engine websocket URL.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
}

fn main() -> anyhow::Result<()> {
    // SAFETY: first action of synchronous main, before the runtime or any thread exists.
    unsafe { mcp_client::disable_trace_payloads() };
    // Already installed is fine: the first provider wins.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cli = Cli::parse();
    let directives = log_directives(std::env::var("RUST_LOG").ok().as_deref());
    let filter =
        EnvFilter::try_new(&directives).unwrap_or_else(|_| EnvFilter::new(log_directives(None)));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(ScrubWriter(std::io::stderr))
        .init();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run(cli))
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "mcp-client".into(),
                description: Some(
                    "Connects to MCP servers and registers their tools as iii functions.".into(),
                ),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    // Interface capture boots with no configuration worker: go on with no servers.
    if let Err(error) = configuration::register_config(&iii).await {
        tracing::warn!(%error, "could not register the configuration entry");
    }
    let manager = Manager::new(
        EngineHost::new(iii.clone()),
        ManagerOptions {
            stdio_enabled: stdio_allowed(std::env::var("III_MCP_CLIENT_STDIO").ok().as_deref()),
            timings: Timings::default(),
            worker_cwd: std::env::current_dir()?,
        },
    );
    functions::register_all(&iii, &manager);
    match configuration::fetch_config(&iii).await {
        Ok(raw) => manager.apply_config(raw).await,
        Err(error) => tracing::warn!(%error, "could not read the configuration entry; no servers"),
    }
    // An equal map is a no-op, so this pass only catches an edit made since the read.
    configuration::register_config_observer(&iii, manager.clone())
        .run()
        .await;
    wait_for_shutdown().await?;
    manager.shutdown().await;
    iii.shutdown_async().await;
    Ok(())
}

#[cfg(unix)]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
    Ok(())
}

#[cfg(not(unix))]
async fn wait_for_shutdown() -> anyhow::Result<()> {
    tokio::signal::ctrl_c().await?;
    Ok(())
}
