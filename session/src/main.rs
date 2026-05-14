use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, WorkerMetadata};

use session::{config, inbox, manifest, tree};

#[derive(Parser, Debug)]
#[command(name = "session", about = "Session storage + inbox worker for iii")]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    /// Engine WebSocket URL. Overrides `engine_url` in `--config` when set (or use `III_URL`).
    #[arg(long, env = "III_URL")]
    url: Option<String>,

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
        println!("{}", serde_json::to_string_pretty(&m)?);
        return Ok(());
    }

    let cfg = match config::load_config(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::SessionConfig::default()
        }
    };

    let url = cli.url.unwrap_or_else(|| cfg.engine_url.clone());

    let iii = Arc::new(register_worker(
        &url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "session".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

    // session-tree::* surface
    let store: Arc<dyn tree::SessionStore> = match cfg.store_backend.as_str() {
        "memory" => {
            tracing::info!("session-tree: using in-memory store (store_backend=memory)");
            Arc::new(tree::InMemoryStore::default())
        }
        "iii_state" => {
            tracing::info!("session-tree: using iii-state store");
            let iii_for_store: Arc<dyn tree::io::IIITrigger> = iii.clone();
            Arc::new(tree::store_iii_state::IiiStateSessionStore::new(
                iii_for_store,
            ))
        }
        other => {
            tracing::warn!(
                unknown_backend = other,
                "unknown store_backend; falling back to iii_state (valid: memory, iii_state)"
            );
            let iii_for_store: Arc<dyn tree::io::IIITrigger> = iii.clone();
            Arc::new(tree::store_iii_state::IiiStateSessionStore::new(
                iii_for_store,
            ))
        }
    };

    let _tree_refs = tree::register_with_iii(&iii, store);
    tracing::info!("session-tree registered (session-tree::*)");

    // session-inbox::* surface
    let inbox_cfg = Arc::new(inbox::config::WorkerConfig {
        engine_url: cfg.engine_url.clone(),
        state_scope: cfg.state_scope.clone(),
    });
    inbox::register_with_iii(&iii, &inbox_cfg);
    tracing::info!(
        "session-inbox registered ({}, {}, {})",
        inbox::PUSH_ID,
        inbox::DRAIN_ID,
        inbox::PEEK_ID
    );

    wait_for_shutdown().await?;

    iii.shutdown_async().await;
    Ok(())
}

async fn wait_for_shutdown() -> Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).context("failed to install SIGTERM handler")?;
        tokio::select! {
            r = tokio::signal::ctrl_c() => r.context("failed to await SIGINT")?,
            _ = sigterm.recv() => {}
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .context("failed to await SIGINT")
    }
}
