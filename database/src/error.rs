//! Discriminated error codes returned to the engine.
//!
//! The `code` field is stable; clients should match on it. The remaining
//! fields are diagnostic.

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "code")]
pub enum DbError {
    #[serde(rename = "POOL_TIMEOUT")]
    #[error("pool acquire timed out for db {db} after {waited_ms}ms")]
    PoolTimeout { db: String, waited_ms: u64 },

    #[serde(rename = "QUERY_TIMEOUT")]
    #[error("query exceeded timeout {timeout_ms}ms on db {db}")]
    QueryTimeout { db: String, timeout_ms: u64 },

    #[serde(rename = "STATEMENT_NOT_FOUND")]
    #[error("statement handle {handle_id} not found or expired")]
    StatementNotFound { handle_id: String },

    #[serde(rename = "TRANSACTION_NOT_FOUND")]
    #[error("transaction {transaction_id} not found, ended, or timed out")]
    TransactionNotFound { transaction_id: String },

    #[serde(rename = "UNKNOWN_DB")]
    #[error("unknown db {db}; available: [{}]", available.join(", "))]
    UnknownDb { db: String, available: Vec<String> },

    #[serde(rename = "MISSING_DB")]
    #[error("no `db` specified and none of the configured databases is an unambiguous default; available: [{}]", available.join(", "))]
    MissingDb { available: Vec<String> },

    #[serde(rename = "INVALID_PARAM")]
    #[error("invalid parameter at index {index}: {reason}")]
    InvalidParam { index: usize, reason: String },

    #[serde(rename = "DRIVER_ERROR")]
    #[error("driver {driver} error: {message}")]
    DriverError {
        driver: String,
        #[serde(rename = "inner_code")]
        code: Option<String>,
        message: String,
        /// Set when this error occurred during a multi-statement transaction.
        /// The 0-based index of the statement that failed.
        #[serde(skip_serializing_if = "Option::is_none")]
        failed_index: Option<usize>,
    },

    #[serde(rename = "REPLICATION_SLOT_EXISTS")]
    #[error("replication slot {slot} already in use")]
    ReplicationSlotExists { slot: String },

    #[serde(rename = "UNSUPPORTED")]
    #[error("operation {op} not supported on driver {driver}")]
    Unsupported { op: String, driver: String },

    #[serde(rename = "CONFIG_ERROR")]
    #[error("config error: {message}")]
    ConfigError { message: String },
}

impl From<DbError> for iii_sdk::errors::Error {
    fn from(e: DbError) -> Self {
        let body = serde_json::to_string(&e)
            .expect("DbError serialization is infallible (only primitive fields)");
        iii_sdk::errors::Error::Handler(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_timeout_serializes_with_stable_code() {
        let e = DbError::PoolTimeout {
            db: "primary".into(),
            waited_ms: 5000,
        };
        let v: serde_json::Value = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "POOL_TIMEOUT");
        assert_eq!(v["db"], "primary");
        assert_eq!(v["waited_ms"], 5000);
    }

    #[test]
    fn unknown_db_serializes_with_stable_code_and_available_handles() {
        let e = DbError::UnknownDb {
            db: "missing".into(),
            available: vec!["analytics".into(), "primary".into()],
        };
        let v: serde_json::Value = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "UNKNOWN_DB");
        assert_eq!(v["db"], "missing");
        assert_eq!(
            v["available"],
            serde_json::json!(["analytics", "primary"]),
            "available handles must be in the wire envelope so callers can self-correct"
        );
        // The human-readable message names the handles too — that's what an
        // LLM caller sees in a function_result.
        assert_eq!(
            e.to_string(),
            "unknown db missing; available: [analytics, primary]"
        );
    }

    #[test]
    fn missing_db_serializes_with_stable_code_and_available_handles() {
        let e = DbError::MissingDb {
            available: vec!["analytics".into(), "main".into()],
        };
        let v: serde_json::Value = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "MISSING_DB");
        assert_eq!(v["available"], serde_json::json!(["analytics", "main"]));
        assert_eq!(
            e.to_string(),
            "no `db` specified and none of the configured databases is an unambiguous default; available: [analytics, main]"
        );
    }

    #[test]
    fn transaction_not_found_serializes_with_stable_code() {
        let e = DbError::TransactionNotFound {
            transaction_id: "tx-123".into(),
        };
        let v: serde_json::Value = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "TRANSACTION_NOT_FOUND");
        assert_eq!(v["transaction_id"], "tx-123");
    }

    #[test]
    fn driver_error_carries_driver_name_and_inner() {
        let e = DbError::DriverError {
            driver: "postgres".into(),
            code: Some("42P01".into()),
            message: "relation \"x\" does not exist".into(),
            failed_index: None,
        };
        let v: serde_json::Value = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "DRIVER_ERROR");
        assert_eq!(v["driver"], "postgres");
        assert_eq!(v["inner_code"], "42P01");
        assert!(v.get("failed_index").is_none());
    }

    #[test]
    fn driver_error_serializes_failed_index_when_set() {
        let e = DbError::DriverError {
            driver: "sqlite".into(),
            code: None,
            message: "constraint failed".into(),
            failed_index: Some(2),
        };
        let v: serde_json::Value = serde_json::to_value(&e).unwrap();
        assert_eq!(v["failed_index"], 2);
    }

    #[test]
    fn into_iii_error_preserves_json_body() {
        let e = DbError::QueryTimeout {
            db: "primary".into(),
            timeout_ms: 30000,
        };
        let iii_e: iii_sdk::errors::Error = e.into();
        let body = format!("{iii_e:?}");
        assert!(body.contains("QUERY_TIMEOUT"));
    }
}
