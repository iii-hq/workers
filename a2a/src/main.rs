use std::collections::HashMap;

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
        default_value = "http://localhost:3111",
        help = "Public base URL advertised in the agent card"
    )]
    base_url: String,

    #[arg(
        long,
        default_value = iii_a2a::handler::DEFAULT_AGENT_NAME,
        help = "Agent name advertised in the agent card"
    )]
    agent_name: String,

    #[arg(
        long,
        default_value = iii_a2a::handler::DEFAULT_AGENT_DESCRIPTION,
        help = "Agent description advertised in the agent card"
    )]
    agent_description: String,

    #[arg(
        long,
        default_value = iii_a2a::handler::DEFAULT_PROVIDER_ORG,
        help = "Provider organization advertised in the agent card"
    )]
    provider_org: String,

    #[arg(
        long,
        default_value = iii_a2a::handler::DEFAULT_PROVIDER_URL,
        help = "Provider URL advertised in the agent card"
    )]
    provider_url: String,

    #[arg(
        long,
        default_value = iii_a2a::handler::DEFAULT_DOCS_URL,
        help = "Documentation URL advertised in the agent card"
    )]
    docs_url: String,

    #[arg(
        long,
        value_name = "TAG",
        help = "Forward an `x-iii-rbac-tag` header on the worker WebSocket \
                upgrade. iii-worker-manager's `auth_function_id` reads this \
                tag to apply policy. RBAC itself lives at iii-worker-manager."
    )]
    rbac_tag: Option<String>,
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

    let mut init_opts = InitOptions::default();
    if let Some(tag) = args.rbac_tag.as_ref() {
        let mut headers = HashMap::new();
        headers.insert("x-iii-rbac-tag".to_string(), tag.clone());
        init_opts.headers = Some(headers);
        tracing::info!(
            rbac_tag = %tag,
            "Forwarding rbac-tag in worker headers; configure your auth_function_id to read x-iii-rbac-tag"
        );
    }

    let iii = register_worker(&args.engine_url, init_opts);

    let identity = handler::AgentIdentity {
        name: args.agent_name,
        description: args.agent_description,
        provider_org: args.provider_org,
        provider_url: args.provider_url,
        docs_url: args.docs_url,
    };
    handler::register(&iii, args.base_url, identity);

    tracing::info!("A2A endpoints registered on engine port. Ctrl+C to stop.");
    tokio::signal::ctrl_c().await?;

    Ok(())
}
