//! `browser` binary entry: connect, register configuration + fetch the
//! authoritative value, register the `browser::*` trigger types and functions
//! plus the native `browser::*` parse surface, restore the saved tabs, then
//! sleep until Ctrl+C. Tabs sleep and expire on their own deadline timers
//! (see `session::Sessions`), and scrapling sessions reap themselves when
//! idle (see `scrapling::sessions`): no periodic sweep.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};

use browser::config::{BrowserEngine, WorkerConfig};
use browser::events::{self, IiiDeliverer};
use browser::session::Sessions;
use browser::{configuration, functions, manifest, scrapling};

#[derive(Parser, Debug)]
#[command(
    name = "browser",
    about = "A browser on the iii bus (browser::*): tabs agents and people share."
)]
struct Cli {
    /// Optional YAML seed used to populate `initial_value` on first registration.
    #[arg(long)]
    config: Option<String>,
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,
    #[arg(long)]
    manifest: bool,
}

/// SIGINT and SIGTERM both shut down cleanly: sessions own Chromium
/// processes and temp profiles, and `kill`/`docker stop`/the worker manager
/// all deliver SIGTERM, so ctrl_c alone would orphan them.
async fn wait_for_shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            r = tokio::signal::ctrl_c() => r?,
            _ = sigterm.recv() => {}
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let rust_log = std::env::var("RUST_LOG").ok();
    tracing_subscriber::fmt()
        .with_env_filter(browser::logging::env_filter(rust_log.as_deref()))
        .init();

    let cli = Cli::parse();
    if cli.manifest {
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest::build_manifest()).unwrap()
        );
        return Ok(());
    }

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "browser".to_string(),
                os: std::env::consts::OS.to_string(),
                description: Some(
                    "A browser on the iii bus (browser::*): tabs agents and people share."
                        .to_string(),
                ),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

    // Optional YAML seed → initial_value on first registration.
    let seed = cli.config.as_deref().and_then(|path| {
        let contents = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(path, error = %e, "failed to read seed config; ignoring");
                return None;
            }
        };
        match serde_yaml::from_str::<serde_json::Value>(&contents) {
            Ok(v) => match WorkerConfig::from_json(&v) {
                Ok(cfg) => Some(cfg),
                Err(e) => {
                    tracing::warn!(path, error = %e, "failed to parse seed config; ignoring");
                    None
                }
            },
            Err(e) => {
                tracing::warn!(path, error = %e, "failed to parse YAML seed config; ignoring");
                None
            }
        }
    });

    configuration::register_config(&iii, seed.as_ref())
        .await
        .map_err(anyhow::Error::msg)
        .context("registering browser configuration schema")?;
    let cfg = configuration::fetch_config(&iii)
        .await
        .map_err(anyhow::Error::msg)
        .context("loading browser configuration")?;
    let scrapling_startup = cfg.scrapling.startup_snapshot();
    scrapling::adaptive::configure(&scrapling_startup.adaptive_storage_path)
        .map_err(anyhow::Error::msg)
        .context("configuring Scrapling adaptive storage")?;
    scrapling::adaptive::configure_quota(cfg.scrapling.adaptive_quota());
    tracing::info!(
        headless = cfg.headless,
        max_sessions = cfg.max_sessions,
        console_buffer = cfg.console_buffer,
        scrapling_security_mode = ?cfg.scrapling.security_mode,
        scrapling_max_sessions = scrapling_startup.max_sessions,
        scrapling_session_idle_timeout_s = scrapling_startup.session_idle_timeout_s,
        scrapling_adaptive_storage_path = %scrapling_startup.adaptive_storage_path,
        "loaded browser configuration"
    );
    // Say it now, not on the first tab: without a Chromium every session
    // fails until one is installed (browser::chromium::install, or the ADE's
    // setup wizard).
    if cfg.engine == BrowserEngine::Chromium {
        match browser::chromium::resolve_executable(&cfg) {
            Some(found) => tracing::info!(
                path = %found.path.display(),
                source = found.source.as_str(),
                "chromium found"
            ),
            None => tracing::warn!("{}", browser::chromium::missing_error(&cfg)),
        }
    }
    let shared = cfg.into_shared();

    // Trigger types before functions, so handlers capture live subscriber sets.
    let sets = events::register_trigger_types(&iii);
    let emitter = Arc::new(events::Emitter::new(
        sets,
        Arc::new(IiiDeliverer::new(iii.clone())),
    ));

    let sessions = Sessions::new(shared.clone(), emitter, iii.clone());
    functions::register_all(&iii, &sessions);

    // Scrapling owns a private HTTP/dynamic/stealthy registry. Its ids never
    // enter or control the interactive browser::sessions::* registry.
    let scrapling_ctx = Arc::new(scrapling::net::Ctx::new(sessions.clone(), iii.clone()));
    scrapling::register_all(&iii, &scrapling_ctx);
    // The guidance hook FUNCTION is registered above (inert without a
    // binding); the binding follows the inject_guidance knob — applied here
    // at boot and re-applied by the config-change handler, so console flips
    // take effect without a restart.
    let guidance = scrapling::GuidanceState::default();
    scrapling::apply_guidance(&iii, &guidance, shared.load().scrapling.inject_guidance);

    // A reload may change `inactive_after_ms`: the tabs' idle timers re-arm.
    let rearm = sessions.clone();
    configuration::register_config_trigger(
        &iii,
        shared.clone(),
        guidance,
        Arc::new(move || rearm.config_changed()),
    )
    .context("registering configuration change trigger")?;

    // Injectable console UI — after the browser::* functions so the console
    // can attribute the assets.
    browser::ui::register(&iii);

    tracing::info!(
        "browser ready: browser::* sessions + console capture + pick, browser::* parsing"
    );
    wait_for_shutdown_signal().await?;
    tracing::info!("browser shutting down");
    scrapling_ctx.http.close_all().await;
    sessions.stop_all().await;
    iii.shutdown_async().await;
    Ok(())
}
