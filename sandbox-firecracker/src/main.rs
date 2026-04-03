use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

mod config;
mod exec;
mod functions;
mod kvm;
mod manifest;
mod types;

use types::VmInstance;

pub type VmRegistry = Arc<RwLock<HashMap<String, Arc<Mutex<VmInstance>>>>>;

#[derive(Parser, Debug)]
#[command(
    name = "sandbox-firecracker",
    about = "III engine KVM CoW fork sandbox"
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
        let manifest = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&manifest).unwrap());
        return Ok(());
    }

    let sandbox_config = match config::load_config(&cli.config) {
        Ok(c) => {
            tracing::info!(
                mem_size_mb = c.mem_size_mb,
                max_sandboxes = c.max_sandboxes,
                default_timeout = c.default_timeout,
                "loaded config from {}",
                cli.config
            );
            c
        }
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::SandboxConfig::default()
        }
    };

    let template = kvm::template::Template::load(
        &sandbox_config.vmstate_path,
        &sandbox_config.memfile_path,
        sandbox_config.mem_size_mb,
    )?;

    let template = Arc::new(template);
    let config = Arc::new(sandbox_config);
    let registry: VmRegistry = Arc::new(RwLock::new(HashMap::new()));

    tracing::info!(url = %cli.url, "connecting to III engine");

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            ..Default::default()
        },
    );

    functions::register_all(
        &iii,
        &cli.url,
        template.clone(),
        config.clone(),
        registry.clone(),
    );

    tracing::info!("all functions registered, waiting for invocations");

    tokio::signal::ctrl_c().await?;

    tracing::info!("shutting down, cleaning up VMs");
    {
        let mut map = registry.write().await;
        map.clear();
    }
    iii.shutdown_async().await;

    Ok(())
}
