use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_helpers::observability::OtelConfig;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{InitOptions, register_worker};
use sandbox_code_runner::engine::{IIIEngine, SandboxProbe, probe_sandbox};
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
    // Register `sandbox-code-runner::on-event` first so the emitter is wired
    // before any function can run. Its observers start and stop the fleet
    // watcher.
    let emitter = events::register_event_function(&iii, engine.clone());
    manager.set_events(emitter);
    functions::register_all(&iii, &manager);
    // Injected console UI: the function-call cards for the ops above.
    sandbox_code_runner::ui::register(&iii);

    // The builtin `configuration` worker owns this worker's console-facing
    // knobs; `inject_guidance` defaults on and hot-applies on change by
    // observing the pre-generate hook point or disposing that observer.
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
    // One serialized re-fetch to close the boot gap: an update landing
    // between the fetch above and the observer becoming active reaches no
    // one, and would otherwise stay invisible until the NEXT change.
    configuration::observe_config_changes(&iii, guidance)
        .run()
        .await;

    // Startup probe: is a sandbox adapter connected? Fail OPEN — the
    // operator may add it later, and every call meanwhile errors with the
    // router's own message — but say it loudly once, at boot, instead of
    // letting the first caller discover it cryptically.
    match probe_sandbox(engine.as_ref()).await {
        SandboxProbe::Ready => tracing::info!("sandbox adapter detected"),
        SandboxProbe::Missing => tracing::warn!(
            "no sandbox adapter answered this engine's router (not installed, not running, or \
             hung): sandbox-code-runner calls can fail until one does; to install one, run \
             `iii call compose::add worker=iii-sandbox`"
        ),
        SandboxProbe::Incompatible(reason) => tracing::warn!(
            "{reason}: every sandbox-code-runner call will fail with S302 until a \
             compatible sandbox adapter release is installed"
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
