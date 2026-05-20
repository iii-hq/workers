//! MySQL pool wrapping `mysql_async::Pool`.

use crate::config::{PoolConfig, TlsConfig};
use crate::error::DbError;
use crate::pool::tls::make_mysql_ssl_opts;
use mysql_async::{Pool as MyPool, PoolConstraints, PoolOpts};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct MysqlPool {
    inner: Arc<MyPool>,
    db_name: Arc<str>,
    acquire_timeout: Duration,
}

pub type MysqlConn = mysql_async::Conn;

impl MysqlPool {
    pub fn new(url: &str, pool_cfg: &PoolConfig, tls_cfg: &TlsConfig) -> Result<Self, DbError> {
        let constraints =
            PoolConstraints::new(0, pool_cfg.max as usize).ok_or_else(|| DbError::ConfigError {
                message: "mysql pool constraints invalid".into(),
            })?;
        let opts = PoolOpts::default()
            .with_constraints(constraints)
            .with_inactive_connection_ttl(Duration::from_millis(pool_cfg.idle_timeout_ms));
        // Don't echo the underlying parse error — mysql_async's error message
        // includes the offending URL verbatim, which would leak any embedded
        // password into logs. Surface a generic message; the operator knows
        // which db they configured.
        let mut url_opts = mysql_async::OptsBuilder::from_opts(
            mysql_async::Opts::from_url(url).map_err(|_| DbError::ConfigError {
                message: "mysql url parse failed; check the configured url".into(),
            })?,
        );
        url_opts = url_opts.pool_opts(opts);
        // `make_mysql_ssl_opts` returns None for `mode: disable`; in that
        // case we leave the SSL knob untouched (= no TLS).
        if let Some(ssl) = make_mysql_ssl_opts(tls_cfg)? {
            url_opts = url_opts.ssl_opts(ssl);
        }
        let pool = MyPool::new(url_opts);
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

    pub async fn acquire(&self) -> Result<MysqlConn, DbError> {
        let db_name = self.db_name.to_string();
        let waited_ms = self.acquire_timeout.as_millis() as u64;
        let fut = self.inner.get_conn();
        match tokio::time::timeout(self.acquire_timeout, fut).await {
            Ok(Ok(conn)) => Ok(conn),
            Ok(Err(e)) => {
                let class = classify_mysql_error(&e);
                // `mysql_async::Error::Display` can echo the server address,
                // username, and other connection details. Log the full error
                // operator-side via tracing; surface a generic message plus
                // a coarse class hint in the RPC reply so untrusted callers
                // can self-triage without seeing infra details.
                tracing::warn!(
                    driver = "mysql",
                    db = %db_name,
                    class,
                    error = ?e,
                    "pool acquire failed"
                );
                Err(DbError::DriverError {
                    driver: "mysql".into(),
                    code: None,
                    message: format!("pool connection failed ({class})"),
                    failed_index: None,
                })
            }
            Err(_) => Err(DbError::PoolTimeout {
                db: db_name,
                waited_ms,
            }),
        }
    }
}

/// Classify a `mysql_async::Error` into a coarse hint word for the RPC
/// reply. Same closed set as the postgres classifier — `tls`, `auth`,
/// `network`, `server-policy`, or `unknown` — and the same redaction
/// contract (no host/userinfo/db ever appears in the returned word).
///
/// `mysql_async::Error` is a public exhaustive enum, so this is a
/// straight `match` — no `Debug`-string sniffing required.
fn classify_mysql_error(err: &mysql_async::Error) -> &'static str {
    use mysql_async::{DriverError, Error, IoError};
    match err {
        // TLS handshake / cert chain failure (gated on the `rustls-tls`
        // feature that we already enable in Cargo.toml).
        Error::Io(IoError::Tls(_)) => "tls",
        // TCP refuse, DNS, route, peer reset, etc.
        Error::Io(IoError::Io(_)) => "network",
        // Server-side rejections by error code. The codes here cover the
        // standard auth surface (access denied, host blocked, bad SSL).
        // Everything else from the server (max_connections, replication
        // lag, etc.) lands under server-policy.
        Error::Server(s) if matches!(s.code, 1045 | 1129 | 1130 | 1156 | 1251 | 1698) => "auth",
        Error::Server(_) => "server-policy",
        Error::Driver(DriverError::ConnectionClosed) => "network",
        // `Url(_)` shouldn't happen here — pool construction already
        // parsed the URL via `Opts::from_url`. Classify defensively.
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{TlsConfig, TlsMode};

    fn url() -> Option<String> {
        std::env::var("TEST_MYSQL_URL").ok()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mysql_pool_acquires() {
        let Some(u) = url() else { return };
        // Local docker mysql in tests is plaintext; explicitly disable TLS.
        let tls = TlsConfig {
            mode: TlsMode::Disable,
            ..Default::default()
        };
        let pool = MysqlPool::new(&u, &PoolConfig::default(), &tls).unwrap();
        let mut conn = pool.acquire().await.unwrap();
        use mysql_async::prelude::Queryable;
        let v: Option<i64> = conn.query_first("SELECT 1").await.unwrap();
        assert_eq!(v, Some(1));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mysql_acquire_failure_message_is_redacted() {
        // Hits a port that nothing listens on so mysql_async surfaces a
        // connect error — the non-Timeout path that previously echoed the
        // underlying error verbatim. Asserts the RPC body uses the generic
        // message, carries the `(network)` class hint, and does not leak
        // userinfo/host fragments.
        let cfg = PoolConfig {
            max: 1,
            idle_timeout_ms: 1_000,
            acquire_timeout_ms: 500,
        };
        let tls = TlsConfig {
            mode: TlsMode::Disable,
            ..Default::default()
        };
        let url = "mysql://leaky_user:leaky_pass@127.0.0.1:1/some_db";
        let pool = MysqlPool::new(url, &cfg, &tls).unwrap();
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

    /// Unit-check the variants of `classify_mysql_error` we can construct
    /// without a live server. `mysql_async::Error` is public so each branch
    /// is exercised directly; covers the contract that auth maps to "auth"
    /// and other server errors map to "server-policy".
    #[test]
    fn classify_mysql_error_branches() {
        use mysql_async::{Error, ServerError};
        // 1045 = ER_ACCESS_DENIED_ERROR — the canonical "wrong password".
        let auth = Error::Server(ServerError {
            code: 1045,
            message: "Access denied".into(),
            state: "28000".into(),
        });
        assert_eq!(classify_mysql_error(&auth), "auth");
        // 1040 = ER_CON_COUNT_ERROR ("too many connections") — server-policy.
        let policy = Error::Server(ServerError {
            code: 1040,
            message: "Too many connections".into(),
            state: "08004".into(),
        });
        assert_eq!(classify_mysql_error(&policy), "server-policy");
    }
}
