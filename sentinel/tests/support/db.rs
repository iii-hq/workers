//! The store a test runs on: an in-memory SQLite, or a schema of its own on
//! the Postgres `SENTINEL_TEST_POSTGRES_URL` names, so the whole suite can be
//! pointed at the server a shared team store would run on:
//!
//! ```text
//! docker run -d --rm -p 127.0.0.1:5439:5432 -e POSTGRES_PASSWORD=pg postgres:17-alpine
//! SENTINEL_TEST_POSTGRES_URL=postgres://postgres:pg@127.0.0.1:5439/postgres cargo test
//! ```

#![allow(dead_code)]

#[path = "postgres.rs"]
mod postgres;
#[path = "sqlite.rs"]
mod sqlite;

use async_trait::async_trait;
use sentinel::{Db, NamedRow, SentinelError, Statement, StepResult};
use serde_json::Value;

pub use postgres::PgDb;
pub use sqlite::SqliteDb;

pub enum TestDb {
    Sqlite(SqliteDb),
    Postgres(PgDb),
}

/// A cell as the assertions compare it. Postgres' `BIGINT` arrives as a
/// decimal string (see `sentinel::store::integer`); SQLite's as a number.
pub fn plain(value: Value) -> Value {
    match sentinel::store::integer(&value) {
        Some(number) if value.is_string() => Value::from(number),
        _ => value,
    }
}

pub async fn test_db() -> TestDb {
    match std::env::var("SENTINEL_TEST_POSTGRES_URL") {
        Ok(url) if !url.is_empty() => TestDb::Postgres(PgDb::connect(&url).await),
        _ => TestDb::Sqlite(SqliteDb::in_memory()),
    }
}

#[async_trait]
impl Db for TestDb {
    async fn query(&self, sql: &str, params: Vec<Value>) -> Result<Vec<NamedRow>, SentinelError> {
        match self {
            Self::Sqlite(db) => db.query(sql, params).await,
            Self::Postgres(db) => db.query(sql, params).await,
        }
    }

    async fn execute(&self, sql: &str, params: Vec<Value>) -> Result<u64, SentinelError> {
        match self {
            Self::Sqlite(db) => db.execute(sql, params).await,
            Self::Postgres(db) => db.execute(sql, params).await,
        }
    }

    async fn transaction(
        &self,
        statements: &[Statement],
    ) -> Result<Vec<StepResult>, SentinelError> {
        match self {
            Self::Sqlite(db) => db.transaction(statements).await,
            Self::Postgres(db) => db.transaction(statements).await,
        }
    }
}
