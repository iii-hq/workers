use std::sync::Arc;

use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{InitOptions, register_worker};
use mcp_client::log_directives;
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
    tracing_subscriber::fmt().with_env_filter(filter).init();
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
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    tokio::signal::ctrl_c().await?;
    iii.shutdown_async().await;
    Ok(())
}
