//! `database::getTableView` / `saveTableView` — how a table is laid out.
//!
//! Column widths, hidden columns and column order. These are preferences, not
//! data, and the obvious place to keep them is browser storage — which is
//! exactly where the query history used to live, for one person, in one
//! browser, until a cache clear.
//!
//! On `state::*` instead they survive a restart, follow the operator to
//! another machine, and are legible to anything else on the bus. An agent
//! preparing a table for someone to look at can widen the column that matters
//! and hide the six that do not.
//!
//! Deliberately *not* validated against the live schema. A view saved before a
//! column was renamed should degrade to "that column has no stored width", not
//! fail the read; the renderer already treats every entry as optional.

use super::saved::{call, SCOPE};
use crate::error::DbError;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

/// Upper bound on stored columns per table. A view is a preference, not a
/// place to accumulate unbounded state from a caller.
const MAX_COLUMNS: usize = 1_000;
/// Clamped so a stored width cannot render a column unusable or push the grid
/// to an absurd scroll width.
const MIN_WIDTH: f64 = 48.0;
const MAX_WIDTH: f64 = 1_200.0;

fn view_key(db: &str, table: &str) -> String {
    format!("view:{db}:{table}")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct TableView {
    /// Per-column pixel width. Absent means "size to content".
    #[serde(default)]
    pub widths: std::collections::BTreeMap<String, f64>,
    /// Columns the reader has hidden. Order is not meaningful.
    #[serde(default)]
    pub hidden: Vec<String>,
    /// Column display order. Names not listed keep their natural position
    /// after those that are, so adding a column to the table does not require
    /// re-saving the view.
    #[serde(default)]
    pub order: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetTableViewReq {
    #[serde(default)]
    pub db: Option<String>,
    pub table: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveTableViewReq {
    #[serde(default)]
    pub db: Option<String>,
    pub table: String,
    #[serde(flatten)]
    pub view: TableView,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SaveTableViewResp {
    pub saved: bool,
}

pub async fn get(iii: &Arc<IIIClient>, db: &str, table: &str) -> Result<TableView, String> {
    let raw = call(
        iii,
        "state::get",
        json!({"scope": SCOPE, "key": view_key(db, table)}),
    )
    .await?;
    let value = raw.get("value").cloned().unwrap_or(raw);
    // A missing or unparseable view is the default one. Losing a layout is a
    // far smaller problem than refusing to show the table.
    Ok(serde_json::from_value(value).unwrap_or_default())
}

pub async fn save(
    iii: &Arc<IIIClient>,
    db: &str,
    req: SaveTableViewReq,
) -> Result<SaveTableViewResp, String> {
    if req.table.trim().is_empty() {
        return Err(super::query::err_to_str(DbError::InvalidParam {
            index: 0,
            reason: "table is required".into(),
        }));
    }
    let view = sanitise(req.view)?;
    call(
        iii,
        "state::set",
        json!({
            "scope": SCOPE,
            "key": view_key(db, &req.table),
            "value": serde_json::to_value(&view).unwrap_or(Value::Null),
        }),
    )
    .await?;
    Ok(SaveTableViewResp { saved: true })
}

/// Clamp widths and bound the lists. The caller is a renderer, so this is not
/// hostile input, but a stored `width: 1e9` would be a layout no one could
/// undo without editing state by hand.
fn sanitise(mut view: TableView) -> Result<TableView, String> {
    let too_many = view.widths.len() > MAX_COLUMNS
        || view.hidden.len() > MAX_COLUMNS
        || view.order.len() > MAX_COLUMNS;
    if too_many {
        return Err(super::query::err_to_str(DbError::InvalidParam {
            index: 0,
            reason: format!("a view may describe at most {MAX_COLUMNS} columns"),
        }));
    }
    view.widths = view
        .widths
        .into_iter()
        .filter(|(_, w)| w.is_finite())
        .map(|(k, w)| (k, w.clamp(MIN_WIDTH, MAX_WIDTH)))
        .collect();
    view.hidden.sort();
    view.hidden.dedup();
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_scoped_per_table() {
        assert_eq!(view_key("primary", "orders"), "view:primary:orders");
    }

    #[test]
    fn widths_are_clamped_into_a_usable_range() {
        let mut v = TableView::default();
        v.widths.insert("a".into(), 5.0);
        v.widths.insert("b".into(), 99_999.0);
        let out = sanitise(v).unwrap();
        assert_eq!(out.widths["a"], MIN_WIDTH);
        assert_eq!(out.widths["b"], MAX_WIDTH);
    }

    #[test]
    fn non_finite_widths_are_dropped_rather_than_stored() {
        let mut v = TableView::default();
        v.widths.insert("a".into(), f64::NAN);
        v.widths.insert("b".into(), f64::INFINITY);
        v.widths.insert("c".into(), 200.0);
        let out = sanitise(v).unwrap();
        assert_eq!(out.widths.len(), 1);
        assert!(out.widths.contains_key("c"));
    }

    #[test]
    fn hidden_columns_are_deduplicated() {
        let v = TableView {
            hidden: vec!["b".into(), "a".into(), "b".into()],
            ..Default::default()
        };
        let out = sanitise(v).unwrap();
        assert_eq!(out.hidden, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn an_oversized_view_is_refused() {
        let mut v = TableView::default();
        for i in 0..(MAX_COLUMNS + 1) {
            v.widths.insert(format!("c{i}"), 100.0);
        }
        assert!(sanitise(v).is_err());
    }
}
