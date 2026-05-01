//! Budget / Alert / Exemption / SpendLogEntry types + state-backed CRUD.
//! Direct port of roster/workers/llm-budget/src/store.ts.

use iii_sdk::{TriggerRequest, Value, III};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::periods::Period;

pub const SCOPE: &str = "budgets";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alert {
    pub alert_id: String,
    pub threshold_pct: f64,
    pub callback_function_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_payload: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fired_period_start: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exemption {
    pub principal_id: String,
    pub reason: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub ceiling_usd: f64,
    pub period: Period,
    pub spent_usd: f64,
    pub period_start_at: i64,
    pub period_resets_at: i64,
    pub enforced: bool,
    pub paused: bool,
    pub alerts: Vec<Alert>,
    pub exemptions: Vec<Exemption>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpendLogEntry {
    pub budget_id: String,
    pub period_start: i64,
    pub period_end: i64,
    pub spent_usd: f64,
    pub records_count: u64,
}

pub fn budget_key(id: &str) -> String {
    format!("budget:{id}")
}

pub fn spend_log_key(id: &str, period_start: i64) -> String {
    format!("spend_log:{id}:{period_start}")
}

/// Reset-archive key: append a unique suffix so a reset that lands at the
/// same period boundary as the post-reset live period doesn't collide.
pub fn reset_log_key(id: &str, period_start: i64, ts: i64, suffix: &str) -> String {
    format!("spend_log:{id}:{period_start}:reset-{ts}-{suffix}")
}

async fn state_set(iii: &III, key: &str, value: &Value) -> anyhow::Result<()> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".into(),
        payload: json!({ "scope": SCOPE, "key": key, "value": value }),
        action: None,
        timeout_ms: None,
    })
    .await?;
    Ok(())
}

async fn state_get_value(iii: &III, key: &str) -> anyhow::Result<Option<Value>> {
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({ "scope": SCOPE, "key": key }),
            action: None,
            timeout_ms: None,
        })
        .await?;
    if resp.is_null() {
        return Ok(None);
    }
    let value = resp.get("value").cloned().unwrap_or(resp);
    if value.is_null() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

async fn state_list(iii: &III, prefix: &str) -> anyhow::Result<Vec<Value>> {
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "state::list".into(),
            payload: json!({ "scope": SCOPE, "prefix": prefix }),
            action: None,
            timeout_ms: None,
        })
        .await?;
    Ok(resp.as_array().cloned().unwrap_or_default())
}

async fn state_delete(iii: &III, key: &str) -> anyhow::Result<()> {
    iii.trigger(TriggerRequest {
        function_id: "state::delete".into(),
        payload: json!({ "scope": SCOPE, "key": key }),
        action: None,
        timeout_ms: None,
    })
    .await?;
    Ok(())
}

pub async fn load_budget(iii: &III, id: &str) -> anyhow::Result<Option<Budget>> {
    let v = state_get_value(iii, &budget_key(id)).await?;
    match v {
        Some(value) => Ok(Some(serde_json::from_value(value)?)),
        None => Ok(None),
    }
}

pub async fn save_budget(iii: &III, b: &Budget) -> anyhow::Result<()> {
    state_set(iii, &budget_key(&b.id), &serde_json::to_value(b)?).await
}

pub async fn delete_budget(iii: &III, id: &str) -> anyhow::Result<()> {
    state_delete(iii, &budget_key(id)).await
}

pub async fn list_all(iii: &III) -> anyhow::Result<Vec<Budget>> {
    let entries = state_list(iii, "budget:").await?;
    Ok(entries
        .into_iter()
        .filter_map(|v| serde_json::from_value::<Budget>(v).ok())
        .collect())
}

pub async fn save_spend_log(
    iii: &III,
    id: &str,
    period_start: i64,
    e: &SpendLogEntry,
) -> anyhow::Result<()> {
    state_set(
        iii,
        &spend_log_key(id, period_start),
        &serde_json::to_value(e)?,
    )
    .await
}

pub async fn save_reset_log(
    iii: &III,
    id: &str,
    period_start: i64,
    ts: i64,
    suffix: &str,
    e: &SpendLogEntry,
) -> anyhow::Result<()> {
    state_set(
        iii,
        &reset_log_key(id, period_start, ts, suffix),
        &serde_json::to_value(e)?,
    )
    .await
}

pub async fn list_spend_logs(iii: &III, budget_id: &str) -> anyhow::Result<Vec<SpendLogEntry>> {
    let entries = state_list(iii, &format!("spend_log:{budget_id}:")).await?;
    Ok(entries
        .into_iter()
        .filter_map(|v| serde_json::from_value::<SpendLogEntry>(v).ok())
        .filter(|e| e.budget_id == budget_id)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_key_format() {
        assert_eq!(budget_key("b-1"), "budget:b-1");
    }

    #[test]
    fn reset_log_key_appends_suffix() {
        let k = reset_log_key("b", 1000, 2000, "uuid");
        assert_eq!(k, "spend_log:b:1000:reset-2000-uuid");
    }
}
