//! Point-in-time CI baseline, called only during setup or explicit recovery.
//! This is per-entity state, not a required-check or mergeability rollup.
use std::collections::BTreeMap;

use serde_json::Value;

use crate::webhooks::{CiEntity, Failure, Result, Service, Snapshot};

impl Service {
    /// Validate selected workflow identities once before delivering an aggregate success.
    pub(crate) async fn confirm_selected_workflows(
        &self,
        repo: &str,
        mut snapshot: Snapshot,
        policy: &super::super::notifications::NotificationPolicy,
    ) -> Result<Snapshot> {
        for (key, ci) in snapshot.ci.iter_mut().filter(|(key, ci)| {
            key.starts_with("workflow_run:")
                && policy.success_checks.contains(&format!(
                    "workflow_run:{}",
                    ci.name.as_deref().unwrap_or("")
                ))
        }) {
            let id = key.strip_prefix("workflow_run:").unwrap_or("");
            if id.parse::<u64>().is_err() {
                return Err(Failure::Invalid("invalid workflow run identity".into()));
            }
            let run = self
                .api("GET", &format!("repos/{repo}/actions/runs/{id}"), None)
                .await?;
            if run["head_sha"].as_str() != Some(ci.sha.as_str()) {
                return Err(Failure::Invalid(
                    "workflow head changed during confirmation".into(),
                ));
            }
            ci.status = text(&run, "status").unwrap_or_else(|| "unknown".into());
            ci.conclusion = text(&run, "conclusion");
            ci.attempt = run["run_attempt"].as_u64().unwrap_or(1);
        }
        Ok(snapshot)
    }
    pub(crate) async fn reconcile_ci(
        &self,
        repo: &str,
        mut snapshot: Snapshot,
    ) -> Result<Snapshot> {
        let Some(sha) = snapshot.head_sha.as_deref() else {
            return Ok(snapshot);
        };
        let mut checks = Vec::new();
        for page in 1..=5 {
            let value = self
                .api(
                    "GET",
                    &format!("repos/{repo}/commits/{sha}/check-runs?per_page=100&page={page}"),
                    None,
                )
                .await?;
            let rows = value
                .get("check_runs")
                .and_then(Value::as_array)
                .ok_or_else(|| Failure::Invalid("invalid check-runs baseline".into()))?;
            if value
                .get("total_count")
                .and_then(Value::as_u64)
                .is_some_and(|n| n > 500)
            {
                return Err(Failure::Invalid(
                    "CI baseline exceeds 500 checks; narrow monitoring or inspect manually".into(),
                ));
            }
            checks.extend(rows.iter().cloned());
            if rows.len() < 100 {
                break;
            }
        }
        let mut statuses = Vec::new();
        for page in 1..=5 {
            let value = self
                .api(
                    "GET",
                    &format!("repos/{repo}/commits/{sha}/status?per_page=100&page={page}"),
                    None,
                )
                .await?;
            let rows = value
                .get("statuses")
                .and_then(Value::as_array)
                .ok_or_else(|| Failure::Invalid("invalid commit-status baseline".into()))?;
            if value
                .get("total_count")
                .and_then(Value::as_u64)
                .is_some_and(|n| n > 500)
            {
                return Err(Failure::Invalid(
                    "CI baseline exceeds 500 statuses; inspect manually".into(),
                ));
            }
            statuses.extend(rows.iter().cloned());
            if rows.len() < 100 {
                break;
            }
        }
        apply_baseline(&mut snapshot, &checks, &statuses)?;
        Ok(snapshot)
    }
}

fn text(item: &Value, field: &str) -> Option<String> {
    item.get(field).and_then(Value::as_str).map(str::to_owned)
}

fn apply_baseline(snapshot: &mut Snapshot, checks: &[Value], statuses: &[Value]) -> Result<()> {
    let sha = snapshot
        .head_sha
        .as_deref()
        .ok_or_else(|| Failure::Invalid("CI baseline requires head SHA".into()))?;
    let mut baseline = BTreeMap::new();
    for (kind, rows) in [("check_run", checks), ("status", statuses)] {
        for row in rows {
            if row
                .get("head_sha")
                .and_then(Value::as_str)
                .is_some_and(|s| s != sha)
            {
                continue;
            }
            let entity = if kind == "check_run" {
                row.get("id")
                    .and_then(Value::as_u64)
                    .map(|id| id.to_string())
            } else {
                text(row, "context")
            }
            .ok_or_else(|| Failure::Invalid("CI baseline entity has no identity".into()))?;
            let state = CiEntity {
                name: text(row, "name").or_else(|| text(row, "context")),
                html_url: text(row, "html_url")
                    .or_else(|| text(row, "details_url"))
                    .or_else(|| text(row, "target_url")),
                sha: sha.into(),
                status: text(row, "status")
                    .or_else(|| text(row, "state"))
                    .unwrap_or_else(|| "unknown".into()),
                conclusion: text(row, "conclusion"),
                attempt: row.get("run_attempt").and_then(Value::as_u64).unwrap_or(1),
                updated_at: text(row, "updated_at")
                    .or_else(|| text(row, "completed_at"))
                    .or_else(|| text(row, "started_at"))
                    .or_else(|| text(row, "created_at"))
                    .unwrap_or_default(),
            };
            let key = format!("{kind}:{entity}");
            if baseline
                .get(&key)
                .is_some_and(|old: &CiEntity| old.updated_at > state.updated_at)
            {
                continue;
            }
            baseline.insert(key, state);
        }
    }
    // Workflow runs remain event-driven; never label the whole PR successful.
    snapshot
        .ci
        .retain(|key, _| key.starts_with("workflow_run:"));
    snapshot.ci.extend(baseline);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn initial_ci_includes_existing_checks_and_latest_commit_status_per_context() {
        let mut snapshot = Snapshot {
            head_sha: Some("head".into()),
            ..Default::default()
        };
        apply_baseline(&mut snapshot, &[
            json!({"id":1,"name":"tests","head_sha":"head","status":"completed","conclusion":"success"}),
            json!({"id":2,"name":"lint","head_sha":"head","status":"in_progress"}),
            json!({"id":3,"head_sha":"other","status":"completed"})
        ], &[
            json!({"context":"external","state":"success","updated_at":"2026-01-02T00:00:00Z"}),
            json!({"context":"external","state":"pending","updated_at":"2026-01-01T00:00:00Z"})
        ]).unwrap();
        assert_eq!(snapshot.ci.len(), 3);
        assert_eq!(snapshot.ci["check_run:1"].name.as_deref(), Some("tests"));
        assert_eq!(snapshot.ci["check_run:2"].status, "in_progress");
        assert_eq!(snapshot.ci["status:external"].status, "success");
    }
}
