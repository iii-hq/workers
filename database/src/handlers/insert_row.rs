//! `database::insertRow` — insert one row from a `{column: value}` map, typed
//! by the catalog rather than by the caller.
//!
//! A form sends what a person typed: numbers as strings, booleans, JSON
//! documents, uuids, dates, bytes as base64. The worker reads the table's
//! columns (`describeTable`), quotes every identifier, and binds each value
//! so the server converts it to the column's type:
//!
//! * postgres — every value is bound as text and cast server-side,
//!   `$n::text::<column type>` (the type is `format_type` from the catalog),
//!   so the column type's own input function parses it; bytea from base64
//!   via `decode($n, 'base64')`; arrays from a JSON array.
//! * mysql — values bound as-is (the server converts strings for numeric,
//!   date and json columns); booleans as 1/0; blob/binary from base64 via
//!   `FROM_BASE64(?)`.
//! * sqlite — values bound by the column's affinity; booleans as 1/0;
//!   BLOB from base64 via `unhex(?)`.
//!
//! Columns left out take their default. The write goes through
//! `database::execute`, so it fires `database::row-changed` like any other.
//! The inserted row comes back via RETURNING (postgres, sqlite) or a
//! re-select by primary key / last insert id (mysql).

use base64::Engine as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::catalog::ColumnDesc;
use super::execute::{self, ExecuteReq};
use super::filter::{quote_ident, quote_table};
use super::query::{self, err_to_str, QueryReq};
use super::schema::{self, DescribeTableReq};
use super::AppState;
use crate::config::DriverKind;
use crate::error::DbError;
use crate::pool::Pool;

fn default_timeout() -> u64 {
    30_000
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct InsertRowReq {
    #[serde(default)]
    pub db: Option<String>,
    pub table: String,
    /// Postgres schema; ignored elsewhere.
    #[serde(default)]
    pub schema: Option<String>,
    /// Column → value. Strings are parsed by the column's type (numbers,
    /// booleans, uuids, dates, json); bytea/blob columns take base64. Columns
    /// left out take their default.
    #[serde(default)]
    pub values: Map<String, Value>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct InsertRowResp {
    /// The inserted row as stored (defaults and generated values filled
    /// in), or null when it cannot be read back (mysql without a usable key).
    pub row: Option<Map<String, Value>>,
    pub affected_rows: u64,
    pub elapsed_ms: u64,
}

fn invalid(reason: impl Into<String>) -> DbError {
    DbError::InvalidParam {
        index: 0,
        reason: reason.into(),
    }
}

/// A bound value and the SQL expression that receives it.
struct Bound {
    expr: String,
    value: Value,
}

fn is_blob(driver: DriverKind, ty: &str) -> bool {
    let t = ty.to_ascii_lowercase();
    match driver {
        DriverKind::Postgres => t == "bytea",
        DriverKind::Mysql => {
            t.contains("blob") || t.starts_with("binary") || t.starts_with("varbinary")
        }
        DriverKind::Sqlite => t.contains("blob"),
    }
}

fn is_bool(driver: DriverKind, ty: &str) -> bool {
    let t = ty.to_ascii_lowercase();
    match driver {
        DriverKind::Postgres => t == "boolean",
        DriverKind::Mysql => t == "tinyint(1)" || t == "bool" || t == "boolean",
        DriverKind::Sqlite => t.contains("bool"),
    }
}

/// A catalog type spelling safe to splice after `::` (format_type output:
/// letters, digits, spaces, `_ . " [ ] ( ) ,`).
fn safe_pg_type(ty: &str) -> Result<&str, DbError> {
    let ok = !ty.is_empty()
        && ty.len() <= 200
        && ty
            .chars()
            .all(|c| c.is_alphanumeric() || " _.\"[](),".contains(c))
        && !ty.contains("--");
    if ok {
        Ok(ty)
    } else {
        Err(invalid(format!(
            "column type {ty:?} cannot be used as a cast"
        )))
    }
}

/// The text a value is bound as on postgres (`None` for NULL).
fn pg_text(column: &str, ty: &str, value: &Value) -> Result<Option<String>, DbError> {
    Ok(match value {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Array(items) if ty.ends_with("[]") => Some(pg_array_literal(column, items)?),
        // json/jsonb columns (and anything else) take the JSON text.
        Value::Array(_) | Value::Object(_) => Some(value.to_string()),
    })
}

/// `{"a","b \" c",NULL}` from a JSON array (one level).
fn pg_array_literal(column: &str, items: &[Value]) -> Result<String, DbError> {
    let mut out = String::from("{");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        match item {
            Value::Null => out.push_str("NULL"),
            Value::Array(_) | Value::Object(_) => {
                return Err(invalid(format!(
                    "column `{column}`: nested arrays/objects are not supported in an array value"
                )))
            }
            other => {
                let text = match other {
                    Value::String(s) => s.clone(),
                    v => v.to_string(),
                };
                out.push('"');
                for ch in text.chars() {
                    if ch == '"' || ch == '\\' {
                        out.push('\\');
                    }
                    out.push(ch);
                }
                out.push('"');
            }
        }
    }
    out.push('}');
    Ok(out)
}

fn base64_bytes(column: &str, value: &Value) -> Result<Vec<u8>, DbError> {
    let Value::String(s) = value else {
        return Err(invalid(format!(
            "column `{column}` takes bytes as a base64 string"
        )));
    };
    let compact: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(compact.trim_end_matches('='))
        .or_else(|_| {
            base64::engine::general_purpose::STANDARD_NO_PAD.decode(compact.trim_end_matches('='))
        })
        .or_else(|_| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(compact.trim_end_matches('='))
        })
        .map_err(|_| {
            invalid(format!(
                "column `{column}` takes bytes as base64; this is not base64"
            ))
        })
}

fn truthy(column: &str, value: &Value) -> Result<Value, DbError> {
    Ok(match value {
        Value::Bool(b) => Value::from(i64::from(*b)),
        Value::Number(_) | Value::Null => value.clone(),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "t" | "yes" | "y" | "on" | "1" => Value::from(1),
            "false" | "f" | "no" | "n" | "off" | "0" => Value::from(0),
            _ => return Err(invalid(format!("column `{column}` takes a boolean"))),
        },
        _ => return Err(invalid(format!("column `{column}` takes a boolean"))),
    })
}

/// Bind one value for its column. `n` is the postgres placeholder number.
fn bind(driver: DriverKind, col: &ColumnDesc, value: &Value, n: usize) -> Result<Bound, DbError> {
    let blob = is_blob(driver, &col.ty);
    match driver {
        DriverKind::Postgres => {
            if blob && !value.is_null() {
                // Decoded here for a clear error, re-encoded canonically
                // (padded, standard alphabet) for postgres to decode.
                let bytes = base64_bytes(&col.name, value)?;
                return Ok(Bound {
                    expr: format!("decode(${n}::text, 'base64')"),
                    value: Value::String(base64::engine::general_purpose::STANDARD.encode(bytes)),
                });
            }
            let ty = safe_pg_type(&col.ty)?;
            Ok(Bound {
                expr: format!("${n}::text::{ty}"),
                value: pg_text(&col.name, &col.ty, value)?.map_or(Value::Null, Value::String),
            })
        }
        DriverKind::Mysql => {
            if blob && !value.is_null() {
                // FROM_BASE64 answers NULL to anything it cannot read: decode
                // here for a clear error and hand it the canonical form.
                let bytes = base64_bytes(&col.name, value)?;
                return Ok(Bound {
                    expr: "FROM_BASE64(?)".into(),
                    value: Value::String(base64::engine::general_purpose::STANDARD.encode(bytes)),
                });
            }
            let value = match value {
                _ if is_bool(driver, &col.ty) => truthy(&col.name, value)?,
                Value::Bool(b) => Value::from(i64::from(*b)),
                Value::Array(_) | Value::Object(_) => Value::String(value.to_string()),
                other => other.clone(),
            };
            Ok(Bound {
                expr: "?".into(),
                value,
            })
        }
        DriverKind::Sqlite => {
            if blob && !value.is_null() {
                let bytes = base64_bytes(&col.name, value)?;
                let hex: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
                return Ok(Bound {
                    expr: "unhex(?)".into(),
                    value: Value::String(hex),
                });
            }
            let value = match value {
                _ if is_bool(driver, &col.ty) => truthy(&col.name, value)?,
                Value::Bool(b) => Value::from(i64::from(*b)),
                Value::Array(_) | Value::Object(_) => Value::String(value.to_string()),
                other => other.clone(),
            };
            Ok(Bound {
                expr: "?".into(),
                value,
            })
        }
    }
}

/// The INSERT statement and its parameters, in the request's column order.
pub(crate) fn build_insert(
    driver: DriverKind,
    schema: Option<&str>,
    table: &str,
    columns: &[ColumnDesc],
    values: &Map<String, Value>,
) -> Result<(String, Vec<Value>), DbError> {
    let target = quote_table(driver, schema, table);
    let mut names = Vec::new();
    let mut exprs = Vec::new();
    let mut params = Vec::new();
    for (name, value) in values {
        let col = columns
            .iter()
            .find(|c| &c.name == name)
            .ok_or_else(|| invalid(format!("`{table}` has no column `{name}`")))?;
        let bound = bind(driver, col, value, params.len() + 1)?;
        names.push(quote_ident(driver, &col.name));
        exprs.push(bound.expr);
        params.push(bound.value);
    }
    let returning = match driver {
        DriverKind::Mysql => "",
        _ => " RETURNING *",
    };
    let sql = if names.is_empty() {
        match driver {
            DriverKind::Mysql => format!("INSERT INTO {target} () VALUES ()"),
            _ => format!("INSERT INTO {target} DEFAULT VALUES{returning}"),
        }
    } else {
        format!(
            "INSERT INTO {target} ({}) VALUES ({}){returning}",
            names.join(", "),
            exprs.join(", ")
        )
    };
    Ok((sql, params))
}

pub async fn handle(state: &AppState, req: InsertRowReq) -> Result<InsertRowResp, String> {
    if req.table.trim().is_empty() {
        return Err(err_to_str(invalid("table is required")));
    }
    let db = state.resolve_db(req.db.clone()).await.map_err(err_to_str)?;
    let driver = match state.pool(&db).await.map_err(err_to_str)? {
        Pool::Sqlite(_) => DriverKind::Sqlite,
        Pool::Postgres(_) => DriverKind::Postgres,
        Pool::Mysql(_) => DriverKind::Mysql,
    };
    let description = schema::describe_table(
        state,
        DescribeTableReq {
            db: Some(db.clone()),
            table: req.table.clone(),
            schema: req.schema.clone(),
            timeout_ms: req.timeout_ms,
        },
    )
    .await?;
    if description.kind == super::catalog::TableKind::View {
        return Err(err_to_str(invalid(format!(
            "`{}` is a view; insert into its table",
            req.table
        ))));
    }
    let schema_name = description.schema.clone().or(req.schema.clone());
    let (sql, params) = build_insert(
        driver,
        schema_name.as_deref(),
        &description.table,
        &description.columns,
        &req.values,
    )
    .map_err(err_to_str)?;

    let result = execute::handle(
        state,
        ExecuteReq {
            db: Some(db.clone()),
            sql,
            params,
            returning: Vec::new(),
        },
    )
    .await?;
    let mut elapsed_ms = result.elapsed_ms;

    let row = match driver {
        DriverKind::Postgres | DriverKind::Sqlite => result.returned_rows.into_iter().next(),
        DriverKind::Mysql => {
            // No RETURNING: read the row back by its key — the auto-increment
            // id the insert produced, or a primary key the request supplied.
            let pk: Vec<&ColumnDesc> = description
                .columns
                .iter()
                .filter(|c| c.primary_key)
                .collect();
            let key: Option<Vec<(String, Value)>> = match (pk.as_slice(), &result.last_insert_id) {
                ([only], Some(id)) if !req.values.contains_key(&only.name) && id != "0" => {
                    Some(vec![(only.name.clone(), Value::String(id.clone()))])
                }
                (cols, _) if !cols.is_empty() => cols
                    .iter()
                    .map(|c| req.values.get(&c.name).map(|v| (c.name.clone(), v.clone())))
                    .collect(),
                _ => None,
            };
            match key {
                Some(key) if result.affected_rows > 0 => {
                    let target = quote_table(driver, None, &description.table);
                    let predicate: Vec<String> = key
                        .iter()
                        .map(|(c, _)| format!("{} = ?", quote_ident(driver, c)))
                        .collect();
                    let params: Vec<Value> = key
                        .into_iter()
                        .map(|(_, v)| match v {
                            Value::Bool(b) => Value::from(i64::from(b)),
                            other => other,
                        })
                        .collect();
                    let read = query::handle(
                        state,
                        QueryReq {
                            db: Some(db),
                            sql: format!(
                                "SELECT * FROM {target} WHERE {} LIMIT 1",
                                predicate.join(" AND ")
                            ),
                            params,
                            timeout_ms: req.timeout_ms,
                            record_history: false,
                            max_rows: None,
                            query_only: false,
                        },
                    )
                    .await;
                    match read {
                        Ok(resp) => {
                            elapsed_ms += resp.elapsed_ms;
                            resp.rows.into_iter().next()
                        }
                        Err(_) => None,
                    }
                }
                _ => None,
            }
        }
    };
    Ok(InsertRowResp {
        row,
        affected_rows: result.affected_rows,
        elapsed_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn col(name: &str, ty: &str, pk: bool) -> ColumnDesc {
        ColumnDesc {
            name: name.into(),
            ty: ty.into(),
            nullable: true,
            default_value: None,
            primary_key: pk,
            position: 1,
            foreign_key: None,
        }
    }

    fn values(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn postgres_casts_every_value_through_text_to_the_catalog_type() {
        let cols = [
            col("id", "uuid", true),
            col("price", "numeric(10,2)", false),
            col("active", "boolean", false),
            col("meta", "jsonb", false),
            col("tags", "text[]", false),
            col("blob", "bytea", false),
            col("at", "timestamp with time zone", false),
            col("note", "text", false),
        ];
        let (sql, params) = build_insert(
            DriverKind::Postgres,
            Some("public"),
            "items",
            &cols,
            &values(json!({
                "id": "6ac4e3f0-0000-4000-8000-000000000001",
                "price": "12.50",
                "active": true,
                "meta": {"a": 1},
                "tags": ["x", "y \"z\""],
                "blob": "aGVsbG8=",
                "at": "2026-10-18T10:00:00Z",
                "note": null
            })),
        )
        .unwrap();
        assert!(
            sql.starts_with(r#"INSERT INTO "public"."items" ("#),
            "{sql}"
        );
        assert!(sql.contains("::text::uuid"), "{sql}");
        assert!(sql.contains("::text::numeric(10,2)"), "{sql}");
        assert!(sql.contains("::text::boolean"), "{sql}");
        assert!(sql.contains("::text::jsonb"), "{sql}");
        assert!(sql.contains("::text::text[]"), "{sql}");
        assert!(sql.contains("decode($"), "{sql}");
        assert!(sql.contains("::text::timestamp with time zone"), "{sql}");
        assert!(sql.ends_with(" RETURNING *"), "{sql}");
        assert!(params.contains(&json!("12.50")));
        assert!(params.contains(&json!("true")));
        assert!(params.contains(&json!(r#"{"a":1}"#)));
        assert!(params.contains(&json!(r#"{"x","y \"z\""}"#)));
        assert!(params.contains(&json!("aGVsbG8=")));
        assert!(params.contains(&Value::Null));
    }

    #[test]
    fn mysql_and_sqlite_bind_booleans_as_integers_and_blobs_from_base64() {
        let cols = [
            col("id", "int", true),
            col("flag", "tinyint(1)", false),
            col("doc", "json", false),
            col("data", "blob", false),
        ];
        let (sql, params) = build_insert(
            DriverKind::Mysql,
            None,
            "t",
            &cols,
            &values(json!({"flag": "true", "doc": {"k": [1]}, "data": "AAEC"})),
        )
        .unwrap();
        // Columns in key order (serde_json's map is sorted).
        assert_eq!(
            sql,
            "INSERT INTO `t` (`data`, `doc`, `flag`) VALUES (FROM_BASE64(?), ?, ?)"
        );
        assert_eq!(params, vec![json!("AAEC"), json!(r#"{"k":[1]}"#), json!(1)]);

        let cols = [
            col("ok", "BOOLEAN", false),
            col("b", "BLOB", false),
            col("n", "INTEGER", false),
        ];
        let (sql, params) = build_insert(
            DriverKind::Sqlite,
            None,
            "t",
            &cols,
            &values(json!({"ok": false, "b": "AAEC", "n": "42"})),
        )
        .unwrap();
        assert_eq!(
            sql,
            r#"INSERT INTO "t" ("b", "n", "ok") VALUES (unhex(?), ?, ?) RETURNING *"#
        );
        assert_eq!(params, vec![json!("000102"), json!("42"), json!(0)]);
    }

    #[test]
    fn empty_values_insert_defaults_and_unknown_columns_are_refused() {
        let (sql, _) = build_insert(DriverKind::Postgres, None, "t", &[], &Map::new()).unwrap();
        assert_eq!(sql, r#"INSERT INTO "t" DEFAULT VALUES RETURNING *"#);
        let (sql, _) = build_insert(DriverKind::Mysql, None, "t", &[], &Map::new()).unwrap();
        assert_eq!(sql, "INSERT INTO `t` () VALUES ()");
        let err = build_insert(
            DriverKind::Sqlite,
            None,
            "t",
            &[col("a", "TEXT", false)],
            &values(json!({"b": 1})),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("no column `b`"));
    }

    #[test]
    fn bad_input_is_refused_with_the_column_named() {
        let cols = [
            col("data", "bytea", false),
            col("flag", "tinyint(1)", false),
        ];
        let err = build_insert(
            DriverKind::Postgres,
            None,
            "t",
            &cols,
            &values(json!({"data": "not base64!!"})),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("`data`"), "{err}");
        let err = build_insert(
            DriverKind::Mysql,
            None,
            "t",
            &cols,
            &values(json!({"flag": "maybe"})),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("`flag` takes a boolean"), "{err}");
        // A catalog type that is not a plain type spelling never reaches SQL.
        let evil = [col("x", "int; DROP TABLE t", false)];
        assert!(build_insert(
            DriverKind::Postgres,
            None,
            "t",
            &evil,
            &values(json!({"x": 1}))
        )
        .is_err());
    }
}
