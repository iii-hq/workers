//! `web` binary entry: connect, register configuration + fetch the
//! authoritative value, register `web::fetch`, bind the config-change
//! trigger for hot-reload, then sleep until Ctrl+C.

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};

use web::config::WebConfig;
use web::{configuration, functions, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "web",
    about = "Outbound HTTP client on the iii bus (web::fetch)."
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
                name: "web".to_string(),
                os: std::env::consts::OS.to_string(),
                description: Some("Outbound HTTP client on the iii bus (web::fetch).".to_string()),
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
            Ok(v) => match WebConfig::from_json(&v) {
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

    // Best-effort on purpose: every WebConfig field has a safe default and
    // hot-reloads, so unlike the full config integrations
    // (docs/sops/configuration.md §4c) a configuration-worker failure must
    // not take web::fetch off the bus — warn, run on defaults, and recover on
    // the next configuration event or restart.
    if let Err(e) = configuration::register_config(&iii, seed.as_ref()).await {
        tracing::warn!(error = %e, "registering web configuration schema failed; continuing");
    }
    let cfg = match configuration::fetch_config(&iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!(error = %e, "loading web configuration failed; using defaults");
            WebConfig::default()
        }
    };
    tracing::info!(
        max_timeout_ms = cfg.max_timeout_ms,
        max_response_bytes = cfg.max_response_bytes,
        allow_loopback = cfg.allow_loopback,
        "loaded web configuration"
    );

    let inject_guidance = cfg.inject_guidance;
    let shared = cfg.into_shared();
    functions::register_all(&iii, &shared);

    // The web worker owns its rich chat presentation (notably viewable image
    // responses) through the console's injectable UI contract.
    web::ui::register(&iii);

    // Reconcile the pre-generate guidance binding (web::inject-guidance) with
    // the configured `inject_guidance` (on by default); config changes
    // re-reconcile it live via the trigger below.
    let state = configuration::SharedState {
        config: shared.clone(),
        guidance: configuration::GuidanceState::default(),
    };
    configuration::apply_guidance(&iii, &state.guidance, inject_guidance);

    match configuration::register_config_trigger(&iii, state) {
        // One serialized re-fetch to close the boot gap: an update landing
        // between the fetch above and the binding just registered fired into
        // nothing, and would otherwise stay invisible until the NEXT change.
        Ok(reload) => reload.run().await,
        Err(e) => {
            tracing::warn!(error = %e, "registering configuration change trigger failed; config frozen until restart");
        }
    }

    tracing::info!(
        inject_guidance,
        "web ready: web::fetch + injectable console UI + configuration hot-reload"
    );
    tokio::signal::ctrl_c().await?;
    tracing::info!("web shutting down");
    iii.shutdown_async().await;
    Ok(())
}
