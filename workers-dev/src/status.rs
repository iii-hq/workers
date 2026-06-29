use anyhow::{anyhow, Context, Result};
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::Deserialize;

use crate::discover::WorkerGroup;

#[derive(Debug, Clone, Deserialize)]
pub struct EngineWorkersResponse {
    pub workers: Vec<EngineWorker>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EngineWorker {
    #[serde(default)]
    pub name: Option<String>,
    pub status: String,
    #[serde(default)]
    pub connected_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct WorkerView {
    pub name: String,
    pub group: WorkerGroup,
    pub spawnable: bool,
    pub display_status: String,
    pub process_status: String,
    pub engine_status: String,
    pub local_pid: Option<u32>,
    pub uptime: String,
    /// Exit code of the last run when the process crashed. Surfaced inline so a
    /// failure is visible in the table without opening the log pane.
    pub exit_code: Option<i32>,
}

/// Query the engine for its connected workers over the shared persistent
/// connection. Reuses one WebSocket instead of spawning `iii trigger` per
/// poll, so the engine no longer logs a register/unregister pair every tick.
pub async fn fetch_engine_workers(
    client: &IIIClient,
    timeout_ms: u64,
) -> Result<Vec<EngineWorker>> {
    let result = client
        .trigger(TriggerRequest {
            function_id: "engine::workers::list".to_string(),
            payload: serde_json::json!({}),
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await
        .map_err(|e| anyhow!("engine::workers::list failed (is the engine running?): {e}"))?;

    let parsed: EngineWorkersResponse =
        serde_json::from_value(result).context("parse engine workers response")?;
    Ok(parsed.workers)
}

pub fn print_status_table(views: &[WorkerView]) {
    if views.is_empty() {
        return;
    }
    let mut last_group: Option<WorkerGroup> = None;
    for v in views {
        if last_group != Some(v.group) {
            if last_group.is_some() {
                println!();
            }
            println!("── {} ──", v.group.label());
            println!(
                "{:<28} {:<12} {:<12} {:<8} {:<8}",
                "WORKER", "PROCESS", "ENGINE", "PID", "UPTIME"
            );
            last_group = Some(v.group);
        }
        let pid = v
            .local_pid
            .map(|p| p.to_string())
            .unwrap_or_else(|| "—".to_string());
        let name = if v.spawnable {
            v.name.clone()
        } else {
            format!("{} (iii worker add)", v.name)
        };
        println!(
            "{:<28} {:<12} {:<12} {:<8} {:<8}",
            name, v.process_status, v.engine_status, pid, v.uptime
        );
    }
}
