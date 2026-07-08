//! `console` worker entry point.
//!
//! Boot sequence:
//!   1. Parse CLI / load YAML config (with fallback to defaults).
//!   2. Connect to the iii engine over WebSocket via the SDK.
//!   3. Register `console::status` against the engine.
//!   4. Bind the HTTP server on `http_port` and serve `/`, `/assets/*`,
//!      and `/ws` (WebSocket proxy back to the engine).
//!   5. Wait for SIGINT or SIGTERM, then signal a graceful HTTP
//!      shutdown and `iii.shutdown_async()`.

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use tokio::sync::oneshot;

use console::{config, configuration, functions, manifest, server};

#[derive(Parser, Debug)]
#[command(
    name = "console",
    about = "Web console for iii — bundles the React UI and proxies the engine WebSocket on a single port."
)]
struct Cli {
    /// Path to `config.yaml`.
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    /// iii engine WebSocket URL.
    #[arg(long, default_value = config::DEFAULT_ENGINE_URL)]
    url: String,

    /// TCP port for `/`, `/assets/*`, and `/ws`. Overrides `http_port`
    /// from the config file when present.
    #[arg(long)]
    http_port: Option<u16>,

    /// Print the publish manifest as JSON and exit. Used by the
    /// registry publish pipeline; no engine connection.
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

    let mut cfg = match config::load_config(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %cli.config,
                "failed to load config, using defaults"
            );
            config::ConsoleConfig::default()
        }
    };
    if let Some(port) = cli.http_port {
        cfg.http_port = port;
    }

    let engine_url = cli.url;

    tracing::info!(
        http_port = cfg.http_port,
        engine_url = %redact_url(&engine_url),
        "starting console worker"
    );

    let cfg = Arc::new(cfg);

    let iii = register_worker(
        &engine_url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "console".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    functions::register_all(&iii, &cfg, &engine_url);

    // Best-effort: guarantee the `console` configuration entry exists for the
    // UI's saved preferences, without delaying HTTP serve.
    {
        let iii = iii.clone();
        tokio::spawn(async move {
            configuration::register_console_config(&iii).await;
        });
    }

    if !console::assets::has_bundle() {
        tracing::warn!(
            "embedded SPA bundle is empty — `/` and `/assets/*` will return 404. \
             Run `pnpm install && pnpm build` in `web/` and rebuild."
        );
    }

    let (http_shutdown_tx, http_shutdown_rx) = oneshot::channel::<()>();
    let engine_url_redacted = redact_url(&engine_url);
    let engine_url_for_proxy = Arc::new(engine_url);
    let http_port = cfg.http_port;
    let server_handle = tokio::spawn(async move {
        if let Err(e) = server::serve(http_port, engine_url_for_proxy, http_shutdown_rx, None).await
        {
            tracing::error!(error = %e, "http server crashed");
        }
    });

    tracing::info!(
        "console ready — UI on http://127.0.0.1:{}/, /ws proxies to {}",
        cfg.http_port,
        engine_url_redacted,
    );

    wait_for_shutdown_signal().await?;
    tracing::info!("console shutting down");

    let _ = http_shutdown_tx.send(());
    if let Err(e) = server_handle.await {
        tracing::warn!(error = %e, "http server task join failed");
    }

    iii.shutdown_async().await;
    Ok(())
}

/// Wait for SIGINT or, on Unix, SIGTERM. `tokio::signal::ctrl_c()`
/// alone only catches SIGINT, leaving Docker `docker stop` / k8s
/// `kubectl delete` (which send SIGTERM) to bypass shutdown entirely.
async fn wait_for_shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
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

/// Strip userinfo (username:password) from a URL before logging it. The
/// engine WebSocket URL is operator-controlled and can carry credentials
/// in `wss://user:secret@host` form; the redactor keeps them out of
/// `tracing` output. Falls back to the original string on parse failure.
fn redact_url(s: &str) -> String {
    match url::Url::parse(s) {
        Ok(mut u) => {
            let _ = u.set_username("");
            let _ = u.set_password(None);
            u.to_string()
        }
        Err(_) => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::redact_url;

    #[test]
    fn redact_url_strips_userinfo_only() {
        assert_eq!(redact_url("ws://127.0.0.1:49134"), "ws://127.0.0.1:49134/");
        assert_eq!(
            redact_url("wss://user:secret@iii.example.com:1234/path"),
            "wss://iii.example.com:1234/path"
        );
        assert_eq!(redact_url("not a url"), "not a url");
    }
}
