//! An in-process [`Db`] over SQLite, for testing the store's statements
//! against the driver the `database` worker actually runs them on.
//!
//! This is deliberately not a hand-written fake of the store: a fake agrees
//! with whatever the code does, while this disagrees when the SQL is wrong —
//! the CHECK constraint, the partial unique index and the `RETURNING` a
//! compare-and-set depends on are all real here.

#![allow(dead_code)]

use async_trait::async_trait;
use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::Connection;
use sentinel::{Db, NamedRow, SentinelError, Statement, StepResult};
use serde_json::{Number, Value};
use tokio::sync::Mutex;

pub struct SqliteDb {
    connection: Mutex<Connection>,
}

impl SqliteDb {
    pub fn in_memory() -> Self {
        let connection = Connection::open_in_memory().expect("open an in-memory database");
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .expect("enable foreign keys");
        Self {
            connection: Mutex::new(connection),
        }
    }
}

#[async_trait]
impl Db for SqliteDb {
    async fn query(&self, sql: &str, params: Vec<Value>) -> Result<Vec<NamedRow>, SentinelError> {
        let connection = self.connection.lock().await;
        let mut statement = connection.prepare(sql).map_err(failed)?;
        let columns: Vec<String> = statement
            .column_names()
            .into_iter()
            .map(str::to_string)
            .collect();
        let mut rows = statement
            .query(rusqlite::params_from_iter(bind(&params)))
            .map_err(failed)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(failed)? {
            let mut named = NamedRow::new();
            for (index, column) in columns.iter().enumerate() {
                named.insert(column.clone(), json_of(row.get_ref(index).map_err(failed)?));
            }
            out.push(named);
        }
        Ok(out)
    }

    async fn execute(&self, sql: &str, params: Vec<Value>) -> Result<u64, SentinelError> {
        let connection = self.connection.lock().await;
        let changed = connection
            .execute(sql, rusqlite::params_from_iter(bind(&params)))
            .map_err(failed)?;
        Ok(changed as u64)
    }

    async fn transaction(
        &self,
        statements: &[Statement],
    ) -> Result<Vec<StepResult>, SentinelError> {
        let mut connection = self.connection.lock().await;
        let tx = connection.transaction().map_err(failed)?;
        let mut results = Vec::with_capacity(statements.len());
        for statement in statements {
            let mut prepared = tx.prepare(&statement.sql).map_err(failed)?;
            let column_count = prepared.column_count();
            let mut rows = prepared
                .query(rusqlite::params_from_iter(bind(&statement.params)))
                .map_err(failed)?;
            let mut returned = Vec::new();
            while let Some(row) = rows.next().map_err(failed)? {
                let mut values = Vec::with_capacity(column_count);
                for index in 0..column_count {
                    values.push(json_of(row.get_ref(index).map_err(failed)?));
                }
                returned.push(values);
            }
            drop(rows);
            drop(prepared);
            results.push(StepResult {
                // `changes()` counts the rows the last statement wrote; a
                // RETURNING statement reports them as rows instead.
                affected_rows: tx.changes(),
                rows: returned,
            });
        }
        tx.commit().map_err(failed)?;
        Ok(results)
    }
}

fn bind(params: &[Value]) -> Vec<SqlValue> {
    params
        .iter()
        .map(|param| match param {
            Value::Null => SqlValue::Null,
            Value::Bool(value) => SqlValue::Integer(i64::from(*value)),
            Value::Number(value) => value
                .as_i64()
                .map(SqlValue::Integer)
                .or_else(|| value.as_f64().map(SqlValue::Real))
                .unwrap_or(SqlValue::Null),
            Value::String(value) => SqlValue::Text(value.clone()),
            other => SqlValue::Text(other.to_string()),
        })
        .collect()
}

fn json_of(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(number) => Value::Number(number.into()),
        ValueRef::Real(number) => Number::from_f64(number)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        ValueRef::Text(text) => Value::String(String::from_utf8_lossy(text).into_owned()),
        ValueRef::Blob(bytes) => Value::String(String::from_utf8_lossy(bytes).into_owned()),
    }
}

fn failed(error: rusqlite::Error) -> SentinelError {
    SentinelError::dependency(error.to_string())
}
