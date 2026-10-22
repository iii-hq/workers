use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_helpers::observability::OtelConfig;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use sandbox_code_runner::engine::{probe_sandbox, IIIEngine, SandboxProbe};
use sandbox_code_runner::manager::RuntimeManager;
use sandbox_code_runner::{config, configuration, events, functions, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "sandbox-code-runner",
    version,
    about = "Run Node.js and Python in iii-sandbox microVMs: run, register bus functions, teardown"
)]
struct Cli {
    /// Operator config file.
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    /// WebSocket URL of the iii engine.
    #[arg(long, env = "III_URL", default_value = "ws://127.0.0.1:49134")]
    url: String,

    /// Print the registry manifest as JSON and exit without connecting.
    #[arg(long)]
    manifest: bool,
}

fn worker_metadata() -> WorkerMetadata {
    WorkerMetadata {
        runtime: "rust".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: "sandbox-code-runner".to_string(),
        os: std::env::consts::OS.to_string(),
        pid: Some(std::process::id()),
        telemetry: None,
        ..WorkerMetadata::default()
    }
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

    let cfg = match config::load_config(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::CodeRunnerConfig::default()
        }
    };
    let cfg = Arc::new(cfg);

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            metadata: Some(worker_metadata()),
            ..InitOptions::default()
        },
    ));

    let engine = Arc::new(IIIEngine::new(iii.clone()));
    let manager = RuntimeManager::new(cfg.clone(), engine.clone(), &cli.url);
    // Register the `sandbox-code-runner::event` trigger type first so the
    // emitter is wired before any function can run.
    let emitter = events::register_event_trigger(&iii);
    manager.set_events(emitter.clone());
    sandbox_code_runner::fleet_watch::spawn(engine.clone(), emitter);
    functions::register_all(&iii, &manager);
    // Injected console UI: the function-trigger cards for the ops above.
    sandbox_code_runner::ui::register(&iii);

    // The builtin `configuration` worker owns this worker's console-facing
    // knobs; `inject_guidance` defaults on and hot-applies on change by
    // binding/unbinding the pre-generate guidance hook.
    //
    // Best-effort on purpose: the entry carries one cosmetic prompt knob, so
    // unlike the full config integrations (docs/sops/configuration.md §4c) a
    // configuration-worker failure must not take the run/register surface off
    // the bus — warn, run on defaults, and recover on the next configuration
    // event or restart.
    if let Err(e) = configuration::register_config(&iii).await {
        tracing::warn!(error = %e, "registering sandbox-code-runner configuration schema failed; continuing");
    }
    let shared_cfg = match configuration::fetch_config(&iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!(error = %e, "loading sandbox-code-runner configuration failed; using defaults");
            configuration::RunnerSharedConfig::default()
        }
    };
    let guidance = configuration::GuidanceState::default();
    configuration::apply_guidance(&iii, &guidance, shared_cfg.inject_guidance);
    match configuration::register_config_trigger(&iii, guidance) {
        // One serialized re-fetch to close the boot gap: an update landing
        // between the fetch above and the binding just registered fired into
        // nothing, and would otherwise stay invisible until the NEXT change.
        Ok(reload) => reload.run().await,
        Err(e) => {
            tracing::warn!(error = %e, "registering configuration change trigger failed; the inject_guidance knob is frozen until restart");
        }
    }

    // Startup probe: is a sandbox adapter connected? Fail OPEN — the
    // operator may add it later, and every call meanwhile errors with the
    // router's own message — but say it loudly once, at boot, instead of
    // letting the first caller discover it cryptically.
    match probe_sandbox(engine.as_ref()).await {
        SandboxProbe::Ready => tracing::info!("sandbox adapter detected"),
        SandboxProbe::Missing => tracing::warn!(
            "iii-sandbox is NOT installed on this engine — every sandbox-code-runner \
             call will fail until an operator runs `iii trigger compose::add worker=iii-sandbox`"
        ),
        SandboxProbe::Unverified(raw) => {
            tracing::warn!(error = %raw, "could not verify iii-sandbox presence");
        }
    }

    tracing::info!(
        idle_ttl_secs = cfg.idle_ttl_secs,
        default_timeout_ms = cfg.default_timeout_ms,
        "sandbox-code-runner ready"
    );
    tokio::signal::ctrl_c().await?;
    tracing::info!("sandbox-code-runner shutting down");
    iii.shutdown_async().await;
    Ok(())
}
