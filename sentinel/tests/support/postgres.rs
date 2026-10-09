//! An in-process [`Db`] over Postgres, so the same statements the SQLite
//! tests run also meet the server a shared team store runs on.
//!
//! It does what the `database` worker's Postgres driver does with a call:
//! `?` numbered as `$n` (what `IiiDb` does before sending), each integer bound
//! at the width the server inferred for its column, reads inside a read-only
//! transaction, and a batch that rolls back on its first failure.
//!
//! Every connection works in a schema of its own, so tests run in parallel
//! against one server and leave their tables behind in a throwaway one.

#![allow(dead_code)]

use async_trait::async_trait;
use sentinel::store::numbered_placeholders;
use sentinel::{Db, NamedRow, SentinelError, Statement, StepResult};
use serde_json::{Number, Value};
use tokio::sync::Mutex;
use tokio_postgres::types::{IsNull, ToSql, Type};
use tokio_postgres::{Client, GenericClient, NoTls, Row};

pub struct PgDb {
    client: Mutex<Client>,
}

impl PgDb {
    pub async fn connect(url: &str) -> Self {
        let (client, connection) = tokio_postgres::connect(url, NoTls)
            .await
            .expect("connect to SENTINEL_TEST_POSTGRES_URL");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let schema = format!("t_{}", uuid::Uuid::now_v7().simple());
        client
            .batch_execute(&format!(
                "CREATE SCHEMA {schema}; SET search_path TO {schema}"
            ))
            .await
            .expect("a schema of its own");
        Self {
            client: Mutex::new(client),
        }
    }
}

#[async_trait]
impl Db for PgDb {
    async fn query(&self, sql: &str, params: Vec<Value>) -> Result<Vec<NamedRow>, SentinelError> {
        let mut client = self.client.lock().await;
        let tx = client
            .build_transaction()
            .read_only(true)
            .start()
            .await
            .map_err(failed)?;
        let rows = run(&tx, sql, &params).await?;
        tx.commit().await.map_err(failed)?;
        rows.iter()
            .map(|row| {
                let mut named = NamedRow::new();
                for (index, column) in row.columns().iter().enumerate() {
                    named.insert(column.name().to_string(), cell(row, index)?);
                }
                Ok(named)
            })
            .collect()
    }

    async fn execute(&self, sql: &str, params: Vec<Value>) -> Result<u64, SentinelError> {
        let client = self.client.lock().await;
        let sql = numbered_placeholders(sql);
        let params = bind_values(&params);
        client
            .execute(sql.as_str(), &refs(&params))
            .await
            .map_err(failed)
    }

    async fn transaction(
        &self,
        statements: &[Statement],
    ) -> Result<Vec<StepResult>, SentinelError> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(failed)?;
        let mut results = Vec::with_capacity(statements.len());
        for statement in statements {
            let sql = numbered_placeholders(&statement.sql);
            let prepared = tx.prepare(&sql).await.map_err(failed)?;
            let params = bind_values(&statement.params);
            let refs = refs(&params);
            if prepared.columns().is_empty() {
                let affected_rows = tx.execute(&prepared, &refs).await.map_err(failed)?;
                results.push(StepResult {
                    affected_rows,
                    rows: Vec::new(),
                });
            } else {
                let rows = tx.query(&prepared, &refs).await.map_err(failed)?;
                let rows = rows
                    .iter()
                    .map(|row| (0..row.len()).map(|index| cell(row, index)).collect())
                    .collect::<Result<Vec<_>, _>>()?;
                results.push(StepResult {
                    affected_rows: 0,
                    rows,
                });
            }
        }
        tx.commit().await.map_err(failed)?;
        Ok(results)
    }
}

async fn run(
    client: &impl GenericClient,
    sql: &str,
    params: &[Value],
) -> Result<Vec<Row>, SentinelError> {
    let sql = numbered_placeholders(sql);
    let params = bind_values(params);
    client
        .query(sql.as_str(), &refs(&params))
        .await
        .map_err(failed)
}

/// A JSON parameter, written at the width the server asked for.
#[derive(Debug)]
struct Param(Value);

impl ToSql for Param {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut bytes::BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match &self.0 {
            Value::Null => Ok(IsNull::Yes),
            Value::Bool(value) => value.to_sql(ty, out),
            Value::Number(number) => match number.as_i64() {
                Some(value) => match *ty {
                    Type::INT2 => i16::try_from(value)?.to_sql(ty, out),
                    Type::INT4 => i32::try_from(value)?.to_sql(ty, out),
                    Type::FLOAT4 => (value as f32).to_sql(ty, out),
                    Type::FLOAT8 => (value as f64).to_sql(ty, out),
                    _ => value.to_sql(ty, out),
                },
                None => {
                    let value = number.as_f64().unwrap_or(0.0);
                    match *ty {
                        Type::FLOAT4 => (value as f32).to_sql(ty, out),
                        _ => value.to_sql(ty, out),
                    }
                }
            },
            Value::String(value) => value.as_str().to_sql(ty, out),
            other => other.to_string().to_sql(ty, out),
        }
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }

    tokio_postgres::types::to_sql_checked!();
}

fn bind_values(params: &[Value]) -> Vec<Param> {
    params.iter().cloned().map(Param).collect()
}

fn refs(params: &[Param]) -> Vec<&(dyn ToSql + Sync)> {
    params
        .iter()
        .map(|param| param as &(dyn ToSql + Sync))
        .collect()
}

/// One cell as the `database` worker reports it.
fn cell(row: &Row, index: usize) -> Result<Value, SentinelError> {
    let ty = row.columns()[index].type_().clone();
    let value = match ty {
        Type::BOOL => row
            .try_get::<_, Option<bool>>(index)
            .map(|v| v.map(Value::Bool)),
        Type::INT2 => row
            .try_get::<_, Option<i16>>(index)
            .map(|v| v.map(Value::from)),
        Type::INT4 => row
            .try_get::<_, Option<i32>>(index)
            .map(|v| v.map(Value::from)),
        // As the worker does: a decimal string, so JavaScript cannot round it.
        Type::INT8 => row
            .try_get::<_, Option<i64>>(index)
            .map(|v| v.map(|v| Value::String(v.to_string()))),
        Type::FLOAT4 => row.try_get::<_, Option<f32>>(index).map(|v| {
            v.and_then(|v| Number::from_f64(v as f64))
                .map(Value::Number)
        }),
        Type::FLOAT8 => row
            .try_get::<_, Option<f64>>(index)
            .map(|v| v.and_then(Number::from_f64).map(Value::Number)),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => row
            .try_get::<_, Option<String>>(index)
            .map(|v| v.map(Value::String)),
        other => {
            return Err(SentinelError::dependency(format!(
                "column {} has type {other}, which these tests do not decode",
                row.columns()[index].name()
            )))
        }
    };
    Ok(value.map_err(failed)?.unwrap_or(Value::Null))
}

/// As the worker reports it: the SQLSTATE and "db error", never the server's
/// own message, so a test cannot lean on words the worker does not pass on.
fn failed(error: tokio_postgres::Error) -> SentinelError {
    SentinelError::dependency(
        serde_json::json!({
            "code": "DRIVER_ERROR",
            "driver": "postgres",
            "inner_code": error.code().map(|code| code.code()),
            "message": error.to_string(),
        })
        .to_string(),
    )
}
