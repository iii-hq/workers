//! `skills` binary entry.
//!
//! Boot sequence:
//!   1. Parse CLI / load YAML config (with fallback to defaults).
//!   2. Connect to the iii engine over WebSocket.
//!   3. Register the custom trigger types `skills::on-change` / `prompts::on-change`.
//!   4. Register every `skills::*` and `prompts::*` function against the engine.
//!   5. Sleep on Ctrl+C, then `shutdown_async` cleanly.
//!
//! All public CRUD functions are reachable over `iii.trigger` from any
//! sibling worker. Internal `skills::resources-*` / `prompts::mcp-*`
//! are called by the mcp worker over the iii bus only.

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig};

use iii_skills::{config, functions, manifest, trigger_types};

#[derive(Parser, Debug)]
#[command(
    name = "skills",
    about = "Agentic content registry worker. Hosts skills + prompts + the iii:// resource resolver."
)]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, default_value = "ws://127.0.0.1:49134")]
    url: String,

    #[arg(long)]
    manifest: bool,
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

    if cli.manifest {
        let m = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m).unwrap());
        return Ok(());
    }

    let cfg = match config::load_config(&cli.config) {
        Ok(c) => {
            tracing::info!(
                skills_scope = %c.scopes.skills,
                prompts_scope = %c.scopes.prompts,
                "loaded config from {}",
                cli.config
            );
            c
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::SkillsConfig::default()
        }
    };
    let cfg = Arc::new(cfg);

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );
    let iii = Arc::new(iii);

    // Custom trigger types come first because the function handlers
    // capture the subscriber sets they'll fan out to.
    let registered = trigger_types::register_all(&iii);
    functions::register_all(&iii, &cfg, &registered);

    tracing::info!("skills ready: 10 functions + 2 custom trigger types");

    tokio::signal::ctrl_c().await?;
    tracing::info!("skills shutting down");
    iii.shutdown_async().await;
    Ok(())
}
