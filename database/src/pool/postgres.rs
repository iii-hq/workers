//! Postgres pool wrapping `deadpool-postgres` over `tokio-postgres`.

use crate::config::{PoolConfig, TlsConfig};
use crate::error::DbError;
use crate::pool::tls::make_pg_connector;
use deadpool_postgres::{Config as DpConfig, ManagerConfig, Pool as DpPool, RecyclingMethod};
use std::sync::Arc;
use std::time::Duration;
use tokio_postgres::NoTls;

#[derive(Clone)]
pub struct PostgresPool {
    inner: Arc<DpPool>,
    db_name: Arc<str>,
    acquire_timeout: Duration,
}

pub type PgClient = deadpool_postgres::Object;

impl PostgresPool {
    pub async fn new(
        url: &str,
        pool_cfg: &PoolConfig,
        tls_cfg: &TlsConfig,
    ) -> Result<Self, DbError> {
        let mut dp = DpConfig::new();
        dp.url = Some(url.to_string());
        dp.manager = Some(ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        });
        // `queue_mode` defaults to `Fifo`; we set it implicitly via
        // `..Default::default()` because `QueueMode` is not re-exported
        // by `deadpool_postgres` (it lives in `deadpool::managed`), and
        // adding a direct `deadpool` dep just for the explicit value would
        // be needless coupling.
        dp.pool = Some(deadpool_postgres::PoolConfig {
            max_size: pool_cfg.max as usize,
            timeouts: deadpool_postgres::Timeouts {
                wait: Some(Duration::from_millis(pool_cfg.acquire_timeout_ms)),
                create: Some(Duration::from_millis(pool_cfg.acquire_timeout_ms)),
                recycle: Some(Duration::from_millis(pool_cfg.idle_timeout_ms)),
            },
            ..Default::default()
        });
        // `make_pg_connector` returns None for `mode: disable`; in that
        // case we hand `NoTls` to deadpool. Otherwise we use the rustls
        // connector which honors the configured chain/hostname policy.
        let pool = match make_pg_connector(tls_cfg)? {
            Some(connector) => dp.create_pool(Some(deadpool_postgres::Runtime::Tokio1), connector),
            None => dp.create_pool(Some(deadpool_postgres::Runtime::Tokio1), NoTls),
        }
        .map_err(|_| DbError::ConfigError {
            message: "postgres pool init failed; check the configured url".into(),
        })?;
        Ok(Self {
            inner: Arc::new(pool),
            db_name: Arc::from("(unset)"),
            acquire_timeout: Duration::from_millis(pool_cfg.acquire_timeout_ms),
        })
    }

    pub fn with_db_name(mut self, name: &str) -> Self {
        self.db_name = Arc::from(name);
        self
    }

    pub async fn acquire(&self) -> Result<PgClient, DbError> {
        let db_name = self.db_name.to_string();
        let waited_ms = self.acquire_timeout.as_millis() as u64;
        self.inner.get().await.map_err(|e| match e {
            deadpool_postgres::PoolError::Timeout(_) => DbError::PoolTimeout {
                db: db_name.clone(),
                waited_ms,
            },
            other => {
                // `deadpool_postgres::PoolError::Display` chains through
                // `tokio_postgres::Error`, which can include the configured
                // host and other connection-string fragments. Logging is
                // operator-only (stderr); the RPC reply gets a generic
                // message so cross-tenant callers don't see infra details.
                tracing::warn!(
                    driver = "postgres",
                    db = %db_name,
                    error = ?other,
                    "pool acquire failed"
                );
                DbError::DriverError {
                    driver: "postgres".into(),
                    code: None,
                    message: "pool connection failed; check server availability".into(),
                    failed_index: None,
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PoolConfig;

    fn url() -> Option<String> {
        std::env::var("TEST_POSTGRES_URL").ok()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pool_acquires_a_connection() {
        let Some(u) = url() else {
            eprintln!("skipping: TEST_POSTGRES_URL not set");
            return;
        };
        // Local docker postgres in tests is plaintext; explicitly disable
        // TLS so the test passes without a server-side cert.
        let tls = crate::config::TlsConfig {
            mode: crate::config::TlsMode::Disable,
            ca_cert: None,
        };
        let pool = PostgresPool::new(&u, &PoolConfig::default(), &tls)
            .await
            .unwrap();
        let client = pool.acquire().await.unwrap();
        let row = client.query_one("SELECT 1::int", &[]).await.unwrap();
        let v: i32 = row.get(0);
        assert_eq!(v, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pg_acquire_failure_message_is_redacted() {
        // Hits a port that nothing listens on so deadpool returns
        // `PoolError::Backend(tokio_postgres::Error)` — the non-Timeout
        // path that previously echoed the underlying error verbatim.
        // Asserts the RPC body uses the generic message and contains
        // none of the userinfo/host fragments from the URL.
        let cfg = PoolConfig {
            max: 1,
            idle_timeout_ms: 1_000,
            acquire_timeout_ms: 500,
        };
        let tls = crate::config::TlsConfig {
            mode: crate::config::TlsMode::Disable,
            ca_cert: None,
        };
        let url = "postgres://leaky_user:leaky_pass@127.0.0.1:1/some_db";
        let pool = PostgresPool::new(url, &cfg, &tls).await.unwrap();
        let err = pool.acquire().await.unwrap_err();
        let body = serde_json::to_string(&err).unwrap();
        assert!(
            body.contains("pool connection failed"),
            "expected generic message; got: {body}"
        );
        for forbidden in ["leaky_user", "leaky_pass", "some_db"] {
            assert!(
                !body.contains(forbidden),
                "leaked `{forbidden}` in RPC body: {body}"
            );
        }
    }
}
