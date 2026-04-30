use std::sync::Arc;

use clap::Parser;
use iii_acp::handler::AcpHandler;
use iii_acp::transport;
use iii_sdk::{InitOptions, register_worker};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[derive(Parser, Debug)]
#[command(name = "iii-acp")]
#[command(version)]
#[command(about = "Agent Client Protocol worker for iii-engine")]
struct Args {
    #[arg(long, short = 'e', default_value = "ws://localhost:49134")]
    engine_url: String,

    #[arg(long, short = 'd')]
    debug: bool,

    #[arg(
        long,
        env = "IIIACP_BRAIN_FN",
        help = "iii function id that processes session/prompt. \
                Receives { sessionId, connId, prompt, respondTopic } and \
                returns { stopReason }. Falls back to a built-in echo \
                brain when unset."
    )]
    brain_fn: Option<String>,

    #[arg(
        long,
        env = "IIIACP_PUBLISH_UPDATES",
        help = "Also publish session/update notifications to the engine \
                durable topic acp:<connId>:session:<sessId>:updates so \
                external observers can subscribe. Stdout delivery is \
                always on; this is opt-in for fan-out."
    )]
    publish_updates: bool,

    #[arg(
        long,
        value_name = "TAG",
        help = "Forward an `x-iii-rbac-tag` header on the worker WebSocket \
                upgrade. iii-worker-manager's `auth_function_id` reads this \
                tag to apply policy."
    )]
    rbac_tag: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let filter = if args.debug {
        EnvFilter::new("iii_acp=debug,iii_sdk=debug")
    } else {
        EnvFilter::new("iii_acp=info,iii_sdk=warn")
    };

    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(filter)
        .init();

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting iii-acp");

    let mut init_opts = InitOptions::default();
    if let Some(tag) = args.rbac_tag.as_ref() {
        let mut headers = std::collections::HashMap::new();
        headers.insert("x-iii-rbac-tag".to_string(), tag.clone());
        init_opts.headers = Some(headers);
    }

    let iii = register_worker(&args.engine_url, init_opts);

    let outbound = Arc::new(transport::Outbound::new());
    let handler = Arc::new(AcpHandler::new(
        iii,
        outbound,
        args.brain_fn,
        args.publish_updates,
    ));

    transport::run_stdio(handler).await?;

    Ok(())
}
