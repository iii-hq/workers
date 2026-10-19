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

/// Live pool occupancy, for `database::health`.
///
/// `size` and `idle` are `None` where the underlying pool does not expose
/// them — `mysql_async` keeps its counters private. Reporting `None` rather
/// than zero matters: "unknown" and "no idle connections" are different
/// answers, and a health panel that conflates them is actively misleading.
#[derive(Debug, Clone, Copy, serde::Serialize, schemars::JsonSchema)]
pub struct PoolStats {
    pub max: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting: Option<u32>,
}

impl Pool {
    pub fn driver(&self) -> DriverKind {
        match self {
            Pool::Postgres(_) => DriverKind::Postgres,
            Pool::Mysql(_) => DriverKind::Mysql,
            Pool::Sqlite(_) => DriverKind::Sqlite,
        }
    }

    pub fn stats(&self) -> PoolStats {
        match self {
            Pool::Postgres(p) => p.stats(),
            Pool::Mysql(p) => p.stats(),
            Pool::Sqlite(p) => p.stats(),
        }
    }
}

/// Build a pool for a single configured database from its literal values
/// (no secrets worker involved — references fail). Tests and tools; the
/// worker builds through [`from_connector`] with resolved references.
pub async fn build(db_name: &str, cfg: &crate::config::DatabaseConfig) -> Result<Pool, DbError> {
    let connector = crate::connector::Connector::literal(db_name, cfg)?;
    from_connector(db_name, std::sync::Arc::new(connector), &cfg.pool)
}

/// Build a pool over a resolved connector. Postgres and mysql pools open
/// connections lazily; sqlite opens (and creates) its file now.
pub fn from_connector(
    db_name: &str,
    connector: std::sync::Arc<crate::connector::Connector>,
    pool_cfg: &crate::config::PoolConfig,
) -> Result<Pool, DbError> {
    match connector.driver {
        DriverKind::Sqlite => {
            // Sqlite is local-file; the `tls` block has no meaning for it.
            let url = connector.sqlite_url().unwrap_or_default().to_string();
            SqlitePool::new(&url, pool_cfg).map(|p| Pool::Sqlite(p.with_db_name(db_name)))
        }
        DriverKind::Postgres => PostgresPool::from_connector(connector, pool_cfg)
            .map(|p| Pool::Postgres(p.with_db_name(db_name))),
        DriverKind::Mysql => MysqlPool::from_connector(connector, pool_cfg)
            .map(|p| Pool::Mysql(p.with_db_name(db_name))),
    }
}

/// `Some("ssh")` when a pool connection just failed because its SSH tunnel
/// did (the driver only sees a closed socket). The detail is logged, not
/// returned: it names infrastructure.
pub(crate) fn tunnel_class(connector: &crate::connector::Connector) -> Option<&'static str> {
    let tunnel = connector.tunnel()?;
    let since = tokio::time::Instant::now().checked_sub(std::time::Duration::from_secs(30))?;
    let error = tunnel.error_since(since)?;
    tracing::warn!(db = %connector.name, error = %error.message, "pool connection failed in the ssh tunnel");
    Some(match error.kind {
        crate::ssh::TunnelErrorKind::Forward => "network",
        _ => "ssh",
    })
}
