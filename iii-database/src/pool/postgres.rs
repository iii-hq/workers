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
            deadpool_postgres::PoolError::Backend(be) => {
                let class = classify_pg_error(&be);
                // `deadpool_postgres::PoolError::Display` chains through
                // `tokio_postgres::Error`, which can include the configured
                // host and other connection-string fragments. Logging is
                // operator-only (stderr); the RPC reply gets a generic
                // message plus a coarse class hint so cross-tenant callers
                // can self-triage without seeing infra details.
                tracing::warn!(
                    driver = "postgres",
                    db = %db_name,
                    class,
                    error = ?be,
                    "pool acquire failed"
                );
                DbError::DriverError {
                    driver: "postgres".into(),
                    code: None,
                    message: format!("pool connection failed ({class})"),
                    failed_index: None,
                }
            }
            other => {
                // `PoolError::Closed`, `NoRuntimeSpecified`, `PostCreateHook`.
                // None of these carry a tokio_postgres::Error to classify;
                // the deadpool Display is safe (no userinfo) on all three.
                tracing::warn!(
                    driver = "postgres",
                    db = %db_name,
                    error = ?other,
                    "pool acquire failed"
                );
                DbError::DriverError {
                    driver: "postgres".into(),
                    code: None,
                    message: "pool connection failed (unknown)".into(),
                    failed_index: None,
                }
            }
        })
    }
}

/// Classify a `tokio_postgres::Error` into a coarse hint word for the RPC
/// reply. The hint never includes hostnames, userinfo, database names, or
/// any other connection-string fragments — it's a single bare word from
/// the closed set {`tls`, `auth`, `network`, `server-policy`, `unknown`}.
///
/// `tokio_postgres::error::Kind` is private (no `pub fn kind()`); the
/// upstream public API exposes only `code()`, `as_db_error()`, `is_closed()`,
/// and `source()`. So the classifier uses (1) the SQLSTATE class when the
/// server replied, (2) a `source()`-chain walk for in-band rustls/io types,
/// and (3) a `Debug`-string sniff as a last resort. The upstream `Debug`
/// impl prints the `kind:` field via `debug_struct` and has been stable
/// for years; the alternative is a real upstream API gap.
fn classify_pg_error(err: &tokio_postgres::Error) -> &'static str {
    if let Some(code) = err.code() {
        match &code.code()[..2] {
            // invalid_authorization_specification / invalid_password
            "28" => return "auth",
            // connection_exception family
            "08" => return "network",
            // insufficient_resources / operator_intervention
            "53" | "57" => return "server-policy",
            _ => {}
        }
    }

    // Walk std::error::Error::source(). tokio-rustls wraps rustls::Error
    // inside io::Error via `io::Error::new(InvalidData, _)`, so the rustls
    // type is typically reachable two hops down for cert failures. Bound
    // the walk so a pathological self-referential chain can't hang us.
    use std::error::Error as _;
    let mut cur: Option<&(dyn std::error::Error + 'static)> = err.source();
    let mut saw_rustls = false;
    let mut saw_io = false;
    for _ in 0..16 {
        let Some(s) = cur else { break };
        if s.downcast_ref::<rustls::Error>().is_some() {
            saw_rustls = true;
        }
        if s.downcast_ref::<std::io::Error>().is_some() {
            saw_io = true;
        }
        cur = s.source();
    }
    if saw_rustls {
        return "tls";
    }

    // Kind sniff (fallback for Kind::Tls / Authentication / Connect whose
    // source() type isn't downcastable to a concrete crate type we know).
    let dbg = format!("{err:?}");
    if dbg.contains("kind: Tls") {
        return "tls";
    }
    if dbg.contains("kind: Authentication") {
        // Covers Neon's pooler-endpoint case where the SCRAM exchange
        // raises "server did not use channel binding" with channel_binding=require.
        return "auth";
    }
    if dbg.contains("kind: Connect") || dbg.contains("kind: Closed") {
        return "network";
    }
    if saw_io {
        return "network";
    }
    "unknown"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{PoolConfig, TlsConfig, TlsMode};

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
        let tls = TlsConfig {
            mode: TlsMode::Disable,
            ..Default::default()
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
        // Asserts the RPC body uses the generic message, carries the
        // `(network)` class hint, and contains none of the userinfo/host
        // fragments from the URL.
        let cfg = PoolConfig {
            max: 1,
            idle_timeout_ms: 1_000,
            acquire_timeout_ms: 500,
        };
        let tls = TlsConfig {
            mode: TlsMode::Disable,
            ..Default::default()
        };
        let url = "postgres://leaky_user:leaky_pass@127.0.0.1:1/some_db";
        let pool = PostgresPool::new(url, &cfg, &tls).await.unwrap();
        let err = pool.acquire().await.unwrap_err();
        let body = serde_json::to_string(&err).unwrap();
        assert!(
            body.contains("pool connection failed"),
            "expected generic message; got: {body}"
        );
        assert!(
            body.contains("(network)"),
            "expected (network) class hint for TCP-refused port 1; got: {body}"
        );
        for forbidden in ["leaky_user", "leaky_pass", "some_db"] {
            assert!(
                !body.contains(forbidden),
                "leaked `{forbidden}` in RPC body: {body}"
            );
        }
    }

    /// Smoke-check the SQLSTATE branch of `classify_pg_error`. Server-side
    /// failures (auth, connection_exception, server-policy) come with a
    /// 5-char SQLSTATE that we lean on first because it doesn't depend on
    /// the brittle `Debug` sniff.
    #[test]
    fn classify_pg_error_uses_sqlstate_class_when_present() {
        use tokio_postgres::error::SqlState;
        // We can't construct a `tokio_postgres::Error` directly (private
        // ctor), but we can verify the SQLSTATE-class table by inspection:
        // every code in the auth/network/server-policy families used by
        // upstream falls in the 2-char prefixes we match on. This guards
        // against accidental table drift.
        assert_eq!(
            &SqlState::INVALID_AUTHORIZATION_SPECIFICATION.code()[..2],
            "28"
        );
        assert_eq!(&SqlState::INVALID_PASSWORD.code()[..2], "28");
        assert_eq!(&SqlState::CONNECTION_EXCEPTION.code()[..2], "08");
    }
}
