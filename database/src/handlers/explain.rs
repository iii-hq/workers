//! `database::explain` — a query plan as a tree, not a grid of text.
//!
//! Every driver spells a plan differently: postgres emits nested JSON, mysql
//! emits a differently-nested JSON, and sqlite emits a flat `(id, parent,
//! detail)` set that has to be reassembled. Callers should not have to know
//! that, so all three collapse into one `PlanNode` tree with the same fields
//! and the same warnings.
//!
//! **`ANALYZE` really executes the statement.** `EXPLAIN ANALYZE DELETE FROM
//! users` deletes users. It is off by default and refused outright for
//! anything that does not parse as a read — the console's own check is a
//! convenience, this one is the authority.

use super::query::{self, err_to_str, QueryReq};
use super::tx_sql_guard;
use super::AppState;
use crate::config::DriverKind;
use crate::error::DbError;
use crate::pool::Pool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

fn default_timeout() -> u64 {
    30_000
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NodeClass {
    Scan,
    Index,
    Join,
    Sort,
    Aggregate,
    Cte,
    Limit,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanFormat {
    PgJson,
    SqliteQueryPlan,
    MysqlJson,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    /// A sequential scan over a large relation.
    SeqScanLarge,
    /// Estimated and actual row counts differ by an order of magnitude —
    /// usually stale statistics.
    EstimateSkew,
    /// An inner loop executed a very large number of times.
    NestedLoopLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warn,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PlanNode {
    pub id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<u32>,
    pub label: String,
    pub node_class: NodeClass,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_startup: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows_estimated: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows_actual: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loops: Option<f64>,
    pub detail: String,
    pub children: Vec<PlanNode>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PlanWarning {
    pub node_id: u32,
    pub kind: WarningKind,
    pub message: String,
    pub severity: Severity,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExplainReq {
    #[serde(default)]
    pub db: Option<String>,
    pub sql: String,
    #[serde(default, deserialize_with = "crate::handlers::lenient_params")]
    pub params: Vec<Value>,
    /// Runs the statement to collect real timings. Refused for anything that
    /// is not a read.
    #[serde(default)]
    pub analyze: bool,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ExplainResp {
    pub format: PlanFormat,
    pub analyzed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<PlanNode>,
    pub warnings: Vec<PlanWarning>,
    /// The driver's own output, so a caller is never stuck when the shape is
    /// one we do not recognise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

/// Rows above this make a sequential scan worth flagging.
const LARGE_ROWS: f64 = 10_000.0;
/// Estimate/actual ratio worth flagging as stale statistics.
const SKEW_FACTOR: f64 = 10.0;
/// Loop count worth flagging on a nested loop.
const LARGE_LOOPS: f64 = 1_000.0;

pub fn classify(label: &str) -> NodeClass {
    let l = label.to_ascii_lowercase();
    if l.contains("index") {
        NodeClass::Index
    } else if l.contains("scan") || l.contains("seek") {
        NodeClass::Scan
    } else if l.contains("join") || l.contains("nested loop") {
        NodeClass::Join
    } else if l.contains("sort") || l.contains("b-tree") {
        NodeClass::Sort
    } else if l.contains("aggregate") || l.contains("group") {
        NodeClass::Aggregate
    } else if l.contains("cte") || l.contains("subquery") {
        NodeClass::Cte
    } else if l.contains("limit") {
        NodeClass::Limit
    } else {
        NodeClass::Other
    }
}

async fn driver_of(state: &AppState, db: &str) -> Result<DriverKind, String> {
    Ok(match state.pool(db).await.map_err(err_to_str)? {
        Pool::Sqlite(_) => DriverKind::Sqlite,
        Pool::Postgres(_) => DriverKind::Postgres,
        Pool::Mysql(_) => DriverKind::Mysql,
    })
}

pub async fn handle(state: &AppState, req: ExplainReq) -> Result<ExplainResp, String> {
    if req.sql.trim().is_empty() {
        return Err(err_to_str(DbError::InvalidParam {
            index: 0,
            reason: "sql is required".into(),
        }));
    }
    let db = state.resolve_db(req.db.clone()).await.map_err(err_to_str)?;
    let driver = driver_of(state, &db).await?;

    // The gate that matters. ANALYZE executes; refuse it on anything that is
    // not unambiguously a read.
    if req.analyze && !tx_sql_guard::is_read_only_sql(&req.sql) {
        return Err(err_to_str(DbError::InvalidParam {
            index: 0,
            reason: "analyze runs the statement, so it is only allowed for a single \
                     read-only statement; run it without analyze to see the estimated plan"
                .into(),
        }));
    }
    // SQLite's EXPLAIN QUERY PLAN has no ANALYZE form.
    let analyzed = req.analyze && driver != DriverKind::Sqlite;

    let prefixed = match (driver, analyzed) {
        (DriverKind::Postgres, true) => format!("EXPLAIN (FORMAT JSON, ANALYZE true) {}", req.sql),
        (DriverKind::Postgres, false) => format!("EXPLAIN (FORMAT JSON) {}", req.sql),
        (DriverKind::Mysql, _) => format!("EXPLAIN FORMAT=JSON {}", req.sql),
        (DriverKind::Sqlite, _) => format!("EXPLAIN QUERY PLAN {}", req.sql),
    };

    let resp = query::handle(
        state,
        QueryReq {
            db: Some(db),
            sql: prefixed,
            params: req.params,
            timeout_ms: req.timeout_ms,
            record_history: false,
        },
    )
    .await?;

    let (format, root) = match driver {
        DriverKind::Postgres => parse_pg(&resp.rows),
        DriverKind::Mysql => parse_mysql(&resp.rows),
        DriverKind::Sqlite => parse_sqlite(&resp.rows),
    };
    let warnings = root.as_ref().map(warnings_for).unwrap_or_default();
    let raw = (format == PlanFormat::Unknown)
        .then(|| Value::Array(resp.rows.iter().cloned().map(Value::Object).collect()));

    Ok(ExplainResp {
        format,
        analyzed,
        root,
        warnings,
        raw,
    })
}

fn num(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.parse().ok(),
        _ => None,
    }
}

/// The driver may hand back a JSON column already parsed, or as text.
fn as_json(v: &Value) -> Option<Value> {
    match v {
        Value::String(s) => serde_json::from_str(s).ok(),
        other => Some(other.clone()),
    }
}

fn first_value(rows: &[serde_json::Map<String, Value>]) -> Option<Value> {
    rows.first()?.values().next().cloned()
}

/* ---------------- postgres ---------------- */

fn parse_pg(rows: &[serde_json::Map<String, Value>]) -> (PlanFormat, Option<PlanNode>) {
    let Some(parsed) = first_value(rows).as_ref().and_then(as_json) else {
        return (PlanFormat::Unknown, None);
    };
    // EXPLAIN (FORMAT JSON) wraps the plan in a single-element array.
    let plan = parsed
        .as_array()
        .and_then(|a| a.first())
        .and_then(|o| o.get("Plan"))
        .cloned();
    match plan {
        Some(p) => {
            let mut next_id = 0;
            (PlanFormat::PgJson, Some(pg_node(&p, None, &mut next_id)))
        }
        None => (PlanFormat::Unknown, None),
    }
}

fn pg_node(v: &Value, parent: Option<u32>, next_id: &mut u32) -> PlanNode {
    let id = *next_id;
    *next_id += 1;

    let label = v
        .get("Node Type")
        .and_then(Value::as_str)
        .unwrap_or("Unknown")
        .to_string();
    let relation = v
        .get("Relation Name")
        .and_then(Value::as_str)
        .map(str::to_string);

    let mut detail = label.clone();
    if let Some(r) = &relation {
        detail = format!("{detail} on {r}");
    }

    let children = v
        .get("Plans")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|c| pg_node(c, Some(id), next_id)).collect())
        .unwrap_or_default();

    PlanNode {
        id,
        parent,
        node_class: classify(&label),
        label,
        relation,
        cost_startup: num(v.get("Startup Cost")),
        cost_total: num(v.get("Total Cost")),
        rows_estimated: num(v.get("Plan Rows")),
        rows_actual: num(v.get("Actual Rows")),
        width: num(v.get("Plan Width")).map(|w| w as i64),
        time_ms: num(v.get("Actual Total Time")),
        loops: num(v.get("Actual Loops")),
        detail,
        children,
    }
}

/* ---------------- mysql ---------------- */

fn parse_mysql(rows: &[serde_json::Map<String, Value>]) -> (PlanFormat, Option<PlanNode>) {
    let Some(parsed) = first_value(rows).as_ref().and_then(as_json) else {
        return (PlanFormat::Unknown, None);
    };
    let Some(block) = parsed.get("query_block") else {
        return (PlanFormat::Unknown, None);
    };
    let mut next_id = 0;
    (
        PlanFormat::MysqlJson,
        Some(mysql_node(block, None, &mut next_id, "query_block")),
    )
}

/// MySQL's shape is a loose bag of nested objects rather than a uniform node
/// list, so walk it generically: any nested object carrying a `table` or
/// another recognisable block becomes a child.
fn mysql_node(v: &Value, parent: Option<u32>, next_id: &mut u32, label: &str) -> PlanNode {
    let id = *next_id;
    *next_id += 1;

    let table = v.get("table");
    let relation = table
        .and_then(|t| t.get("table_name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let access = table
        .and_then(|t| t.get("access_type"))
        .and_then(Value::as_str)
        .unwrap_or(label);

    let cost = table
        .and_then(|t| t.get("cost_info"))
        .and_then(|c| c.get("read_cost").or_else(|| c.get("query_cost")));

    let mut children = Vec::new();
    if let Some(obj) = v.as_object() {
        for (k, child) in obj {
            if k == "table" || k == "cost_info" {
                continue;
            }
            match child {
                Value::Object(_) => children.push(mysql_node(child, Some(id), next_id, k)),
                Value::Array(items) => {
                    for item in items.iter().filter(|i| i.is_object()) {
                        children.push(mysql_node(item, Some(id), next_id, k));
                    }
                }
                _ => {}
            }
        }
    }

    let label = access.to_string();
    PlanNode {
        id,
        parent,
        node_class: classify(&label),
        detail: match &relation {
            Some(r) => format!("{label} on {r}"),
            None => label.clone(),
        },
        label,
        relation,
        cost_startup: None,
        cost_total: num(cost),
        rows_estimated: num(table.and_then(|t| t.get("rows_examined_per_scan"))),
        rows_actual: num(table.and_then(|t| t.get("rows_produced_per_join"))),
        width: None,
        time_ms: None,
        loops: None,
        children,
    }
}

/* ---------------- sqlite ---------------- */

/// `EXPLAIN QUERY PLAN` returns a flat `(id, parent, notused, detail)` set;
/// the tree is implied by `parent` and has to be rebuilt.
fn parse_sqlite(rows: &[serde_json::Map<String, Value>]) -> (PlanFormat, Option<PlanNode>) {
    if rows.is_empty() {
        return (PlanFormat::Unknown, None);
    }
    let flat: Vec<(u32, u32, String)> = rows
        .iter()
        .filter_map(|r| {
            let id = num(r.get("id"))? as u32;
            let parent = num(r.get("parent")).unwrap_or(0.0) as u32;
            let detail = r.get("detail").and_then(Value::as_str)?.to_string();
            Some((id, parent, detail))
        })
        .collect();
    if flat.is_empty() {
        return (PlanFormat::Unknown, None);
    }

    // Group by parent first, then build the tree in one recursive pass. Doing
    // it this way avoids searching a partly-built tree for each row, and it
    // tolerates rows arriving in any order.
    let mut children_of: std::collections::HashMap<u32, Vec<(u32, String)>> =
        std::collections::HashMap::new();
    for (id, parent, detail) in flat {
        children_of.entry(parent).or_default().push((id, detail));
    }

    // sqlite numbers top-level rows with parent 0, so synthesise a root to
    // hang them from rather than promoting an arbitrary step.
    let root = PlanNode {
        id: 0,
        parent: None,
        label: "QUERY PLAN".into(),
        node_class: NodeClass::Other,
        relation: None,
        cost_startup: None,
        cost_total: None,
        rows_estimated: None,
        rows_actual: None,
        width: None,
        time_ms: None,
        loops: None,
        detail: "QUERY PLAN".into(),
        children: sqlite_children(0, &children_of, 0),
    };
    (PlanFormat::SqliteQueryPlan, Some(root))
}

/// Pull the relation out of an `EXPLAIN QUERY PLAN` detail string.
///
/// SQLite changed this wording: older versions say `SCAN TABLE users`, 3.36
/// and later say `SCAN users`. Both forms are accepted, as is the `SUBQUERY`
/// spelling, so the relation does not silently go missing on one build.
fn sqlite_relation(detail: &str) -> Option<String> {
    let mut words = detail
        .split_whitespace()
        .skip_while(|w| !matches!(*w, "SCAN" | "SEARCH"))
        .skip(1);
    let first = words.next()?;
    let name = match first {
        "TABLE" | "SUBQUERY" => words.next()?,
        other => other,
    };
    // `SCAN users USING INDEX ...` — the name never starts a clause keyword.
    (!matches!(name, "USING" | "AS" | "COVERING")).then(|| name.to_string())
}

/// Depth bound guards against a malformed set whose parent links form a cycle.
const MAX_PLAN_DEPTH: u32 = 64;

fn sqlite_children(
    parent: u32,
    children_of: &std::collections::HashMap<u32, Vec<(u32, String)>>,
    depth: u32,
) -> Vec<PlanNode> {
    if depth >= MAX_PLAN_DEPTH {
        return Vec::new();
    }
    children_of
        .get(&parent)
        .map(|kids| {
            kids.iter()
                .map(|(id, detail)| PlanNode {
                    id: *id,
                    parent: Some(parent),
                    node_class: classify(detail),
                    label: detail.clone(),
                    relation: sqlite_relation(detail),
                    cost_startup: None,
                    cost_total: None,
                    rows_estimated: None,
                    rows_actual: None,
                    width: None,
                    time_ms: None,
                    loops: None,
                    detail: detail.clone(),
                    children: sqlite_children(*id, children_of, depth + 1),
                })
                .collect()
        })
        .unwrap_or_default()
}

/* ---------------- warnings ---------------- */

pub fn warnings_for(root: &PlanNode) -> Vec<PlanWarning> {
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

fn walk(node: &PlanNode, out: &mut Vec<PlanWarning>) {
    let rows = node.rows_actual.or(node.rows_estimated).unwrap_or(0.0);

    if node.node_class == NodeClass::Scan
        && !node.label.to_ascii_lowercase().contains("index")
        && rows > LARGE_ROWS
    {
        out.push(PlanWarning {
            node_id: node.id,
            kind: WarningKind::SeqScanLarge,
            message: format!(
                "sequential scan over ~{rows:.0} rows{}; an index on the filtered \
                 column would avoid reading the whole relation",
                node.relation
                    .as_ref()
                    .map(|r| format!(" of {r}"))
                    .unwrap_or_default()
            ),
            severity: Severity::Warn,
        });
    }

    if let (Some(est), Some(act)) = (node.rows_estimated, node.rows_actual) {
        let hi = est.max(act);
        let lo = est.min(act).max(1.0);
        if hi / lo > SKEW_FACTOR {
            out.push(PlanWarning {
                node_id: node.id,
                kind: WarningKind::EstimateSkew,
                message: format!(
                    "planner estimated {est:.0} rows but saw {act:.0}; statistics are \
                     likely stale — ANALYZE the table"
                ),
                severity: Severity::Warn,
            });
        }
    }

    if node.node_class == NodeClass::Join && node.loops.unwrap_or(0.0) > LARGE_LOOPS {
        out.push(PlanWarning {
            node_id: node.id,
            kind: WarningKind::NestedLoopLarge,
            message: format!(
                "inner side executed {:.0} times; a hash or merge join would usually \
                 be cheaper at this size",
                node.loops.unwrap_or(0.0)
            ),
            severity: Severity::Info,
        });
    }

    for c in &node.children {
        walk(c, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rows(v: Value) -> Vec<serde_json::Map<String, Value>> {
        vec![[("x".to_string(), v)].into_iter().collect()]
    }

    #[test]
    fn classify_maps_the_common_node_names() {
        assert_eq!(classify("Seq Scan"), NodeClass::Scan);
        assert_eq!(classify("Index Only Scan"), NodeClass::Index);
        assert_eq!(classify("Hash Join"), NodeClass::Join);
        assert_eq!(classify("Sort"), NodeClass::Sort);
        assert_eq!(classify("HashAggregate"), NodeClass::Aggregate);
        assert_eq!(classify("Limit"), NodeClass::Limit);
        assert_eq!(classify("Gather Merge"), NodeClass::Other);
    }

    #[test]
    fn pg_plan_becomes_a_tree_with_costs() {
        let plan = json!([{
            "Plan": {
                "Node Type": "Hash Join", "Startup Cost": 1.5, "Total Cost": 42.0,
                "Plan Rows": 100, "Actual Rows": 90, "Plan Width": 32,
                "Actual Total Time": 3.5, "Actual Loops": 1,
                "Plans": [
                    {"Node Type": "Seq Scan", "Relation Name": "users",
                     "Total Cost": 20.0, "Plan Rows": 50},
                    {"Node Type": "Index Scan", "Relation Name": "orders",
                     "Total Cost": 10.0, "Plan Rows": 50}
                ]
            }
        }]);
        let (fmt, root) = parse_pg(&rows(plan));
        assert_eq!(fmt, PlanFormat::PgJson);
        let root = root.unwrap();
        assert_eq!(root.label, "Hash Join");
        assert_eq!(root.node_class, NodeClass::Join);
        assert_eq!(root.cost_total, Some(42.0));
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[0].relation.as_deref(), Some("users"));
        assert_eq!(root.children[0].detail, "Seq Scan on users");
        // Ids are unique across the tree so warnings can point at a node.
        assert_eq!(root.id, 0);
        assert_eq!(root.children[0].id, 1);
        assert_eq!(root.children[1].id, 2);
    }

    #[test]
    fn pg_plan_accepts_json_delivered_as_text() {
        let text = json!([{"Plan": {"Node Type": "Result"}}]).to_string();
        let (fmt, root) = parse_pg(&rows(Value::String(text)));
        assert_eq!(fmt, PlanFormat::PgJson);
        assert_eq!(root.unwrap().label, "Result");
    }

    #[test]
    fn sqlite_flat_rows_are_reassembled_into_a_tree() {
        let flat: Vec<serde_json::Map<String, Value>> = vec![
            json!({"id": 2, "parent": 0, "detail": "SCAN TABLE users"}),
            json!({"id": 4, "parent": 2, "detail": "SEARCH TABLE orders USING INDEX ix"}),
        ]
        .into_iter()
        .map(|v| v.as_object().unwrap().clone())
        .collect();

        let (fmt, root) = parse_sqlite(&flat);
        assert_eq!(fmt, PlanFormat::SqliteQueryPlan);
        let root = root.unwrap();
        assert_eq!(root.children.len(), 1, "top-level rows hang off the root");
        let scan = &root.children[0];
        assert_eq!(scan.node_class, NodeClass::Scan);
        assert_eq!(scan.relation.as_deref(), Some("users"));
        assert_eq!(scan.children.len(), 1, "child attaches to its parent id");
        assert_eq!(scan.children[0].node_class, NodeClass::Index);
    }

    #[test]
    fn sqlite_relation_survives_both_wordings() {
        // SQLite dropped the TABLE keyword in 3.36; both forms are in the wild.
        assert_eq!(sqlite_relation("SCAN TABLE users"), Some("users".into()));
        assert_eq!(sqlite_relation("SCAN people"), Some("people".into()));
        assert_eq!(
            sqlite_relation("SEARCH orders USING INDEX ix_o (user_id=?)"),
            Some("orders".into())
        );
        assert_eq!(
            sqlite_relation("SEARCH TABLE orders USING INDEX ix_o"),
            Some("orders".into())
        );
        assert_eq!(sqlite_relation("USE TEMP B-TREE FOR ORDER BY"), None);
    }

    #[test]
    fn unrecognised_output_reports_unknown_rather_than_guessing() {
        let (fmt, root) = parse_pg(&rows(json!("not a plan")));
        assert_eq!(fmt, PlanFormat::Unknown);
        assert!(root.is_none());
        assert_eq!(parse_sqlite(&[]).0, PlanFormat::Unknown);
    }

    fn node(id: u32, label: &str, est: f64, act: Option<f64>) -> PlanNode {
        PlanNode {
            id,
            parent: None,
            node_class: classify(label),
            label: label.into(),
            relation: Some("t".into()),
            cost_startup: None,
            cost_total: None,
            rows_estimated: Some(est),
            rows_actual: act,
            width: None,
            time_ms: None,
            loops: None,
            detail: label.into(),
            children: vec![],
        }
    }

    #[test]
    fn a_large_sequential_scan_is_flagged_but_a_small_one_is_not() {
        let big = warnings_for(&node(0, "Seq Scan", 50_000.0, None));
        assert_eq!(big.len(), 1);
        assert_eq!(big[0].kind, WarningKind::SeqScanLarge);

        let small = warnings_for(&node(0, "Seq Scan", 10.0, None));
        assert!(small.is_empty(), "a small scan is not a problem");

        // An index scan of the same size is fine.
        let indexed = warnings_for(&node(0, "Index Scan", 50_000.0, None));
        assert!(indexed.is_empty());
    }

    #[test]
    fn estimate_skew_fires_only_past_the_threshold() {
        let skewed = warnings_for(&node(0, "Index Scan", 10.0, Some(5_000.0)));
        assert!(skewed.iter().any(|w| w.kind == WarningKind::EstimateSkew));

        let close = warnings_for(&node(0, "Index Scan", 100.0, Some(150.0)));
        assert!(close.is_empty(), "a 1.5x difference is normal");
    }

    #[test]
    fn warnings_reach_nested_nodes() {
        let mut root = node(0, "Limit", 1.0, None);
        root.children.push(node(7, "Seq Scan", 90_000.0, None));
        let w = warnings_for(&root);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].node_id, 7, "the warning points at the offending node");
    }
}
