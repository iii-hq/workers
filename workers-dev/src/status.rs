use std::process::Stdio;

use anyhow::{Context, Result};
use serde::Deserialize;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::discover::WorkerGroup;
use crate::config::Config;

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
    pub engine_pid: Option<u32>,
    pub uptime: String,
    pub last_logs: Vec<String>,
}

pub async fn fetch_engine_workers(config: &Config) -> Result<Vec<EngineWorker>> {
    let mut cmd = Command::new("iii");
    cmd.args([
        "trigger",
        "engine::workers::list",
        "--json",
        "{}",
        "--address",
        &config.engine_host,
        "--port",
        &config.engine_port.to_string(),
    ]);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .context("spawn iii trigger (is iii on PATH?)")?;
    let mut stdout = child.stdout.take().context("iii stdout")?;
    let mut stderr = child.stderr.take().context("iii stderr")?;

    let status = child.wait().await.context("wait for iii trigger")?;

    let mut out = String::new();
    stdout.read_to_string(&mut out).await?;
    let mut err = String::new();
    stderr.read_to_string(&mut err).await?;

    if !status.success() {
        anyhow::bail!(
            "iii trigger engine::workers::list failed: {}",
            err.trim()
        );
    }

    let parsed: EngineWorkersResponse = serde_json::from_str(out.trim())
        .with_context(|| format!("parse engine workers response: {}", out.trim()))?;
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
            .engine_pid
            .or(v.local_pid)
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
