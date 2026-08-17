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
    /// Injectable-UI dev watcher: `None` = the worker ships no ui/ project,
    /// `Some(false)` = has one (watch off), `Some(true)` = watcher mode on.
    pub ui_watch: Option<bool>,
}

use std::collections::HashSet;

/// Group header text for one stack: the Stack group is named after the
/// current stack, everything else is "other".
pub fn group_label(group: WorkerGroup, stack_name: &str) -> String {
    match group {
        WorkerGroup::Stack => format!("stack:{stack_name}"),
        WorkerGroup::Other => "other".to_string(),
    }
}

/// Assign groups + display order for one stack's member set: members first,
/// then other, alphabetical within each. Runs at view time so switching
/// stacks in the TUI regroups without touching discovery data.
pub fn assign_view_groups(views: &mut [WorkerView], members: &HashSet<String>) {
    for v in views.iter_mut() {
        v.group = if members.contains(&v.name) {
            WorkerGroup::Stack
        } else {
            WorkerGroup::Other
        };
    }
    views.sort_by(|a, b| a.group.cmp(&b.group).then_with(|| a.name.cmp(&b.name)));
}

/// Table text for the UI column, shared by the TUI and `status`.
pub fn ui_watch_label(ui_watch: Option<bool>) -> &'static str {
    match ui_watch {
        None => "—",
        Some(false) => "ui",
        Some(true) => "watch",
    }
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

pub fn print_status_table(views: &[WorkerView], stack_name: &str) {
    if views.is_empty() {
        return;
    }
    let mut last_group: Option<WorkerGroup> = None;
    for v in views {
        if last_group != Some(v.group) {
            if last_group.is_some() {
                println!();
            }
            println!("── {} ──", group_label(v.group, stack_name));
            println!(
                "{:<28} {:<12} {:<12} {:<6} {:<8} {:<8}",
                "WORKER", "PROCESS", "ENGINE", "UI", "PID", "UPTIME"
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
            "{:<28} {:<12} {:<12} {:<6} {:<8} {:<8}",
            name,
            v.process_status,
            v.engine_status,
            ui_watch_label(v.ui_watch),
            pid,
            v.uptime
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn view(name: &str) -> WorkerView {
        WorkerView {
            name: name.to_string(),
            group: WorkerGroup::Other,
            spawnable: true,
            display_status: "stopped".to_string(),
            process_status: "stopped".to_string(),
            engine_status: "—".to_string(),
            local_pid: None,
            uptime: "—".to_string(),
            exit_code: None,
            ui_watch: None,
        }
    }

    #[test]
    fn assign_view_groups_orders_stack_first_alpha_within() {
        let mut views = vec![view("zeta"), view("console"), view("alpha")];
        let members: HashSet<String> = ["zeta".to_string(), "alpha".to_string()]
            .into_iter()
            .collect();
        assign_view_groups(&mut views, &members);
        let order: Vec<(&str, WorkerGroup)> =
            views.iter().map(|v| (v.name.as_str(), v.group)).collect();
        assert_eq!(
            order,
            vec![
                ("alpha", WorkerGroup::Stack),
                ("zeta", WorkerGroup::Stack),
                ("console", WorkerGroup::Other),
            ]
        );
    }

    #[test]
    fn group_labels() {
        assert_eq!(group_label(WorkerGroup::Stack, "console"), "stack:console");
        assert_eq!(group_label(WorkerGroup::Other, "console"), "other");
    }
}
