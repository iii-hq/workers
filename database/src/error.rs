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

    #[serde(rename = "UNKNOWN_DB")]
    #[error("unknown db {db}")]
    UnknownDb { db: String },

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

impl From<DbError> for iii_sdk::IIIError {
    fn from(e: DbError) -> Self {
        let body = serde_json::to_string(&e)
            .expect("DbError serialization is infallible (only primitive fields)");
        iii_sdk::IIIError::Handler(body)
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
    fn unknown_db_serializes_with_stable_code() {
        let e = DbError::UnknownDb {
            db: "missing".into(),
        };
        let v: serde_json::Value = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "UNKNOWN_DB");
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
        // None should not appear in JSON.
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
        let iii_e: iii_sdk::IIIError = e.into();
        let body = format!("{iii_e:?}");
        assert!(body.contains("QUERY_TIMEOUT"));
    }
}
