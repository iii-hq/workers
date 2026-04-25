use anyhow::{Context, Result};
use clap::Parser;
use iii_mcp_client::{
    registration::register_all,
    session::{Session, SessionSpec},
};
use iii_sdk::{register_worker, InitOptions};

#[derive(Parser, Debug)]
#[command(
    name = "iii-mcp-client",
    about = "Consume external MCP servers; register their tools, resources, and prompts as iii functions."
)]
struct Cli {
    #[arg(long, default_value = "ws://localhost:49134")]
    engine_url: String,

    /// Repeatable connection spec.
    /// Grammar: `stdio:<name>:<bin>[:arg1:arg2:...]` or `http:<name>:<url>`.
    #[arg(long = "connect")]
    connect: Vec<String>,

    #[arg(long, default_value = "mcp")]
    namespace_prefix: String,

    #[arg(long)]
    debug: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let level = if cli.debug { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level)),
        )
        .init();

    tracing::info!(url = %cli.engine_url, "connecting to iii engine");

    let iii = register_worker(&cli.engine_url, InitOptions::default());

    if cli.connect.is_empty() {
        tracing::warn!("no --connect specs provided; iii-mcp-client has nothing to register");
    }

    for raw in &cli.connect {
        let spec = SessionSpec::parse(raw)
            .with_context(|| format!("invalid --connect spec: {raw}"))?;
        let name = spec.name().to_string();

        tracing::info!(server = %name, raw = %raw, "establishing MCP session");
        match Session::connect(spec).await {
            Ok(session) => {
                if let Err(e) =
                    register_all(&iii, session.clone(), &cli.namespace_prefix).await
                {
                    tracing::warn!(server = %name, error = %e, "register_all failed");
                }
            }
            Err(e) => {
                tracing::error!(server = %name, error = %e, "MCP session failed");
            }
        }
    }

    tracing::info!("iii-mcp-client running, waiting for invocations");
    tokio::signal::ctrl_c().await?;

    tracing::info!("iii-mcp-client shutting down");
    iii.shutdown_async().await;

    Ok(())
}
