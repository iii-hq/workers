use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use iii_sdk::{register_worker, InitOptions, OtelConfig, TriggerRequest, WorkerMetadata};

mod manifest;

#[derive(Parser, Debug)]
#[command(
    name = "models-catalog",
    about = "Model capabilities knowledge base (models::*) on the iii bus."
)]
struct Cli {
    #[arg(
        long,
        env = "III_MODELS_CATALOG_CONFIG",
        default_value = "./config.yaml"
    )]
    config: String,

    /// Engine WebSocket URL. Falls back to `engine_url` in `--config` when unset.
    #[arg(long, env = "III_URL")]
    url: Option<String>,

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
        println!("{}", serde_json::to_string_pretty(&m)?);
        return Ok(());
    }

    let cfg = match models_catalog::config::load_config(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            models_catalog::config::ModelsCatalogConfig::default()
        }
    };
    let cfg = Arc::new(cfg);

    let url = cli.url.clone().unwrap_or_else(|| cfg.engine_url.clone());

    let iii = register_worker(
        &url,
        InitOptions {
            otel: Some(OtelConfig::default()),
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "models-catalog".to_string(),
                os: std::env::consts::OS.to_string(),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let iii = Arc::new(iii);

    let _refs = models_catalog::register_with_iii(&iii, &cfg)
        .await
        .context("models-catalog register failed")?;
    tracing::info!("models-catalog registered (models::*)");

    spawn_skill_register(iii.clone(), Arc::clone(&cfg));

    wait_for_shutdown().await?;

    unregister_skill(&iii, &cfg).await;
    tracing::info!("models-catalog shutting down");
    iii.shutdown_async().await;
    Ok(())
}

async fn register_skill_with_retry(
    iii: &iii_sdk::III,
    id: &str,
    body: &str,
    trigger_timeout_ms: u64,
) {
    let mut backoff = Duration::from_secs(5);
    let started = Instant::now();
    loop {
        let res = iii
            .trigger(TriggerRequest {
                function_id: "skills::register".into(),
                payload: serde_json::json!({ "id": id, "skill": body }),
                action: None,
                timeout_ms: Some(trigger_timeout_ms),
            })
            .await;
        match res {
            Ok(_) => {
                tracing::info!(id, "registered skill");
                return;
            }
            Err(e) => {
                if started.elapsed() > Duration::from_mins(3) {
                    tracing::warn!(
                        id,
                        error = %e,
                        "skills handshake gave up; install/start the skills worker and restart"
                    );
                    return;
                }
                tracing::debug!(id, error = %e, ?backoff, "skills::register failed; retrying");
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_mins(1));
    }
}

fn spawn_skill_register(
    iii: Arc<iii_sdk::III>,
    cfg: Arc<models_catalog::config::ModelsCatalogConfig>,
) {
    let timeout = cfg.skills_register_timeout_ms;
    tokio::spawn(async move {
        register_skill_with_retry(
            &iii,
            models_catalog::SKILL_ID,
            models_catalog::SKILL_MD,
            timeout,
        )
        .await;
        for (id, body) in models_catalog::SUB_SKILLS {
            register_skill_with_retry(&iii, id, body, timeout).await;
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
async fn unregister_skill(
    iii: &Arc<iii_sdk::III>,
    cfg: &Arc<models_catalog::config::ModelsCatalogConfig>,
) {
    let t = cfg.skills_unregister_timeout_ms;
    for (id, _) in models_catalog::SUB_SKILLS {
        let _ = iii
            .trigger(TriggerRequest {
                function_id: "skills::unregister".into(),
                payload: serde_json::json!({ "id": id }),
                action: None,
                timeout_ms: Some(t),
            })
            .await;
    }
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "skills::unregister".into(),
            payload: serde_json::json!({ "id": models_catalog::SKILL_ID }),
            action: None,
            timeout_ms: Some(t),
        })
        .await;
}
