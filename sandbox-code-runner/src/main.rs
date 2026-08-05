use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_helpers::observability::OtelConfig;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions};
use sandbox_code_runner::engine::{Engine as _, IIIEngine};
use sandbox_code_runner::error::{classify_probe_error, ProbeOutcome};
use sandbox_code_runner::manager::RuntimeManager;
use sandbox_code_runner::{config, functions, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "sandbox-code-runner",
    version,
    about = "Run Node.js and Python in iii-sandbox microVMs: eval, register bus functions, teardown"
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
    functions::register_all(&iii, &manager);
    functions::setup_harness_hooks(&iii);
    // Injected console UI: the function-trigger cards for the ops above.
    sandbox_code_runner::ui::register(&iii);

    // Startup probe: is the iii-sandbox daemon serving? Fail OPEN — the
    // operator may add it later, and every call meanwhile errors with the
    // daemon's own message — but say it loudly once, at boot, instead of
    // letting the first caller discover it cryptically.
    {
        let probe = engine
            .call(
                "engine::functions::info".to_string(),
                serde_json::json!({ "function_id": "sandbox::create" }),
                5_000,
            )
            .await;
        match probe {
            Ok(_) => tracing::info!("iii-sandbox detected: sandbox::create is registered"),
            Err(raw) => {
                // Same disambiguation as the register-time probe (see
                // `classify_probe_error`'s doc): a "not found" naming
                // `engine::functions::info` itself means THIS engine can't
                // dispatch the probe, not that `sandbox::create` is absent.
                // Only mis-words a log line here (this path always fails
                // open — code-runner keeps serving either way), but should
                // still say the honest thing.
                if classify_probe_error(&raw, "sandbox::create") == ProbeOutcome::Free {
                    tracing::warn!(
                        "iii-sandbox is NOT installed on this engine — every code-runner call \
                         will fail until an operator runs `iii worker add iii-sandbox`"
                    );
                } else {
                    tracing::warn!(error = %raw, "could not verify iii-sandbox presence");
                }
            }
        }
    }

    tracing::info!(
        idle_ttl_secs = cfg.idle_ttl_secs,
        default_timeout_ms = cfg.default_timeout_ms,
        "code-runner ready"
    );
    tokio::signal::ctrl_c().await?;
    tracing::info!("code-runner shutting down");
    iii.shutdown_async().await;
    Ok(())
}
