//! `console` worker entry point.
//!
//! Boot sequence:
//!   1. Parse CLI / load the optional YAML seed (with fallback to defaults).
//!   2. Connect to the iii engine over WebSocket via the SDK.
//!   3. Register the Console configuration schema and fetch its authoritative
//!      `http_port` (falling back to the local seed if unavailable).
//!   4. Register `console::status` against the engine.
//!   5. Bind the HTTP server on `http_port` and serve `/`, `/assets/*`,
//!      and `/ws` (WebSocket proxy back to the engine).
//!   6. Subscribe to configuration changes so port edits rebind live.
//!   7. Wait for SIGINT or SIGTERM, then signal a graceful HTTP
//!      shutdown and `iii.shutdown_async()`.

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};

use console::{config, configuration, functions, manifest, server, ui, ui_assets};

#[derive(Parser, Debug)]
#[command(
    name = "console",
    about = "Web console for iii — bundles the React UI and proxies the engine WebSocket on a single port."
)]
struct Cli {
    /// Path to the YAML seed used on first configuration registration.
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    /// iii engine WebSocket URL.
    #[arg(long, env = "III_URL", default_value = config::DEFAULT_ENGINE_URL)]
    url: String,

    /// TCP port seed for `/`, `/assets/*`, and `/ws`. Overrides `http_port`
    /// from the seed file; an existing configuration-worker value still wins.
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

    // Match the HTTP worker's configuration lifecycle: local config is only a
    // first-registration seed/fallback; a stored value is authoritative and is
    // fetched before binding the listener.
    if let Err(error) = configuration::register_console_config(&iii, cfg.http_port).await {
        tracing::warn!(
            %error,
            "console configuration registration failed; continuing with the local port seed"
        );
    }
    let runtime_config = match configuration::fetch_runtime_config(&iii, cfg.http_port).await {
        Ok(config) => config,
        Err(error) => {
            tracing::warn!(
                %error,
                "console configuration fetch failed; continuing with the local port seed"
            );
            configuration::RuntimeConfig::fallback(cfg.http_port)
        }
    };
    cfg.http_port = runtime_config.http_port;

    tracing::info!(
        http_port = cfg.http_port,
        engine_url = %redact_url(&engine_url),
        "starting console worker"
    );

    let cfg = Arc::new(cfg);
    let port = configuration::new_port_cell(cfg.http_port);

    // Trigger types register before functions (approval-gate/memory
    // ordering convention). `None` = injectable UI disabled via config.
    let (ui, ui_control) = if cfg.injectable_ui {
        let (registry, control) = ui_assets::start(&iii);
        (Some(registry), Some(control))
    } else {
        tracing::info!("injectable_ui disabled — skipping console:* trigger types and /ui routes");
        (None, None)
    };

    functions::register_all(&iii, port.clone(), &engine_url, ui.clone());

    // The console's own injected UI — live port + injectable-UI controls for
    // the `console` configuration entry. Same mechanism as any worker's.
    if cfg.injectable_ui {
        ui::register(&iii);
    }

    configuration::apply_runtime_ui(&runtime_config, ui_control.as_ref()).await;

    if !console::assets::has_bundle() {
        tracing::warn!(
            "embedded SPA bundle is empty — `/` and `/assets/*` will return 404. \
             Run `pnpm install && pnpm build` in `web/` and rebuild."
        );
    }

    let engine_url_redacted = redact_url(&engine_url);
    let state = server::AppState::new(Arc::new(engine_url), iii.namespace(), ui);
    let server_handle = server::start(cfg.http_port, state.clone()).await?;
    let apply_lock: configuration::ApplyLock = Arc::new(tokio::sync::Mutex::new(()));

    if let Err(error) = configuration::register_config_trigger(
        &iii,
        port.clone(),
        state.clone(),
        server_handle.control.clone(),
        ui_control.clone(),
        apply_lock.clone(),
    ) {
        tracing::warn!(
            %error,
            "console configuration trigger registration failed; live updates are disabled"
        );
    } else {
        // Close the fetch→subscribe race: an edit that landed while the server
        // was binding is reconciled through the same serialized apply path.
        configuration::apply_current_config(
            &iii,
            &port,
            &state,
            &server_handle.control,
            ui_control.as_ref(),
            &apply_lock,
        )
        .await;
    }

    let ready_addr = server_handle
        .current_addr()
        .await
        .unwrap_or(server_handle.local_addr);

    tracing::info!(
        "console ready — UI on http://127.0.0.1:{}/, /ws proxies to {}",
        ready_addr.port(),
        engine_url_redacted,
    );

    wait_for_shutdown_signal().await?;
    tracing::info!("console shutting down");

    // Serialize shutdown with any in-flight rebind. `rebind` also refuses to
    // install a replacement after the control slot becomes `None`.
    let _apply_guard = apply_lock.lock().await;
    server_handle.shutdown().await;
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
