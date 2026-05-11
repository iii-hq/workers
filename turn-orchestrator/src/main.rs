use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, TriggerRequest, WorkerMetadata};
use serde_json::json;

use turn_orchestrator::{config, manifest};

#[derive(Parser, Debug)]
#[command(
    name = "turn-orchestrator",
    about = "Durable run::start state machine driving each agent turn through provisioning, assistant, tools, steering, and tearing-down."
)]
struct Cli {
    #[arg(
        long,
        env = "TURN_ORCHESTRATOR_CONFIG",
        default_value = "./config.yaml"
    )]
    config: String,

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
        let m = manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m).unwrap());
        return Ok(());
    }

    let cfg = match config::load_config(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::TurnOrchestratorConfig::default()
        }
    };
    let cfg = Arc::new(cfg);

    let iii = register_worker(
        &cli.url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "turn-orchestrator".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    turn_orchestrator::register_with_iii(&iii, &cfg)
        .await
        .expect("turn-orchestrator register failed");
    tracing::info!("turn-orchestrator registered (run::start, run::start_and_wait, turn::step)");

    spawn_skill_register(iii.clone());

    tokio::signal::ctrl_c().await?;
    tracing::info!("turn-orchestrator shutting down");
    iii.shutdown_async().await;
    Ok(())
}

fn spawn_skill_register(iii: Arc<iii_sdk::III>) {
    tokio::spawn(async move {
        register_skill_with_retry(&iii, "turn-orchestrator", include_str!("../skill.md")).await;
    });
}

async fn register_skill_with_retry(iii: &iii_sdk::III, id: &str, body: &str) {
    let mut backoff = Duration::from_secs(5);
    let started = Instant::now();
    loop {
        let res = iii
            .trigger(TriggerRequest {
                function_id: "skills::register".into(),
                payload: json!({ "id": id, "skill": body }),
                action: None,
                timeout_ms: Some(5_000),
            })
            .await;
        match res {
            Ok(_) => {
                tracing::info!(%id, "registered skill");
                return;
            }
            Err(e) => {
                if started.elapsed() > Duration::from_mins(3) {
                    tracing::warn!(
                        %id,
                        error = %e,
                        "skills handshake gave up; install/start the skills worker and restart"
                    );
                    return;
                }
                tracing::debug!(%id, error = %e, ?backoff, "skills::register failed; retrying");
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_mins(1));
    }
}
