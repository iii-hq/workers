//! `database::saveQuery` / `listSavedQueries` / `deleteSavedQuery` /
//! `history` — the queries you keep, and the ones you ran.
//!
//! These are thin wrappers over the `state` worker, not a store of their own.
//! Query history used to live in browser `localStorage`, which meant it
//! existed for one person, in one browser, and vanished on a cache clear. On
//! `state::*` it survives restarts, any agent can read it, and an agent can
//! save a query for a human to find in the console.
//!
//! Recording is deliberately cheap and deliberately lossy: it is fire and
//! forget, never awaited on the query path, and a `state` failure never fails
//! the user's query. History is a convenience, not an audit log — for an
//! audit trail, bind the `database::row-changed` trigger instead.

use super::query::err_to_str;
use crate::error::DbError;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

/// Scope every key lives under in the `state` worker.
pub(super) const SCOPE: &str = "database";
/// Entries returned by `history` unless the caller asks for fewer.
const HISTORY_LIMIT: usize = 50;
/// Stored entries are trimmed back to `HISTORY_LIMIT` once they exceed this.
/// Trimming on read rather than on write keeps the hot path a single atomic
/// append instead of a read-modify-write.
const HISTORY_HIGH_WATER: usize = 200;
/// SQL longer than this is truncated before it is stored.
const MAX_SQL_CHARS: usize = 4_000;

fn saved_key(db: &str) -> String {
    format!("saved:{db}")
}

fn history_key(db: &str) -> String {
    format!("history:{db}")
}

pub(super) async fn call(
    iii: &Arc<IIIClient>,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
    iii.trigger(TriggerRequest {
        function_id: function_id.to_string(),
        payload,
        action: None,
        timeout_ms: Some(10_000),
    })
    .await
    .map_err(|e| {
        err_to_str(DbError::ConfigError {
            message: format!(
                "{function_id} failed: {e}. Saved queries and history need the `state` \
                 worker — run `iii worker add state`."
            ),
        })
    })
}

async fn state_get(iii: &Arc<IIIClient>, key: &str) -> Result<Vec<Value>, String> {
    let raw = call(iii, "state::get", json!({"scope": SCOPE, "key": key})).await?;
    // A missing key is an empty list, not an error.
    Ok(raw
        .get("value")
        .or(Some(&raw))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

async fn state_set(iii: &Arc<IIIClient>, key: &str, value: Value) -> Result<(), String> {
    call(
        iii,
        "state::set",
        json!({"scope": SCOPE, "key": key, "value": value}),
    )
    .await?;
    Ok(())
}

/* ---------------- saved queries ---------------- */

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SavedQuery {
    pub id: String,
    pub name: String,
    pub sql: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub saved_at: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveQueryReq {
    #[serde(default)]
    pub db: Option<String>,
    pub name: String,
    pub sql: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SaveQueryResp {
    pub id: String,
    pub replaced: bool,
}

pub async fn save(
    iii: &Arc<IIIClient>,
    db: &str,
    req: SaveQueryReq,
) -> Result<SaveQueryResp, String> {
    if req.name.trim().is_empty() || req.sql.trim().is_empty() {
        return Err(err_to_str(DbError::InvalidParam {
            index: 0,
            reason: "name and sql are both required".into(),
        }));
    }
    let key = saved_key(db);
    let mut items: Vec<SavedQuery> = state_get(iii, &key)
        .await?
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();

    // Saving under an existing name replaces it rather than accumulating
    // near-duplicates the user then has to tell apart.
    let replaced = items.iter().any(|q| q.name == req.name);
    let id = items
        .iter()
        .find(|q| q.name == req.name)
        .map(|q| q.id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    items.retain(|q| q.name != req.name);
    items.push(SavedQuery {
        id: id.clone(),
        name: req.name,
        sql: truncate(&req.sql),
        description: req.description,
        saved_at: now(),
    });
    items.sort_by(|a, b| a.name.cmp(&b.name));

    state_set(iii, &key, serde_json::to_value(&items).unwrap_or(json!([]))).await?;
    Ok(SaveQueryResp { id, replaced })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListSavedReq {
    #[serde(default)]
    pub db: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListSavedResp {
    pub queries: Vec<SavedQuery>,
    pub count: usize,
}

pub async fn list(iii: &Arc<IIIClient>, db: &str) -> Result<ListSavedResp, String> {
    let queries: Vec<SavedQuery> = state_get(iii, &saved_key(db))
        .await?
        .into_iter()
        // Drop anything that no longer parses rather than failing the call —
        // a stale entry should not make the list unreadable.
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();
    Ok(ListSavedResp {
        count: queries.len(),
        queries,
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteSavedReq {
    #[serde(default)]
    pub db: Option<String>,
    /// Either the id returned by `saveQuery`, or the name it was saved under.
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeleteSavedResp {
    pub deleted: bool,
}

pub async fn delete(
    iii: &Arc<IIIClient>,
    db: &str,
    req: DeleteSavedReq,
) -> Result<DeleteSavedResp, String> {
    let key = saved_key(db);
    let mut items: Vec<SavedQuery> = state_get(iii, &key)
        .await?
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();
    let before = items.len();
    items.retain(|q| q.id != req.id && q.name != req.id);
    let deleted = items.len() != before;
    if deleted {
        state_set(iii, &key, serde_json::to_value(&items).unwrap_or(json!([]))).await?;
    }
    Ok(DeleteSavedResp { deleted })
}

/* ---------------- history ---------------- */

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HistoryEntry {
    pub sql: String,
    pub verb: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_count: Option<usize>,
    pub at: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HistoryReq {
    #[serde(default)]
    pub db: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct HistoryResp {
    pub entries: Vec<HistoryEntry>,
    pub count: usize,
}

pub async fn history(
    iii: &Arc<IIIClient>,
    db: &str,
    req: HistoryReq,
) -> Result<HistoryResp, String> {
    let key = history_key(db);
    let raw = state_get(iii, &key).await?;
    let all: Vec<HistoryEntry> = raw
        .iter()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect();

    // Self-healing trim: the write path only appends, so the stored list is
    // capped here once it drifts past the high-water mark.
    if all.len() > HISTORY_HIGH_WATER {
        let tail: Vec<&HistoryEntry> = all.iter().rev().take(HISTORY_LIMIT).rev().collect();
        let _ = state_set(iii, &key, serde_json::to_value(&tail).unwrap_or(json!([]))).await;
    }

    let limit = req
        .limit
        .unwrap_or(HISTORY_LIMIT)
        .clamp(1, HISTORY_HIGH_WATER);
    let entries: Vec<HistoryEntry> = all.into_iter().rev().take(limit).collect();
    Ok(HistoryResp {
        count: entries.len(),
        entries,
    })
}

/// The `state::update` op list for appending one entry.
///
/// Split out only so a test can assert the discriminator, which is the one
/// detail of this file that cannot be caught at runtime.
fn append_ops(entry: Value) -> Vec<Value> {
    vec![json!({ "type": "append", "value": entry })]
}

/// Record one run. Fire and forget: never awaited on the query path, and a
/// failure is logged rather than surfaced, because losing a history line must
/// never fail the query the user actually asked for.
pub fn record(iii: Arc<IIIClient>, db: String, sql: &str, duration_ms: u64, row_count: usize) {
    let entry = HistoryEntry {
        sql: truncate(sql),
        verb: leading_verb(sql),
        duration_ms: Some(duration_ms),
        row_count: Some(row_count),
        at: now(),
    };
    tokio::spawn(async move {
        let payload = json!({
            "scope": SCOPE,
            "key": history_key(&db),
            "ops": append_ops(serde_json::to_value(&entry).unwrap_or(json!({}))),
        });
        if let Err(e) = call(&iii, "state::update", payload).await {
            tracing::warn!(error = %e, "history not recorded");
        }
    });
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn truncate(sql: &str) -> String {
    if sql.chars().count() <= MAX_SQL_CHARS {
        return sql.to_string();
    }
    let head: String = sql.chars().take(MAX_SQL_CHARS).collect();
    format!("{head}…")
}

/// First keyword, lowercased — enough to group history without storing a
/// parse tree.
fn leading_verb(sql: &str) -> String {
    sql.split(|c: char| !c.is_ascii_alphabetic())
        .find(|w| !w.is_empty())
        .unwrap_or("unknown")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_verb_ignores_punctuation_and_case() {
        assert_eq!(leading_verb("  SELECT * FROM t"), "select");
        assert_eq!(leading_verb("(select 1)"), "select");
        assert_eq!(leading_verb("INSERT INTO t VALUES (1)"), "insert");
        assert_eq!(leading_verb(""), "unknown");
    }

    #[test]
    fn long_sql_is_truncated_with_a_marker() {
        let long = "x".repeat(MAX_SQL_CHARS + 50);
        let t = truncate(&long);
        assert_eq!(t.chars().count(), MAX_SQL_CHARS + 1);
        assert!(t.ends_with('…'));

        let short = "SELECT 1";
        assert_eq!(truncate(short), short);
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        // A byte-based cut would slice a multi-byte character in half.
        let s = "é".repeat(MAX_SQL_CHARS + 10);
        let t = truncate(&s);
        assert_eq!(t.chars().count(), MAX_SQL_CHARS + 1);
    }

    #[test]
    fn append_op_uses_the_type_discriminator() {
        // Locked deliberately: `record` is fire-and-forget, so a wrong op
        // shape fails where nobody is looking. This is the only cheap place
        // to notice.
        let ops = append_ops(json!({"sql": "select 1"}));
        assert_eq!(ops[0]["type"], "append");
        assert!(ops[0].get("op").is_none());
    }

    #[test]
    fn keys_are_scoped_per_database() {
        assert_eq!(saved_key("primary"), "saved:primary");
        assert_eq!(history_key("analytics"), "history:analytics");
    }
}
