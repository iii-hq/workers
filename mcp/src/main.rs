use std::collections::HashMap;
use std::sync::Arc;

use clap::Parser;
use iii_mcp::handler;
use iii_mcp::transport;
use iii_sdk::{InitOptions, register_worker};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[derive(Parser, Debug)]
#[command(name = "iii-mcp")]
#[command(version)]
#[command(about = "MCP protocol worker for iii-engine")]
struct Args {
    #[arg(long, short = 'e', default_value = "ws://localhost:49134")]
    engine_url: String,

    #[arg(long, short = 'd')]
    debug: bool,

    #[arg(long, help = "Skip stdio, run as HTTP-only (POST /mcp on engine port)")]
    no_stdio: bool,

    #[arg(
        long,
        help = "Hide the 6 built-in management tools (iii_worker_register, \
                iii_worker_stop, iii_trigger_*) from stdio. HTTP hides them \
                by default — see --http-builtins to opt in."
    )]
    no_builtins: bool,

    #[arg(
        long,
        help = "Opt in to exposing built-in management tools on the HTTP \
                transport. Default: HTTP hides them (worker/trigger management \
                requires stdio-attached process anyway)."
    )]
    http_builtins: bool,

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
        EnvFilter::new("iii_mcp=debug,iii_sdk=debug")
    } else {
        EnvFilter::new("iii_mcp=info,iii_sdk=warn")
    };

    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(filter)
        .init();

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Starting iii-mcp");

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

    // HTTP transport hides builtins by default — worker/trigger management
    // needs the stdio-attached process anyway, so exposing them over HTTP
    // was pure noise that errored on invocation. Opt in with
    // --http-builtins when a deploy genuinely needs it. --no-builtins
    // still wins (forces hidden everywhere). stdio keeps the default of
    // showing builtins (common Claude Desktop path).
    let http_no_builtins = args.no_builtins || !args.http_builtins;
    handler::register_http(&iii, http_no_builtins);

    if args.no_stdio {
        tracing::info!("MCP HTTP-only mode. POST /mcp on engine port. Ctrl+C to stop.");
        tokio::signal::ctrl_c().await?;
    } else {
        let h = Arc::new(handler::McpHandler::new(
            iii,
            args.engine_url,
            args.no_builtins,
        ));
        transport::run_stdio(h).await?;
    }

    Ok(())
}
