use std::num::NonZero;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{InitOptions, RegisterFunction, register_worker};
use image_resize::{config, configuration, handler, ui};
use tokio::sync::Semaphore;

#[derive(Parser, Debug)]
#[command(name = "image-resize", about = "III engine image resize module")]
struct Cli {
    /// YAML seed for the configuration entry, applied only while nothing is stored yet.
    #[arg(long)]
    config: Option<String>,

    /// Engine WebSocket URL. Defaults to III_URL, then ws://127.0.0.1:49134.
    #[arg(long)]
    url: Option<String>,

    /// Namespace to register under. Defaults to III_NAMESPACE, then the engine's default.
    #[arg(long)]
    namespace: Option<String>,
}

/// Resolves on interrupt or terminate. A managed worker is stopped with
/// SIGTERM, so that listener is installed by this call, not by the first
/// poll: a signal sent once the worker reports ready is never lost.
#[cfg(unix)]
fn shutdown_signal() -> std::io::Result<impl Future<Output = std::io::Result<()>>> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate())?;
    Ok(async move {
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    })
}

#[cfg(not(unix))]
fn shutdown_signal() -> std::io::Result<impl Future<Output = std::io::Result<()>>> {
    Ok(tokio::signal::ctrl_c())
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
    let url = cli.url.unwrap_or_else(iii_sdk::engine_url_from_env);

    let iii = Arc::new(register_worker(
        &url,
        InitOptions {
            namespace: cli.namespace,
            ..Default::default()
        },
    ));
    tracing::info!(%url, namespace = ?iii.namespace(), "connecting to III engine");

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
    ui::register(&iii);

    // Last, so the reload handler closes over fully built state.
    configuration::bind_reload(&iii, shared).run().await;

    // Not before the configuration calls: a SIGTERM during a failing boot keeps its default action.
    let shutdown = shutdown_signal()?;
    tracing::info!("image_resize::resize function registered, waiting for invocations");
    shutdown.await?;

    tracing::info!("image-resize shutting down");
    // shutdown() joins the SDK connection thread, so the close frame and the OTel shutdown finish before main returns.
    tokio::task::spawn_blocking(move || iii.shutdown()).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_has_no_implicit_defaults() {
        let cli = Cli::try_parse_from(["image-resize"]).unwrap();
        assert_eq!((cli.config, cli.url, cli.namespace), (None, None, None));
    }
}
