//! Boot.
//!
//! Order matters and is asserted by a test: the **interface** — functions,
//! the configuration doorbell — registers before the durable dependencies are
//! claimed. Release interface capture boots this binary against an isolated
//! engine that has no `database` and no `queue`, and expects a non-empty
//! surface within seconds; claiming the store first would hang that capture
//! and hide the worker from the registry.
//!
//! The dependencies are then claimed in the background, and ingest stays
//! closed until they answer (`sentinel::status.enabled`).

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_helpers::observability::OtelConfig;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use sentinel::{configuration, dependencies, functions, manifest, Counters};
use tokio::sync::RwLock;

#[derive(Debug, Parser)]
#[command(
    name = "sentinel",
    about = "Error monitoring for iii: group failures, freeze the evidence, investigate with the harness"
)]
struct Cli {
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    #[arg(long)]
    manifest: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest::build_manifest())
                .expect("manifest must serialize")
        );
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                name: "sentinel".into(),
                os: std::env::consts::OS.into(),
                description: Some(manifest::DESCRIPTION.into()),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            // Named explicitly: the SDK would otherwise report the binary's
            // file name as `service.name`, and this worker attributes errors
            // by service name — including its own, which it must recognise to
            // avoid ingesting itself.
            otel: Some(OtelConfig {
                service_name: Some("sentinel".into()),
                service_version: Some(env!("CARGO_PKG_VERSION").into()),
                ..OtelConfig::default()
            }),
            ..InitOptions::default()
        },
    ));

    let (config, config_error) = configuration::register_and_fetch_until_ready(&iii).await;
    let counters = Arc::new(Counters::default());
    let cell: sentinel::ConfigCell = Arc::new(RwLock::new(Arc::new(config)));
    let error_cell: sentinel::ConfigErrorCell = Arc::new(RwLock::new(config_error));
    let deps = Arc::new(functions::Deps {
        config: cell.clone(),
        config_error: error_cell.clone(),
        counters: counters.clone(),
    });

    functions::register_all(&iii, &deps);

    match configuration::bind_reload(&iii, cell.clone(), error_cell.clone()) {
        // One extra pass closes the boot gap: an update that landed between
        // the fetch above and this binding fired into nothing.
        Ok(reload) => reload.run().await,
        Err(error) => tracing::warn!(
            %error,
            "configuration updates are not bound; settings changes need a restart until it recovers"
        ),
    }

    let readiness = tokio::spawn(dependencies::wait_until_ready(
        iii.clone(),
        counters.clone(),
    ));

    tracing::info!(
        config_id = configuration::config_id(),
        "sentinel ready; ingest opens when the store and queue answer"
    );
    tokio::signal::ctrl_c().await?;
    readiness.abort();
    let _ = readiness.await;
    iii.shutdown_async().await;
    Ok(())
}
