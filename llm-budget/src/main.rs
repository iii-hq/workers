use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, TriggerRequest, WorkerMetadata};
use serde_json::json;

#[derive(Parser, Debug)]
#[command(
    name = "llm-budget",
    about = "LLM spend budgets on the iii bus (budget::* + skills)."
)]
struct Cli {
    #[arg(long, default_value = "./config.yaml")]
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
        let m = llm_budget::manifest::build_manifest();
        println!("{}", serde_json::to_string_pretty(&m)?);
        return Ok(());
    }

    let cfg = match llm_budget::config::load_config(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            llm_budget::config::WorkerConfig::default()
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
                name: "llm-budget".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    let _refs = llm_budget::register_with_iii(&iii)
        .await
        .context("llm-budget register failed")?;
    tracing::info!("llm-budget registered (budget::*)");

    spawn_skill_register(iii.clone(), cfg.clone());

    wait_for_shutdown().await?;

    unregister_skill(&iii).await;

    iii.shutdown_async().await;

    Ok(())
}

async fn register_skill_with_retry(
    iii: &iii_sdk::III,
    id: &str,
    body: &str,
    trigger_timeout_ms: u64,
    handshake_deadline: Duration,
) {
    let mut backoff = Duration::from_secs(5);
    let started = Instant::now();
    loop {
        let res = iii
            .trigger(TriggerRequest {
                function_id: "skills::register".into(),
                payload: json!({ "id": id, "skill": body }),
                action: None,
                timeout_ms: Some(trigger_timeout_ms),
            })
            .await;
        match res {
            Ok(_) => {
                tracing::info!("registered skill: {id}");
                return;
            }
            Err(e) => {
                if started.elapsed() > handshake_deadline {
                    tracing::warn!(
                        "skills handshake gave up for {id}; install/start the skills worker and restart (last error: {e})"
                    );
                    return;
                }
                tracing::debug!("skills::register failed for {id}: {e}; retrying in {backoff:?}");
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_mins(1));
    }
}

fn spawn_skill_register(iii: Arc<iii_sdk::III>, cfg: Arc<llm_budget::config::WorkerConfig>) {
    let trigger_ms = cfg.skills_trigger_timeout_ms;
    let deadline = Duration::from_secs(cfg.skills_handshake_deadline_secs);

    tokio::spawn(async move {
        register_skill_with_retry(
            &iii,
            llm_budget::SKILL_ID,
            llm_budget::SKILL_MD,
            trigger_ms,
            deadline,
        )
        .await;
        for (id, body) in llm_budget::SUB_SKILLS {
            register_skill_with_retry(&iii, id, body, trigger_ms, deadline).await;
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

// Best-effort: a missed unregister is self-healing on next boot's re-register.
// Leaves go first so the router is the last entry to disappear from iii://skills.
async fn unregister_skill(iii: &Arc<iii_sdk::III>) {
    for (id, _) in llm_budget::SUB_SKILLS {
        let _ = iii
            .trigger(TriggerRequest {
                function_id: "skills::unregister".into(),
                payload: json!({ "id": id }),
                action: None,
                timeout_ms: Some(2000),
            })
            .await;
    }
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "skills::unregister".into(),
            payload: json!({ "id": llm_budget::SKILL_ID }),
            action: None,
            timeout_ms: Some(2000),
        })
        .await;
}
