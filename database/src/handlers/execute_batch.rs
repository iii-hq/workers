//! `database::execute_batch` — atomic batch of statements.
//!
//! Thin convenience form of `database::transaction`: each statement may be a
//! bare SQL string or a full `{ sql, params }` object. Delegates to
//! `transaction::handle`, so semantics (atomicity, isolation, guards,
//! response envelope) are identical. Also registered under the camelCase
//! alias `database::executeBatch`.

use super::AppState;
use crate::handlers::transaction::{self, TxReq, TxResp, TxStmtReq};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, JsonSchema)]
pub struct ExecuteBatchReq {
    pub db: String,
    /// Statements to run in order inside one transaction. Each entry is
    /// either a bare SQL string or `{ "sql": "...", "params": [...] }` —
    /// use `params` for dynamic values instead of inlining them into the SQL.
    #[serde(deserialize_with = "lenient_statements")]
    pub statements: Vec<BatchStatement>,
    /// Optional: `read_committed` | `repeatable_read` | `serializable`.
    #[serde(default)]
    pub isolation: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum BatchStatement {
    Sql(String),
    Full(TxStmtReq),
}

/// Like `handlers::lenient_params`: engine/LLM callers sometimes send the
/// array JSON-encoded as a string; absorb that instead of failing serde.
fn lenient_statements<'de, D>(de: D) -> Result<Vec<BatchStatement>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;
    use serde::Deserialize as _;

    let v = match Value::deserialize(de)? {
        Value::String(s) => serde_json::from_str(&s)
            .map_err(|e| D::Error::custom(format!("statements string is not a JSON array: {e}")))?,
        other => other,
    };
    serde_json::from_value(v).map_err(D::Error::custom)
}

pub async fn handle(state: &AppState, req: ExecuteBatchReq) -> Result<TxResp, String> {
    let statements = req
        .statements
        .into_iter()
        .map(|s| match s {
            BatchStatement::Sql(sql) => TxStmtReq {
                sql,
                params: Vec::new(),
            },
            BatchStatement::Full(stmt) => stmt,
        })
        .collect();
    transaction::handle(
        state,
        TxReq {
            db: req.db,
            statements,
            isolation: req.isolation,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::begin_transaction::tests::state;
    use serde_json::json;

    fn batch_req(v: Value) -> ExecuteBatchReq {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn request_accepts_stringified_statements_array() {
        let req = batch_req(json!({
            "db": "primary",
            "statements": "[\"INSERT INTO t VALUES (1)\"]"
        }));
        assert_eq!(req.statements.len(), 1);
        assert!(matches!(&req.statements[0], BatchStatement::Sql(s) if s.contains("INSERT")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn commits_bare_string_statements() {
        let st = state();
        crate::handlers::execute::handle(
            &st,
            serde_json::from_value(json!({"db": "primary", "sql": "CREATE TABLE t (n INT)"}))
                .unwrap(),
        )
        .await
        .unwrap();

        let resp = handle(
            &st,
            batch_req(json!({
                "db": "primary",
                "statements": ["INSERT INTO t VALUES (1)", "INSERT INTO t VALUES (2)"]
            })),
        )
        .await
        .unwrap();

        assert!(resp.committed);
        let results = resp.results.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].affected_rows, 1);
        assert!(resp.failed_index.is_none());
        assert!(resp.error.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mixed_forms_support_bound_params() {
        let st = state();
        crate::handlers::execute::handle(
            &st,
            serde_json::from_value(json!({"db": "primary", "sql": "CREATE TABLE t (s TEXT)"}))
                .unwrap(),
        )
        .await
        .unwrap();

        // The bound param carries a semicolon — impossible to inline on
        // sqlite (multi-statement guard), fine as a param.
        let resp = handle(
            &st,
            batch_req(json!({
                "db": "primary",
                "statements": [
                    {"sql": "INSERT INTO t VALUES (?)", "params": ["hi; bye"]},
                    "INSERT INTO t VALUES ('plain')",
                ]
            })),
        )
        .await
        .unwrap();
        assert!(resp.committed);

        let rows = crate::handlers::query::handle(
            &st,
            serde_json::from_value(json!({"db": "primary", "sql": "SELECT s FROM t ORDER BY s"}))
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(rows.rows[0]["s"], json!("hi; bye"));
        assert_eq!(rows.rows[1]["s"], json!("plain"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rolls_back_and_reports_failed_index_on_error() {
        let st = state();
        crate::handlers::execute::handle(
            &st,
            serde_json::from_value(json!({
                "db": "primary",
                "sql": "CREATE TABLE t (n INT NOT NULL)"
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let resp = handle(
            &st,
            batch_req(json!({
                "db": "primary",
                "statements": ["INSERT INTO t VALUES (1)", "INSERT INTO t VALUES (NULL)"]
            })),
        )
        .await
        .unwrap();

        assert!(!resp.committed);
        assert_eq!(resp.failed_index, Some(1));
        assert!(resp.error.is_some());
        assert!(resp.results.is_none());

        let rows = crate::handlers::query::handle(
            &st,
            serde_json::from_value(json!({"db": "primary", "sql": "SELECT COUNT(*) AS c FROM t"}))
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(rows.rows[0]["c"], json!(0));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rejects_embedded_transaction_control_sql() {
        let st = state();
        let err = handle(
            &st,
            batch_req(json!({
                "db": "primary",
                "statements": ["SELECT 1", "COMMIT"]
            })),
        )
        .await
        .unwrap_err();
        assert!(err.contains("INVALID_PARAM"), "got: {err}");
        assert!(err.contains("transaction-control"), "got: {err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rejects_empty_statement() {
        let st = state();
        let err = handle(&st, batch_req(json!({"db": "primary", "statements": [""]})))
            .await
            .unwrap_err();
        assert!(err.contains("INVALID_PARAM"), "got: {err}");
        assert!(err.contains("empty SQL"), "got: {err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn isolation_field_is_honored_not_dropped() {
        let st = state();
        let err = handle(
            &st,
            batch_req(json!({
                "db": "primary",
                "statements": ["SELECT 1"],
                "isolation": "banana"
            })),
        )
        .await
        .unwrap_err();
        assert!(err.contains("unknown isolation"), "got: {err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn empty_statements_commits_as_no_op() {
        let st = state();
        let resp = handle(&st, batch_req(json!({"db": "primary", "statements": []})))
            .await
            .unwrap();
        assert!(resp.committed);
        assert_eq!(resp.results.unwrap().len(), 0);
    }
}
