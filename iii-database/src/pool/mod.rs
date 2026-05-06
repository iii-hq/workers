//! Connection pool dispatch. The `Pool` enum holds one of three concrete
//! pool types; method-level dispatch lives in driver-specific modules and
//! is wired in by `handlers::*` via `match pool { ... }`.

pub(crate) mod mysql;
pub(crate) mod postgres;
pub(crate) mod sqlite;
pub(crate) mod tls;

use crate::config::DriverKind;
use crate::error::DbError;
pub(crate) use mysql::MysqlPool;
pub(crate) use postgres::PostgresPool;
pub(crate) use sqlite::SqlitePool;

#[derive(Clone)]
pub enum Pool {
    Postgres(PostgresPool),
    Mysql(MysqlPool),
    Sqlite(SqlitePool),
}

impl Pool {
    pub fn driver(&self) -> DriverKind {
        match self {
            Pool::Postgres(_) => DriverKind::Postgres,
            Pool::Mysql(_) => DriverKind::Mysql,
            Pool::Sqlite(_) => DriverKind::Sqlite,
        }
    }
}

/// Build a pool for a single configured database. Used at startup by main.rs.
pub async fn build(db_name: &str, cfg: &crate::config::DatabaseConfig) -> Result<Pool, DbError> {
    match cfg.driver {
        DriverKind::Sqlite => {
            // Sqlite is local-file; the `tls` block has no meaning for it.
            SqlitePool::new(&cfg.url, &cfg.pool).map(|p| Pool::Sqlite(p.with_db_name(db_name)))
        }
        DriverKind::Postgres => PostgresPool::new(&cfg.url, &cfg.pool, &cfg.tls)
            .await
            .map(|p| Pool::Postgres(p.with_db_name(db_name))),
        DriverKind::Mysql => MysqlPool::new(&cfg.url, &cfg.pool, &cfg.tls)
            .map(|p| Pool::Mysql(p.with_db_name(db_name))),
    }
}
