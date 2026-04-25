//! `iii-a2a-client` — register external A2A agents' skills as local iii
//! functions. Pair with `iii-a2a` (the server-side worker) to run a
//! cross-protocol harness.

use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use iii_a2a_client::{registration, session};
use iii_sdk::{register_worker, InitOptions};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Parser, Debug)]
#[command(
    name = "iii-a2a-client",
    about = "Consume external A2A agents; register their skills as iii functions.",
    version
)]
struct Cli {
    /// WebSocket URL of the iii engine to register against.
    #[arg(long, default_value = "ws://localhost:49134")]
    engine_url: String,

    /// Base URL of an external A2A agent. The agent card must be served at
    /// `<URL>/.well-known/agent-card.json`. Pass repeatedly for multiple agents.
    #[arg(long = "connect", required = true)]
    connect: Vec<String>,

    /// Namespace prefix used when registering remote skills as local
    /// functions (`<prefix>.<agent>::<skill>`).
    #[arg(long, default_value = "a2a")]
    namespace_prefix: String,

    /// Re-fetch each connected agent's card at this cadence (seconds) to pick
    /// up newly added or removed skills.
    #[arg(long, default_value_t = 30)]
    poll_interval: u64,

    /// Verbose logging.
    #[arg(long)]
    debug: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let filter = if cli.debug {
        EnvFilter::new("iii_a2a_client=debug,iii_sdk=info")
    } else {
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("iii_a2a_client=info,iii_sdk=warn"))
    };
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(filter)
        .init();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        engine = %cli.engine_url,
        agents = cli.connect.len(),
        "starting iii-a2a-client"
    );

    let iii = register_worker(&cli.engine_url, InitOptions::default());
    let poll = Duration::from_secs(cli.poll_interval.max(1));

    for base_url in cli.connect.iter().cloned() {
        let session = match session::Session::connect(&base_url).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, base_url = %base_url, "skip agent: connect failed");
                continue;
            }
        };
        let map = registration::register_all(&iii, session.clone(), &cli.namespace_prefix).await;
        registration::spawn_poll_loop(
            iii.clone(),
            session,
            cli.namespace_prefix.clone(),
            map,
            poll,
        );
    }

    tracing::info!("registrations complete; Ctrl+C to stop");
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down iii-a2a-client");
    Ok(())
}
