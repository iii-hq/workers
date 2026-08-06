//! `database::columnStats` — profile a column without reading the table.
//!
//! Two modes, and the default matters. A naive profile runs
//! `COUNT(DISTINCT col)`, `MIN`, `MAX` and a `GROUP BY`, each of which is a
//! full scan; doing that from a console panel is how a read-only viewer
//! causes a production incident. So the default reads the statistics the
//! planner already maintains (`pg_stats`, `information_schema.STATISTICS`,
//! `sqlite_stat1`) — O(1) catalog reads that cost nothing — and `exact: true`
//! is an explicit opt-in that runs the real aggregates behind a timeout.
//!
//! Approximate numbers are always labelled `source: planner`, never presented
//! as if they were counted.
//!
//! Scope note: this profiles the *whole table* server-side. To profile rows
//! you already hold, pipe a `browseTable` result through the `fp` worker
//! instead — that is what it is for, and duplicating it here would be worse
//! on both counts.

use super::filter::{quote_ident, quote_table};
use super::query::{self, err_to_str, QueryReq};
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

fn default_top_n() -> usize {
    10
}

/// Cap on `top_n` — a "most common values" list longer than this is a report,
/// not a profile.
const MAX_TOP_N: usize = 100;

/// Above this planner-estimated row count, `exact: true` is refused rather
/// than silently scanning a very large table.
const EXACT_ROW_CEILING: i64 = 5_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StatSource {
    /// Read from the planner's own statistics. Approximate, and free.
    Planner,
    /// Counted by running aggregates over the table.
    Computed,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TopValue {
    pub value: Value,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ColumnStat {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distinct_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub null_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub null_fraction: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mean: Option<f64>,
    /// Only populated in `exact` mode; the planner's own most-common-value
    /// lists are not portable enough to report faithfully.
    pub top_values: Vec<TopValue>,
    pub source: StatSource,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ColumnStatsReq {
    #[serde(default)]
    pub db: Option<String>,
    pub table: String,
    #[serde(default)]
    pub schema: Option<String>,
    /// Omit to profile every column.
    #[serde(default)]
    pub columns: Option<Vec<String>>,
    /// Run real aggregates instead of reading planner statistics. This scans
    /// the table.
    #[serde(default)]
    pub exact: bool,
    #[serde(default = "default_top_n")]
    pub top_n: usize,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ColumnStatsResp {
    pub table: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub columns: Vec<ColumnStat>,
    /// True when the numbers came from the planner rather than a count.
    pub approximate: bool,
}

async fn driver_of(state: &AppState, db: &str) -> Result<DriverKind, String> {
    Ok(match state.pool(db).await.map_err(err_to_str)? {
        Pool::Sqlite(_) => DriverKind::Sqlite,
        Pool::Postgres(_) => DriverKind::Postgres,
        Pool::Mysql(_) => DriverKind::Mysql,
    })
}

async fn run(
    state: &AppState,
    db: &str,
    sql: String,
    params: Vec<Value>,
    timeout_ms: u64,
) -> Result<Vec<serde_json::Map<String, Value>>, String> {
    Ok(query::handle(
        state,
        QueryReq {
            db: Some(db.to_string()),
            sql,
            params,
            timeout_ms,
            record_history: false,
        },
    )
    .await?
    .rows)
}

fn f64_at(row: &serde_json::Map<String, Value>, key: &str) -> Option<f64> {
    match row.get(key) {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.parse().ok(),
        _ => None,
    }
}

fn i64_at(row: &serde_json::Map<String, Value>, key: &str) -> Option<i64> {
    f64_at(row, key).map(|f| f as i64)
}

pub async fn handle(state: &AppState, req: ColumnStatsReq) -> Result<ColumnStatsResp, String> {
    if req.table.trim().is_empty() {
        return Err(err_to_str(DbError::InvalidParam {
            index: 0,
            reason: "table is required".into(),
        }));
    }
    let db = state.resolve_db(req.db.clone()).await.map_err(err_to_str)?;
    let driver = driver_of(state, &db).await?;
    let top_n = req.top_n.clamp(1, MAX_TOP_N);

    // Resolve the column list from the catalog rather than trusting the
    // caller, so an unknown name fails here instead of inside an aggregate.
    let described = super::schema::describe_table(
        state,
        super::schema::DescribeTableReq {
            db: Some(db.clone()),
            table: req.table.clone(),
            schema: req.schema.clone(),
            timeout_ms: req.timeout_ms,
        },
    )
    .await?;

    let wanted: Vec<String> = match &req.columns {
        Some(list) => {
            for c in list {
                if !described.columns.iter().any(|d| &d.name == c) {
                    return Err(err_to_str(DbError::InvalidParam {
                        index: 0,
                        reason: format!("no such column `{c}` on `{}`", req.table),
                    }));
                }
            }
            list.clone()
        }
        None => described.columns.iter().map(|c| c.name.clone()).collect(),
    };

    let target = quote_table(driver, described.schema.as_deref(), &described.table);

    if !req.exact {
        let columns =
            planner_stats(state, &db, driver, &described, &wanted, req.timeout_ms).await?;
        return Ok(ColumnStatsResp {
            table: described.table,
            schema: described.schema,
            columns,
            approximate: true,
        });
    }

    // Refuse an exact profile of a very large table rather than starting a
    // scan the caller cannot cancel — on sqlite the driver drops `timeout_ms`
    // entirely, so a row-count guard is the only brake available.
    if let Some(est) = described.row_count_estimate {
        if est > EXACT_ROW_CEILING {
            return Err(err_to_str(DbError::InvalidParam {
                index: 0,
                reason: format!(
                    "`{}` has roughly {est} rows; an exact profile would scan all of \
                     them. Re-run without `exact` for planner statistics, or profile \
                     a filtered subset.",
                    described.table
                ),
            }));
        }
    }

    let mut columns = Vec::new();
    for name in &wanted {
        columns.push(exact_stats(state, &db, driver, &target, name, top_n, req.timeout_ms).await?);
    }
    Ok(ColumnStatsResp {
        table: described.table,
        schema: described.schema,
        columns,
        approximate: false,
    })
}

/// Read what the planner already knows. Cheap, approximate, and clearly
/// labelled as such.
async fn planner_stats(
    state: &AppState,
    db: &str,
    driver: DriverKind,
    described: &super::schema::TableDescription,
    wanted: &[String],
    timeout_ms: u64,
) -> Result<Vec<ColumnStat>, String> {
    let row_count = described.row_count_estimate;
    let mut out: Vec<ColumnStat> = wanted
        .iter()
        .map(|name| ColumnStat {
            name: name.clone(),
            row_count,
            distinct_count: None,
            null_count: None,
            null_fraction: None,
            min: None,
            max: None,
            mean: None,
            top_values: Vec::new(),
            source: StatSource::Planner,
        })
        .collect();

    match driver {
        DriverKind::Postgres => {
            // `pg_stats` exposes null_frac and n_distinct as plain scalars.
            // n_distinct is negative when it is a ratio of the row count.
            let rows = run(
                state,
                db,
                "SELECT attname AS column_name, null_frac, n_distinct \
                 FROM pg_stats WHERE schemaname = COALESCE($1, 'public') AND tablename = $2"
                    .into(),
                vec![
                    described
                        .schema
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                    Value::String(described.table.clone()),
                ],
                timeout_ms,
            )
            .await?;
            for r in &rows {
                let Some(col) = r.get("column_name").and_then(Value::as_str) else {
                    continue;
                };
                let Some(stat) = out.iter_mut().find(|s| s.name == col) else {
                    continue;
                };
                stat.null_fraction = f64_at(r, "null_frac");
                if let (Some(frac), Some(total)) = (stat.null_fraction, row_count) {
                    stat.null_count = Some((frac * total as f64).round() as i64);
                }
                stat.distinct_count = f64_at(r, "n_distinct").map(|n| {
                    if n < 0.0 {
                        // Negative means "this fraction of the row count".
                        (-n * row_count.unwrap_or(0) as f64).round() as i64
                    } else {
                        n as i64
                    }
                });
            }
        }
        DriverKind::Mysql => {
            // CARDINALITY is per leading index column, so it only answers for
            // indexed columns — which is honest: unindexed columns get None.
            let rows = run(
                state,
                db,
                "SELECT COLUMN_NAME AS column_name, MAX(CARDINALITY) AS cardinality \
                 FROM information_schema.STATISTICS \
                 WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ? AND SEQ_IN_INDEX = 1 \
                 GROUP BY COLUMN_NAME"
                    .into(),
                vec![Value::String(described.table.clone())],
                timeout_ms,
            )
            .await?;
            for r in &rows {
                let Some(col) = r.get("column_name").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(stat) = out.iter_mut().find(|s| s.name == col) {
                    stat.distinct_count = i64_at(r, "cardinality");
                }
            }
        }
        DriverKind::Sqlite => {
            // `sqlite_stat1` exists only after an explicit ANALYZE. Its `stat`
            // column is "<rows> <avg-rows-per-distinct> ..." per index column.
            let rows = run(
                state,
                db,
                "SELECT tbl, idx, stat FROM sqlite_stat1 WHERE tbl = ?".into(),
                vec![Value::String(described.table.clone())],
                timeout_ms,
            )
            .await
            // The table does not exist until someone runs ANALYZE; that is a
            // missing answer, not an error.
            .unwrap_or_default();
            if let Some(total) = rows
                .first()
                .and_then(|r| r.get("stat"))
                .and_then(Value::as_str)
                .and_then(|s| s.split_whitespace().next())
                .and_then(|s| s.parse::<i64>().ok())
            {
                for stat in out.iter_mut() {
                    stat.row_count = Some(total);
                }
            }
        }
    }
    Ok(out)
}

/// Real aggregates. One pass for the scalars, one for the top values.
async fn exact_stats(
    state: &AppState,
    db: &str,
    driver: DriverKind,
    target: &str,
    column: &str,
    top_n: usize,
    timeout_ms: u64,
) -> Result<ColumnStat, String> {
    let col = quote_ident(driver, column);
    let scalars = run(
        state,
        db,
        format!(
            "SELECT COUNT(*) AS row_count, COUNT({col}) AS non_null, \
                    COUNT(DISTINCT {col}) AS distinct_count, \
                    MIN({col}) AS min_value, MAX({col}) AS max_value, \
                    AVG(CASE WHEN {col} + 0 = {col} THEN {col} END) AS mean_value \
             FROM {target}"
        ),
        vec![],
        timeout_ms,
    )
    .await?;

    let row = scalars.first().cloned().unwrap_or_default();
    let row_count = i64_at(&row, "row_count");
    let non_null = i64_at(&row, "non_null");
    let null_count = match (row_count, non_null) {
        (Some(t), Some(n)) => Some(t - n),
        _ => None,
    };

    let tops = run(
        state,
        db,
        format!(
            "SELECT {col} AS value, COUNT(*) AS n FROM {target} \
             WHERE {col} IS NOT NULL GROUP BY {col} ORDER BY n DESC, 1 LIMIT {top_n}"
        ),
        vec![],
        timeout_ms,
    )
    .await?;

    Ok(ColumnStat {
        name: column.to_string(),
        row_count,
        distinct_count: i64_at(&row, "distinct_count"),
        null_count,
        null_fraction: match (null_count, row_count) {
            (Some(n), Some(t)) if t > 0 => Some(n as f64 / t as f64),
            _ => None,
        },
        min: row.get("min_value").cloned().filter(|v| !v.is_null()),
        max: row.get("max_value").cloned().filter(|v| !v.is_null()),
        mean: f64_at(&row, "mean_value"),
        top_values: tops
            .iter()
            .filter_map(|r| {
                Some(TopValue {
                    value: r.get("value")?.clone(),
                    count: i64_at(r, "n")?,
                })
            })
            .collect(),
        source: StatSource::Computed,
    })
}
