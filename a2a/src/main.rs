use clap::Parser;
use iii_a2a::handler;
use iii_sdk::{InitOptions, register_worker};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[derive(Parser, Debug)]
#[command(name = "iii-a2a")]
#[command(version)]
#[command(about = "A2A protocol worker for iii-engine")]
struct Args {
    #[arg(long, short = 'e', default_value = "ws://localhost:49134")]
    engine_url: String,

    #[arg(long, short = 'd')]
    debug: bool,

    #[arg(
        long,
        help = "Expose all functions as skills (ignore a2a.expose metadata). \
                Infra namespaces (engine::*, state::*, stream::*, iii.*, a2a::*) \
                stay hidden even with this flag."
    )]
    expose_all: bool,

    #[arg(
        long,
        help = "Show only functions whose `a2a.tier` metadata equals this value \
                (e.g. `user`, `agent`, `ops`). When unset, tier filtering is off."
    )]
    tier: Option<String>,

    #[arg(
        long,
        default_value = "http://localhost:3111",
        help = "Public base URL advertised in the agent card"
    )]
    base_url: String,

    #[arg(
        long,
        default_value = "iii-engine",
        help = "Agent name advertised in the agent card"
    )]
    agent_name: String,

    #[arg(
        long,
        default_value = "iii-engine agent — invoke any registered function via A2A",
        help = "Agent description advertised in the agent card"
    )]
    agent_description: String,

    #[arg(
        long,
        default_value = "iii",
        help = "Provider organization advertised in the agent card"
    )]
    provider_org: String,

    #[arg(
        long,
        default_value = "https://github.com/iii-hq/iii",
        help = "Provider URL advertised in the agent card"
    )]
    provider_url: String,

    #[arg(
        long,
        default_value = "https://github.com/iii-hq/workers/tree/main/a2a",
        help = "Documentation URL advertised in the agent card"
    )]
    docs_url: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let filter = if args.debug {
        EnvFilter::new("iii_a2a=debug,iii_sdk=debug")
    } else {
        EnvFilter::new("iii_a2a=info,iii_sdk=warn")
    };

    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(filter)
        .init();

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Starting iii-a2a");

    let iii = register_worker(&args.engine_url, InitOptions::default());

    let exposure = handler::ExposureConfig::new(args.expose_all, args.tier.clone());
    let identity = handler::AgentIdentity {
        name: args.agent_name,
        description: args.agent_description,
        provider_org: args.provider_org,
        provider_url: args.provider_url,
        docs_url: args.docs_url,
    };
    handler::register(&iii, exposure, args.base_url, identity);

    tracing::info!("A2A endpoints registered on engine port. Ctrl+C to stop.");
    tokio::signal::ctrl_c().await?;

    Ok(())
}
