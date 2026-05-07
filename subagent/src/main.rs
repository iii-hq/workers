use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, TriggerRequest, WorkerMetadata};
use serde_json::json;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "subagent",
    about = "Spawn child agent sessions via subagent::start / run::start_and_wait."
)]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, default_value = "ws://127.0.0.1:49134")]
    url: String,

    #[arg(long)]
    manifest: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    if cli.manifest {
        let m = subagent::manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m)?);
        return Ok(());
    }

    let cfg = match subagent::config::load_config(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            subagent::config::SubagentConfig::default()
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
                name: "subagent".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    subagent::register_with_iii(&iii, &cfg);
    tracing::info!("subagent ready (subagent::start registered)");

    spawn_skill_register(iii.clone());

    wait_for_shutdown().await?;

    unregister_skill(&iii).await;
    tracing::info!("subagent shutting down");
    iii.shutdown_async().await;
    Ok(())
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
                tracing::info!(skill_id = %id, "registered skill");
                return;
            }
            Err(e) => {
                if started.elapsed() > Duration::from_secs(3 * 60) {
                    tracing::warn!(
                        skill_id = %id,
                        error = %e,
                        "skills handshake gave up; install/start the skills worker and restart"
                    );
                    return;
                }
                tracing::debug!(skill_id = %id, error = %e, wait = ?backoff, "skills::register failed; retrying");
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

fn spawn_skill_register(iii: Arc<iii_sdk::III>) {
    tokio::spawn(async move {
        register_skill_with_retry(&iii, subagent::SKILL_ID, subagent::SKILL_MD).await;
        for (id, body) in subagent::SUB_SKILLS {
            register_skill_with_retry(&iii, id, body).await;
        }
    });
}

async fn wait_for_shutdown() -> Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).context("failed to install SIGTERM handler")?;
        tokio::select! {
            r = tokio::signal::ctrl_c() => r.context("failed to await SIGINT")?,
            _ = sigterm.recv() => {}
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .context("failed to await SIGINT")
    }
}

async fn unregister_skill(iii: &Arc<iii_sdk::III>) {
    for (id, _) in subagent::SUB_SKILLS {
        let _ = iii
            .trigger(TriggerRequest {
                function_id: "skills::unregister".into(),
                payload: json!({ "id": id }),
                action: None,
                timeout_ms: Some(2_000),
            })
            .await;
    }
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "skills::unregister".into(),
            payload: json!({ "id": subagent::SKILL_ID }),
            action: None,
            timeout_ms: Some(2_000),
        })
        .await;
}
