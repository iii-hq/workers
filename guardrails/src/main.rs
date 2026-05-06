use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let engine_url = std::env::var("III_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.to_string());
    let iii = Arc::new(register_worker(&engine_url, InitOptions::default()));

    let _refs = guardrails::register_with_iii(&iii)
        .await
        .context("guardrails register failed")?;
    log::info!("guardrails registered (guardrails::*)");

    spawn_skill_register(iii.clone());

    wait_for_shutdown().await?;

    unregister_skill(&iii).await;
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
                log::info!("registered skill: {id}");
                return;
            }
            Err(e) => {
                if started.elapsed() > Duration::from_mins(3) {
                    log::warn!(
                        "skills handshake gave up for {id}; install/start the skills worker and restart (last error: {e})"
                    );
                    return;
                }
                log::debug!("skills::register failed for {id}: {e}; retrying in {backoff:?}");
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_mins(1));
    }
}

fn spawn_skill_register(iii: Arc<iii_sdk::III>) {
    tokio::spawn(async move {
        register_skill_with_retry(&iii, guardrails::SKILL_ID, guardrails::SKILL_MD).await;
        for (id, body) in guardrails::SUB_SKILLS {
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

// Best-effort: a missed unregister is self-healing on next boot's re-register.
// Leaves go first so the router is the last entry to disappear from iii://skills.
async fn unregister_skill(iii: &Arc<iii_sdk::III>) {
    for (id, _) in guardrails::SUB_SKILLS {
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
            payload: json!({ "id": guardrails::SKILL_ID }),
            action: None,
            timeout_ms: Some(2_000),
        })
        .await;
}
