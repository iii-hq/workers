use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::Mutex;

use crate::adapter::{ContainerSpec, RuntimeAdapter};
use crate::state::LauncherState;

const HEALTH_CHECK_INTERVAL_SECS: u64 = 15;
const MAX_RESTART_COUNT: u32 = 5;
const MAX_BACKOFF_SECS: i64 = 60;
const STOP_TIMEOUT_SECS: u32 = 30;

pub async fn run_health_loop(
    adapter: Arc<dyn RuntimeAdapter>,
    state: Arc<Mutex<LauncherState>>,
    iii: iii_sdk::III,
) {
    tracing::info!("health check loop started (interval={}s)", HEALTH_CHECK_INTERVAL_SECS);

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(HEALTH_CHECK_INTERVAL_SECS)).await;

        if let Err(e) = run_health_check(&adapter, &state, &iii).await {
            tracing::error!(error = %e, "health check iteration failed");
        }
    }
}

async fn run_health_check(
    adapter: &Arc<dyn RuntimeAdapter>,
    state: &Arc<Mutex<LauncherState>>,
    iii: &iii_sdk::III,
) -> anyhow::Result<()> {
    // Fetch the list of engine-connected workers once per cycle
    let engine_workers = match iii.list_workers().await {
        Ok(workers) => workers,
        Err(e) => {
            tracing::warn!(error = %e, "failed to query engine workers, skipping engine check this cycle");
            Vec::new()
        }
    };

    // Snapshot worker names + container IDs to avoid holding the lock during async I/O
    let worker_snapshots: Vec<(String, String)> = {
        let st = state.lock().await;
        st.managed_workers
            .iter()
            .filter(|(_, w)| w.status != "failed")
            .map(|(name, w)| (name.clone(), w.container_id.clone()))
            .collect()
    };

    for (name, container_id) in worker_snapshots {
        check_worker(adapter, state, &engine_workers, &name, &container_id).await;
    }

    Ok(())
}

async fn check_worker(
    adapter: &Arc<dyn RuntimeAdapter>,
    state: &Arc<Mutex<LauncherState>>,
    engine_workers: &[iii_sdk::WorkerInfo],
    name: &str,
    container_id: &str,
) {
    // 1. Check container running status
    let container_healthy = match adapter.status(container_id).await {
        Ok(status) => status.running,
        Err(e) => {
            tracing::warn!(worker = %name, error = %e, "failed to query container status");
            false
        }
    };

    // 2. Check engine registration (worker name matches)
    let engine_healthy = engine_workers.iter().any(|w| {
        w.name.as_deref() == Some(name)
    });

    if container_healthy && engine_healthy {
        // Both checks pass: mark healthy, reset backoff state
        let mut st = state.lock().await;
        if let Some(worker) = st.managed_workers.get_mut(name) {
            if worker.restart_count > 0 {
                tracing::info!(worker = %name, "worker recovered, resetting restart count");
            }
            worker.restart_count = 0;
            worker.last_failure = None;
            worker.backoff_until = None;
        }
    } else {
        // Unhealthy: enter restart flow
        tracing::warn!(
            worker = %name,
            container_healthy = container_healthy,
            engine_healthy = engine_healthy,
            "worker unhealthy, entering restart flow"
        );
        handle_unhealthy_worker(adapter, state, name).await;
    }
}

async fn handle_unhealthy_worker(
    adapter: &Arc<dyn RuntimeAdapter>,
    state: &Arc<Mutex<LauncherState>>,
    name: &str,
) {
    // Phase 1: Under the lock, check backoff cooldown, then update restart tracking and collect
    // the data needed for the restart. Release the lock before doing async container I/O.
    let restart_info = {
        let mut st = state.lock().await;

        let worker = match st.managed_workers.get(name) {
            Some(w) => w,
            None => return,
        };

        // Check if we are still in backoff cooldown from a previous restart attempt
        if let Some(backoff_until) = worker.backoff_until {
            if Utc::now() < backoff_until {
                let remaining = backoff_until.signed_duration_since(Utc::now());
                tracing::info!(
                    worker = %name,
                    backoff_remaining_secs = remaining.num_seconds(),
                    "worker in backoff cooldown, skipping restart"
                );
                return;
            }
        }

        // Increment restart count and record failure
        let worker = st.managed_workers.get_mut(name).unwrap();
        worker.restart_count += 1;
        worker.last_failure = Some(Utc::now().to_rfc3339());

        // Too many restarts: mark as permanently failed
        if worker.restart_count > MAX_RESTART_COUNT {
            tracing::error!(
                worker = %name,
                restart_count = worker.restart_count,
                "worker exceeded max restart attempts, marking as failed"
            );
            worker.status = "failed".to_string();
            let _ = st.save();
            return;
        }

        // Compute exponential backoff: min(5 * 2^(restart_count - 1), 60) seconds
        let backoff_secs = std::cmp::min(
            5i64 * 2i64.pow(worker.restart_count - 1),
            MAX_BACKOFF_SECS,
        );
        let backoff_until = Utc::now() + chrono::Duration::seconds(backoff_secs);
        worker.backoff_until = Some(backoff_until);

        // Collect restart data before dropping the mutable borrow
        let info = RestartInfo {
            container_id: worker.container_id.clone(),
            image: worker.image.clone(),
            engine_url: worker.engine_url.clone(),
            auth_token: worker.auth_token.clone(),
            config: worker.config.clone(),
            memory_limit: worker.memory_limit.clone(),
            cpu_limit: worker.cpu_limit.clone(),
            restart_count: worker.restart_count,
        };

        let _ = st.save();
        info
    };

    let info = restart_info;

    tracing::info!(
        worker = %name,
        restart_count = info.restart_count,
        "restarting unhealthy worker"
    );

    // Phase 2: Stop and remove the old container (best-effort, no lock held)
    if let Err(e) = adapter.stop(&info.container_id, STOP_TIMEOUT_SECS).await {
        tracing::warn!(worker = %name, error = %e, "failed to stop old container during restart");
    }
    if let Err(e) = adapter.remove(&info.container_id).await {
        tracing::warn!(worker = %name, error = %e, "failed to remove old container during restart");
    }

    // Phase 3: Reconstruct ContainerSpec and start new container
    let engine_url = info.engine_url.unwrap_or_default();
    let auth_token = info.auth_token.unwrap_or_default();

    let config_json = serde_json::to_string(&info.config).unwrap_or_else(|_| "{}".to_string());
    let config_b64 = data_encoding::BASE64.encode(config_json.as_bytes());

    let mut env = HashMap::new();
    env.insert("III_ENGINE_URL".to_string(), engine_url);
    env.insert("III_AUTH_TOKEN".to_string(), auth_token);
    env.insert("III_WORKER_CONFIG".to_string(), config_b64);

    let spec = ContainerSpec {
        name: name.to_string(),
        image: info.image,
        env,
        memory_limit: info.memory_limit,
        cpu_limit: info.cpu_limit,
    };

    match adapter.start(&spec).await {
        Ok(new_container_id) => {
            tracing::info!(
                worker = %name,
                new_container_id = %new_container_id,
                "worker restarted successfully"
            );
            let mut st = state.lock().await;
            if let Some(worker) = st.managed_workers.get_mut(name) {
                worker.container_id = new_container_id;
                worker.status = "running".to_string();
                worker.started_at = Utc::now();
            }
            let _ = st.save();
        }
        Err(e) => {
            tracing::error!(
                worker = %name,
                error = %e,
                "failed to restart worker container"
            );
            let mut st = state.lock().await;
            if let Some(worker) = st.managed_workers.get_mut(name) {
                worker.status = "restart_failed".to_string();
            }
            let _ = st.save();
        }
    }
}

struct RestartInfo {
    container_id: String,
    image: String,
    engine_url: Option<String>,
    auth_token: Option<String>,
    config: serde_json::Value,
    memory_limit: Option<String>,
    cpu_limit: Option<String>,
    restart_count: u32,
}
