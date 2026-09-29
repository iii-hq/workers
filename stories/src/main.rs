//! `stories` binary entry.
//!
//! Boot: parse the CLI, connect to the engine, register the trigger types,
//! register the `stories` configuration entry and load it, materialize the
//! Node compiler under the data folder, register the `stories::*` functions
//! and the console UI, start the working-tree watchers, then sleep until
//! SIGINT/SIGTERM.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{InitOptions, register_worker};

use iii_stories::builder::{self, Ctx};
use iii_stories::compiler::Compiler;
use iii_stories::events::Subscribers;
use iii_stories::store::Store;
use iii_stories::{configuration, functions, ui, watch};

#[derive(Parser, Debug)]
#[command(name = "stories", about = "Component stories worker for iii.")]
struct Cli {
    /// Optional seed config.yaml, used only when no `stories` configuration
    /// is stored yet.
    #[arg(long)]
    config: Option<String>,
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
}

fn worker_metadata() -> WorkerMetadata {
    WorkerMetadata {
        runtime: "rust".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: "stories".to_string(),
        os: std::env::consts::OS.to_string(),
        pid: Some(std::process::id()),
        telemetry: None,
        ..WorkerMetadata::default()
    }
}

fn read_seed(path: &str) -> Option<serde_json::Value> {
    match std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|raw| serde_yaml::from_str::<serde_json::Value>(&raw).map_err(|e| e.to_string()))
    {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(
                path,
                error,
                "failed to load --config seed; continuing without it"
            );
            None
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let cli = Cli::parse();
    let seed = cli.config.as_deref().and_then(read_seed);

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(worker_metadata()),
            ..InitOptions::default()
        },
    ));

    let config = Arc::new(RwLock::new(iii_stories::config::StoriesConfig::default()));
    let subscribers = Subscribers::default();
    iii_stories::events::register_trigger_types(&iii, &subscribers);

    configuration::register(&iii, seed)
        .await
        .map_err(anyhow::Error::msg)
        .context("registering stories configuration schema")?;
    let reload = configuration::bind_reload(&iii, config.clone())
        .map_err(anyhow::Error::msg)
        .context("binding configuration trigger")?;
    reload.run().await;

    let data_dir = config
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .data_path_resolved();
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("creating {}", data_dir.display()))?;
    let store = Store::new(data_dir.clone());
    let compiler = Compiler::ensure(store.compiler_dir())
        .await
        .map_err(anyhow::Error::msg)
        .context("preparing the stories compiler")?;

    let (rebuild_tx, rebuild_rx) = tokio::sync::mpsc::unbounded_channel();
    let ctx = Arc::new(Ctx {
        iii: iii.clone(),
        config: config.clone(),
        store,
        compiler,
        subscribers,
        build_lock: tokio::sync::Mutex::new(()),
        builds: RwLock::new(HashMap::new()),
        rebuild_tx,
    });
    functions::register_functions(&ctx);
    ui::register(&ctx);

    tokio::spawn(builder::build_loop(ctx.clone(), rebuild_rx));
    watch::spawn_all(ctx.clone());
    tracing::info!(data_dir = %data_dir.display(), "stories ready");

    wait_for_shutdown_signal().await?;
    tracing::info!("stories shutting down");
    iii.shutdown_async().await;
    Ok(())
}

async fn wait_for_shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm = signal(SignalKind::terminate())?;
        tokio::select! {
            r = tokio::signal::ctrl_c() => r,
            _ = sigterm.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}
