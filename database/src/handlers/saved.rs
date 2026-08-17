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
//! the user's query. Every write re-stores the capped tail of the list
//! (`history_max_entries` / `history_max_bytes`) — an unbounded value grows
//! until the `state` worker can no longer serve it over its engine
//! connection, which takes `state::*` down for everyone. History is a
//! convenience, not an audit log — for an audit trail, bind the
//! `database::row-changed` trigger instead.

use super::query::err_to_str;
use crate::config::WorkerConfig;
use crate::error::DbError;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex};
use tokio::sync::RwLock;

/// Scope every key lives under in the `state` worker.
pub(super) const SCOPE: &str = "database";
/// Entries returned by `history` unless the caller asks otherwise.
const HISTORY_LIMIT: usize = 50;
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
    let raw = state_get(iii, &history_key(db)).await?;
    let all: Vec<HistoryEntry> = raw
        .iter()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect();

    // The stored list is capped on write, so `take` self-bounds.
    let limit = req.limit.unwrap_or(HISTORY_LIMIT).max(1);
    let entries: Vec<HistoryEntry> = all.into_iter().rev().take(limit).collect();
    Ok(HistoryResp {
        count: entries.len(),
        entries,
    })
}

/// Databases whose stored history could not be read. Their next write skips
/// the read and replaces the value wholesale: merely serving an oversized
/// value can reset the `state` worker's connection, so recovery must never
/// depend on reading the value it is recovering from.
static RESET_PENDING: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

fn reset_pending(db: &str) -> bool {
    RESET_PENDING.lock().is_ok_and(|s| s.contains(db))
}

fn mark_reset_pending(db: &str) {
    if let Ok(mut s) = RESET_PENDING.lock() {
        s.insert(db.to_string());
    }
}

fn clear_reset_pending(db: &str) {
    if let Ok(mut s) = RESET_PENDING.lock() {
        s.remove(db);
    }
}

/// Record one run. Fire and forget: never awaited on the query path, and a
/// failure is logged rather than surfaced, because losing a history line must
/// never fail the query the user actually asked for.
///
/// Each write stores the capped tail of the list: read, append, trim to the
/// configured caps, replace. Last-writer-wins — two concurrent records can
/// drop a line, which the lossy-by-design charter above allows. A stored
/// value that cannot be read (missing worker, or a pre-cap oversized blob) is
/// replaced instead of retried, losing old lines but unwedging `state`.
pub fn record(
    iii: Arc<IIIClient>,
    config: Arc<RwLock<WorkerConfig>>,
    db: String,
    sql: &str,
    duration_ms: u64,
    row_count: usize,
) {
    let entry = HistoryEntry {
        sql: truncate(sql),
        verb: leading_verb(sql),
        duration_ms: Some(duration_ms),
        row_count: Some(row_count),
        at: now(),
    };
    tokio::spawn(async move {
        let (max_entries, max_bytes) = {
            let cfg = config.read().await;
            (cfg.history_max_entries, cfg.history_max_bytes)
        };
        if max_entries == 0 || max_bytes == 0 {
            return;
        }
        let key = history_key(&db);
        let mut items = if reset_pending(&db) {
            Vec::new()
        } else {
            match state_get(&iii, &key).await {
                Ok(items) => items,
                Err(e) => {
                    mark_reset_pending(&db);
                    tracing::warn!(error = %e, "history unreadable; next write resets it");
                    Vec::new()
                }
            }
        };
        items.push(serde_json::to_value(&entry).unwrap_or(json!({})));
        trim_to_caps(&mut items, max_entries, max_bytes);
        match state_set(&iii, &key, Value::Array(items)).await {
            Ok(()) => clear_reset_pending(&db),
            Err(e) => tracing::warn!(error = %e, "history not recorded"),
        }
    });
}

/// Drop oldest entries until the list fits both caps.
///
/// Byte size is the compact `serde_json` encoding, computed without
/// re-serializing the list per drop: `[]` is 2 bytes, `n` entries cost the
/// 2 brackets plus their summed lengths plus `n − 1` commas.
fn trim_to_caps(entries: &mut Vec<Value>, max_entries: usize, max_bytes: usize) {
    let lens: Vec<usize> = entries
        .iter()
        .map(|v| serde_json::to_vec(v).map_or(usize::MAX, |b| b.len()))
        .collect();
    let mut kept = 0usize;
    let mut bytes = 0usize;
    for len in lens.iter().rev() {
        let with_next = bytes
            .saturating_add(*len)
            .saturating_add(2) // brackets
            .saturating_add(kept); // commas once this entry joins
        if kept == max_entries || with_next > max_bytes {
            break;
        }
        bytes += len;
        kept += 1;
    }
    let surplus = entries.len() - kept;
    entries.drain(..surplus);
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
    fn keys_are_scoped_per_database() {
        assert_eq!(saved_key("primary"), "saved:primary");
        assert_eq!(history_key("analytics"), "history:analytics");
    }

    fn entry(sql: &str) -> Value {
        serde_json::to_value(HistoryEntry {
            sql: sql.into(),
            verb: leading_verb(sql),
            duration_ms: Some(12),
            row_count: Some(3),
            at: "2026-08-06T00:00:00+00:00".into(),
        })
        .unwrap()
    }

    #[test]
    fn trim_keeps_newest_within_entry_cap() {
        let mut items: Vec<Value> = (0..5).map(|i| entry(&format!("select {i}"))).collect();
        trim_to_caps(&mut items, 3, usize::MAX);
        let sqls: Vec<&str> = items.iter().map(|v| v["sql"].as_str().unwrap()).collect();
        assert_eq!(sqls, ["select 2", "select 3", "select 4"]);
    }

    #[test]
    fn trim_enforces_byte_cap_dropping_oldest() {
        let mut items: Vec<Value> = (0..10).map(|i| entry(&format!("select {i}"))).collect();
        // One byte short of fitting all ten forces at least one drop.
        let cap = serde_json::to_vec(&Value::Array(items.clone()))
            .unwrap()
            .len()
            - 1;
        trim_to_caps(&mut items, usize::MAX, cap);
        assert!(!items.is_empty() && items.len() < 10);
        assert_eq!(items.last().unwrap()["sql"], "select 9");
        assert!(serde_json::to_vec(&Value::Array(items)).unwrap().len() <= cap);
    }

    #[test]
    fn trim_byte_accounting_matches_serde_exactly() {
        // The trim never re-serializes the whole list, so its arithmetic must
        // match serde's compact encoding to the byte — including multi-byte
        // and escaped content.
        let items = vec![
            entry("select 'plain'"),
            entry("select 'héllo … ↹'"),
            entry("select \"quoted\\backslash\"\n\t"),
        ];
        let summed: usize = items
            .iter()
            .map(|v| serde_json::to_vec(v).unwrap().len())
            .sum();
        let actual = serde_json::to_vec(&Value::Array(items.clone()))
            .unwrap()
            .len();
        assert_eq!(2 + summed + (items.len() - 1), actual);
        assert_eq!(serde_json::to_vec(&Value::Array(vec![])).unwrap().len(), 2);
    }

    #[test]
    fn oversized_single_entry_yields_empty_history() {
        let mut items = vec![entry(&"x".repeat(1_000))];
        trim_to_caps(&mut items, 10, 64);
        assert!(items.is_empty());
    }

    #[test]
    fn trim_noop_when_within_caps() {
        let mut items: Vec<Value> = (0..3).map(|i| entry(&format!("select {i}"))).collect();
        let before = items.clone();
        trim_to_caps(&mut items, 200, 262_144);
        assert_eq!(items, before);
    }

    #[test]
    fn reset_flag_marks_and_clears_per_database() {
        // Unique names: the flag set is a process-wide static shared by tests.
        assert!(!reset_pending("reset-flag-test-a"));
        mark_reset_pending("reset-flag-test-a");
        assert!(reset_pending("reset-flag-test-a"));
        assert!(!reset_pending("reset-flag-test-b"));
        clear_reset_pending("reset-flag-test-a");
        assert!(!reset_pending("reset-flag-test-a"));
    }
}
