use std::sync::Arc;

use clap::Parser;
use iii_acp::handler::{AcpHandler, BrainConfig, DEFAULT_BRAIN_FN};
use iii_acp::transport;
use iii_sdk::{InitOptions, register_worker};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[derive(Parser, Debug)]
#[command(name = "iii-acp")]
#[command(version)]
#[command(about = "Agent Client Protocol worker for iii-engine")]
struct Args {
    #[arg(
        long,
        short = 'e',
        env = "III_URL",
        default_value = "ws://localhost:49134"
    )]
    engine_url: String,

    #[arg(long, short = 'd')]
    debug: bool,

    #[arg(
        long,
        env = "IIIACP_BRAIN_FN",
        help = "iii function id that processes session/prompt. Defaults to \
                run::start_and_wait when --use-canonical-brain is set; off \
                otherwise (built-in echo brain). Any function with the \
                turn-orchestrator wire shape (session_id, messages, model, \
                ...) emitting AgentEvent frames into the agent::events \
                stream will work."
    )]
    brain_fn: Option<String>,

    #[arg(
        long,
        env = "IIIACP_USE_CANONICAL_BRAIN",
        help = "Shortcut for --brain-fn run::start_and_wait. Wires iii-acp \
                straight to turn-orchestrator. Ignored if --brain-fn is \
                already set."
    )]
    use_canonical_brain: bool,

    #[arg(
        long,
        env = "IIIACP_MODEL",
        help = "Model id passed to the brain (e.g., claude-opus-4-7). \
                Required when the brain talks to a multi-model provider."
    )]
    model: Option<String>,

    #[arg(
        long,
        env = "IIIACP_PROVIDER",
        help = "Provider id passed to the brain (e.g., anthropic, openai). \
                Routes to the matching provider::<provider>::complete worker."
    )]
    provider: Option<String>,

    #[arg(
        long,
        env = "IIIACP_SYSTEM_PROMPT",
        help = "System prompt prepended to the agent's transcript on every \
                session/prompt turn."
    )]
    system_prompt: Option<String>,

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

    // Honor RUST_LOG when set (gives operators per-module control); fall
    // back to the --debug switch otherwise.
    let fallback = if args.debug {
        "iii_acp=debug,iii_sdk=debug"
    } else {
        "iii_acp=info,iii_sdk=warn"
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(fallback));

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
    let brain_fn = args.brain_fn.or_else(|| {
        args.use_canonical_brain
            .then(|| DEFAULT_BRAIN_FN.to_string())
    });
    let handler = Arc::new(AcpHandler::new(
        iii,
        outbound,
        BrainConfig {
            function_id: brain_fn,
            model: args.model,
            provider: args.provider,
            system_prompt: args.system_prompt,
        },
    ));

    transport::run_stdio(handler).await?;

    Ok(())
}
