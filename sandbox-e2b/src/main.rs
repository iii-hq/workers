use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, RegisterFunctionMessage};
use serde_json::Value;

use sandbox_e2b::client::E2bClient;
use sandbox_e2b::config::Config;
use sandbox_e2b::handler::{
    do_create, do_exec, do_expose_port, do_fs_read, do_fs_write, do_list, do_snapshot, do_stop,
    to_iii, HandlerCtx,
};

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

#[derive(Parser, Debug)]
#[command(name = "iii-sandbox-e2b")]
struct Cli {
    /// Path to config.yaml.
    #[arg(long, default_value = "./config.yaml")]
    config: PathBuf,
    /// WebSocket URL of the iii engine.
    #[arg(long, default_value = DEFAULT_ENGINE_URL)]
    url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let cfg = Config::load(&cli.config).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "failed to load config, using defaults");
        Config::default()
    });
    let api_key = std::env::var(&cfg.api_key_env)
        .map_err(|_| anyhow!("env var {} is not set", cfg.api_key_env))?;
    let client = Arc::new(E2bClient::new(cfg.api_base.clone(), api_key));
    let ctx = HandlerCtx::new(Arc::new(cfg), client);

    let iii = register_worker(&cli.url, InitOptions::default());

    register_all(&iii, ctx);

    tracing::info!("sandbox-e2b registered, awaiting invocations");
    tokio::signal::ctrl_c().await?;
    iii.shutdown_async().await;
    Ok(())
}

fn register_all(iii: &iii_sdk::III, ctx: HandlerCtx) {
    macro_rules! reg {
        ($id:expr, $desc:expr, $fn:ident) => {{
            let ctx = ctx.clone();
            iii.register_function_with(
                RegisterFunctionMessage {
                    id: $id.to_string(),
                    description: Some($desc.to_string()),
                    request_format: None,
                    response_format: None,
                    metadata: None,
                    invocation: None,
                },
                move |input: Value| {
                    let ctx = ctx.clone();
                    async move { $fn(&ctx, input).await.map_err(to_iii) }
                },
            );
        }};
    }

    reg!(
        "sandbox::provider::e2b::create",
        "Boot an E2B sandbox; returns {sandbox_id, image, capabilities}",
        do_create
    );
    reg!(
        "sandbox::provider::e2b::exec",
        "Run a command inside a live sandbox",
        do_exec
    );
    reg!("sandbox::provider::e2b::stop", "Tear down a sandbox", do_stop);
    reg!(
        "sandbox::provider::e2b::list",
        "List live sandboxes plus concurrency status",
        do_list
    );
    reg!(
        "sandbox::provider::e2b::snapshot",
        "Pause a sandbox into a resumable snapshot",
        do_snapshot
    );
    reg!(
        "sandbox::provider::e2b::expose_port",
        "Return a public URL for a port inside the sandbox",
        do_expose_port
    );
    reg!(
        "sandbox::provider::e2b::fs::read",
        "Read a file out of a sandbox; returns base64-encoded bytes",
        do_fs_read
    );
    reg!(
        "sandbox::provider::e2b::fs::write",
        "Write a file into a sandbox; payload carries base64-encoded bytes",
        do_fs_write
    );
}
