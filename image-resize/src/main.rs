use std::num::NonZero;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{InitOptions, RegisterFunction, register_worker};
use image_resize::shutdown::{DRAIN_DEADLINE, InFlight};
use image_resize::{config, configuration, handler, ui};
use tokio::sync::Semaphore;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::prelude::*;

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

/// RUST_LOG as `level` and `target=level` directives; unset, empty or unparsable means info.
fn log_filter(var: Option<&str>) -> Targets {
    // Targets reads an empty directive as `error`, so "info," would log errors only.
    var.map(|v| {
        v.split(',')
            .filter(|d| !d.trim().is_empty())
            .collect::<Vec<_>>()
            .join(",")
    })
    .filter(|v| !v.is_empty())
    .and_then(|v| v.parse().ok())
    .unwrap_or_else(|| Targets::new().with_default(LevelFilter::INFO))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().log_internal_errors(true))
        .with(log_filter(std::env::var("RUST_LOG").ok().as_deref()))
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

    let calls = Arc::new(InFlight::default());
    let resize_fn = iii.register_function(
        "image_resize::resize",
        RegisterFunction::new_async({
            let shared = shared.clone();
            let calls = calls.clone();
            move |req: handler::ResizeRequest| {
                handler::resize(
                    configuration::snapshot(&shared),
                    permits.clone(),
                    calls.clone(),
                    req,
                )
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
    // Unregistered first, so the engine routes no new call while the ones in flight finish.
    resize_fn.unregister();
    let running = calls.drain(DRAIN_DEADLINE).await;
    if running > 0 {
        tracing::warn!(
            running,
            "calls still in flight at the drain deadline are cut off"
        );
    }
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

    #[test]
    fn rust_log_takes_levels_and_targets_and_falls_back_to_info() {
        use tracing::Level;
        let on = |var: Option<&str>, target: &str, level: Level| {
            log_filter(var).would_enable(target, &level)
        };
        assert!(on(None, "iii_sdk", Level::INFO) && !on(None, "iii_sdk", Level::DEBUG));
        assert!(on(Some("debug"), "rustls", Level::DEBUG));
        for var in ["info,", ",info", "info,,", ",", ""] {
            assert!(on(Some(var), "iii_sdk", Level::INFO), "{var:?}");
            assert!(!on(Some(var), "iii_sdk", Level::DEBUG), "{var:?}");
        }
        assert!(on(
            Some("warn,,image_resize=debug,"),
            "image_resize",
            Level::DEBUG
        ));
        let scoped = Some("image_resize=debug,iii_sdk=warn");
        assert!(on(scoped, "image_resize::handler", Level::DEBUG));
        assert!(!on(scoped, "iii_sdk::iii", Level::INFO) && !on(scoped, "rustls", Level::ERROR));
        let span_filter = Some("[request{id=7}]=debug");
        assert!(on(span_filter, "x", Level::INFO) && !on(span_filter, "x", Level::DEBUG));
    }
}
