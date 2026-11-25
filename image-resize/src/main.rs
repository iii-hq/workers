use std::num::NonZero;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{InitOptions, RegisterFunction, register_worker};
use image_resize::{config, configuration, handler};
use tokio::sync::Semaphore;

#[derive(Parser, Debug)]
#[command(name = "image-resize", about = "III engine image resize module")]
struct Cli {
    /// YAML seed for the configuration entry, applied only while nothing is stored yet.
    #[arg(long)]
    config: Option<String>,

    /// WebSocket URL of the III engine (port 49134 = engine main WS)
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,

    /// Namespace to register under. Absent uses the engine's default namespace.
    /// Falls back to the III_NAMESPACE env var.
    #[arg(long, env = "III_NAMESPACE")]
    namespace: Option<String>,
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

    tracing::info!(url = %cli.url, namespace = ?cli.namespace, "connecting to III engine");

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            namespace: cli.namespace.clone(),
            ..Default::default()
        },
    ));

    let seed = cli.config.as_deref().and_then(config::load_seed);
    configuration::register(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering the image-resize configuration")?;
    let shared = configuration::shared(
        configuration::fetch(&iii)
            .await
            .map_err(anyhow::Error::msg)
            .context("reading the image-resize configuration")?,
    );

    let permits = Arc::new(Semaphore::new(
        std::thread::available_parallelism().map_or(1, NonZero::get),
    ));

    let _fn_ref = iii.register_function(
        "image_resize::resize",
        RegisterFunction::new_async({
            let shared = shared.clone();
            move |req: handler::ResizeRequest| {
                handler::resize(configuration::snapshot(&shared), permits.clone(), req)
            }
        })
        .description(
            "Downscale a JPEG, PNG or WebP image read from input_channel to fit or fill a box, never enlarging it, and write JPEG, PNG or lossy WebP bytes to output_channel.",
        ),
    );

    // Last, so the reload handler closes over fully built state.
    configuration::bind_reload(&iii, shared).run().await;

    tracing::info!("image_resize::resize function registered, waiting for invocations");

    tokio::signal::ctrl_c().await?;

    tracing::info!("image-resize shutting down");
    iii.shutdown_async().await;

    Ok(())
}
