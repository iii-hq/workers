use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use code_runner::manager::Manager;
use code_runner::node_bus::IIIEngine;
use code_runner::{config, functions, manifest};
use iii_helpers::observability::OtelConfig;
use iii_node_core::manager::RuntimeManager;
use iii_node_core::runtime;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};

#[derive(Parser, Debug)]
#[command(
    name = "code-runner",
    version,
    about = "Run untrusted Node.js and Python in-process, no microVM required"
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
        name: "code-runner".to_string(),
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

    // Once per process, before any isolate exists.
    runtime::init_v8_platform();

    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            metadata: Some(worker_metadata()),
            ..InitOptions::default()
        },
    ));

    // One `Engine` shared by both halves: node-core registers through it from
    // its ops, and the router registers python handlers through it directly.
    let engine = Arc::new(IIIEngine::new(iii.clone()));
    // `seeded_ids`, not `STATIC_IDS`: the console UI's content function is
    // registered after this, and an unseeded worker id is claimable through
    // `register_function` — which ends in the SDK's duplicate-id abort.
    let node = RuntimeManager::new(
        Arc::new(cfg.node()),
        engine.clone(),
        &functions::seeded_ids(),
    );
    // Booting the python engine extracts and compiles the embedded CPython
    // artifact, which can fail on a host with no writable cache. Degrade to
    // node-only and say so loudly rather than refusing to start: a node-only
    // deployment losing its whole worker to a python problem is the wrong
    // trade, and every python call reports the reason.
    let python = match iii_python_core::runner::Runner::boot() {
        Ok(runner) => Some(iii_python_core::manager::Manager::with_bridge(
            Arc::new(cfg.python()),
            runner,
            Arc::new(code_runner::python_bus::IIIBridge::new(iii.clone())),
        )),
        Err(e) => {
            tracing::error!(
                error = %e,
                "python engine failed to start; code-runner will serve node only and refuse \
                 lang=\"python\""
            );
            None
        }
    };

    let manager = Manager::new(cfg.clone(), node, engine, python);
    functions::register_all(&iii, &manager);
    // After the functions: the UI's renderers key off function ids, so there
    // is nothing for a console to render until those exist.
    code_runner::ui::register(&iii);
    functions::setup_harness_hooks(&iii);

    // Backstop for callers that never call teardown.
    let sweeper = manager.clone();
    let ttl = manager.idle_ttl_secs();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(ttl.clamp(1, 60)));
        loop {
            ticker.tick().await;
            // An unguarded panic here would kill the task silently — `main` is
            // parked on ctrl_c and would never notice — and idle runtimes would
            // accumulate until `max_runtimes` is exhausted.
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sweeper.sweep_idle())) {
                Ok(reaped) if !reaped.is_empty() => {
                    tracing::info!(count = reaped.len(), "reaped idle runtimes");
                }
                Ok(_) => {}
                Err(_) => tracing::error!(
                    "idle-runtime sweep panicked; runtimes whose callers never call \
                     code-runner::teardown will accumulate until max_runtimes is exhausted"
                ),
            }
        }
    });

    tracing::info!(
        max_runtimes = cfg.max_runtimes,
        heap_mb = cfg.heap_mb,
        scratch_mb = cfg.scratch_mb,
        "code-runner ready"
    );
    tokio::signal::ctrl_c().await?;
    tracing::info!("code-runner shutting down");
    iii.shutdown_async().await;
    Ok(())
}
