//! `database::testConnection` — probe a CANDIDATE database config.
//!
//! Exists for the console's configuration form: the operator edits a url,
//! presses "test connection", and learns whether it works *before* saving.
//! The probe therefore takes the url/tls straight from the request instead
//! of a configured handle, opens one throwaway connection outside every
//! pool, runs the smallest possible query, and reports the outcome as data
//! (`ok: false` is a normal response, not a handler error).
//!
//! Error texts are returned verbatim except that the url's credentials are
//! scrubbed. That is safe here where it would not be elsewhere: everything
//! echoed back originates from the caller's own request, so nothing
//! cross-tenant can leak — but a driver error that quotes the url must not
//! turn a stored password into log/UI text.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

use crate::config::{detect_driver, DriverKind, TlsConfig};
use crate::pool::tls::make_pg_connector;
use crate::transaction::driver_system;

const DEFAULT_TIMEOUT_MS: u64 = 5_000;
const MAX_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TestConnectionReq {
    /// Connection url to probe (`postgres://…`, `mysql://…`, `sqlite:…`).
    pub url: String,
    /// TLS settings to probe with. Defaults like a configured database
    /// (mode `require`) when omitted.
    #[serde(default)]
    pub tls: Option<TlsConfig>,
    /// Overall budget for the attempt. Default 5000, capped at 30000.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TestConnectionResp {
    /// Whether a connection was established and answered a query.
    pub ok: bool,
    /// "postgres" | "mysql" | "sqlite" | "unknown".
    pub driver: String,
    /// Wall time of the whole attempt.
    pub latency_ms: u64,
    /// Server version string, when the probe got far enough to ask.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
    /// Why the probe failed (credentials scrubbed). Absent on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

pub async fn handle(req: TestConnectionReq) -> Result<TestConnectionResp, crate::error::DbError> {
    let started = Instant::now();
    let Some(driver) = detect_driver(&req.url) else {
        return Ok(TestConnectionResp {
            ok: false,
            driver: "unknown".into(),
            latency_ms: 0,
            server_version: None,
            message: Some(
                "unknown url scheme — expected sqlite:, postgres:// / postgresql://, or mysql://"
                    .into(),
            ),
        });
    };
    let tls = req.tls.clone().unwrap_or_default();
    let budget = Duration::from_millis(
        req.timeout_ms
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS),
    );

    let outcome = tokio::time::timeout(budget, probe(driver, &req.url, &tls)).await;
    let latency_ms = started.elapsed().as_millis() as u64;
    let (ok, server_version, message) = match outcome {
        Ok(Ok(version)) => (true, Some(version), None),
        Ok(Err(e)) => (false, None, Some(scrub_credentials(&e, &req.url))),
        Err(_) => (
            false,
            None,
            Some(format!("timed out after {}ms", budget.as_millis())),
        ),
    };
    Ok(TestConnectionResp {
        ok,
        driver: driver_system(driver).to_string(),
        latency_ms,
        server_version,
        message,
    })
}

async fn probe(driver: DriverKind, url: &str, tls: &TlsConfig) -> Result<String, String> {
    match driver {
        DriverKind::Postgres => probe_postgres(url, tls).await,
        DriverKind::Mysql => probe_mysql(url, tls).await,
        DriverKind::Sqlite => probe_sqlite(url).await,
    }
}

async fn probe_postgres(url: &str, tls: &TlsConfig) -> Result<String, String> {
    async fn ping(client: tokio_postgres::Client) -> Result<String, String> {
        let row = client
            .query_one("SELECT version()", &[])
            .await
            .map_err(|e| e.to_string())?;
        Ok(row.get::<_, String>(0))
    }
    match make_pg_connector(tls).map_err(|e| format!("{e:?}"))? {
        Some(connector) => {
            let (client, conn) = tokio_postgres::connect(url, connector)
                .await
                .map_err(|e| e.to_string())?;
            tokio::spawn(async move {
                let _ = conn.await;
            });
            ping(client).await
        }
        None => {
            let (client, conn) = tokio_postgres::connect(url, tokio_postgres::NoTls)
                .await
                .map_err(|e| e.to_string())?;
            tokio::spawn(async move {
                let _ = conn.await;
            });
            ping(client).await
        }
    }
}

async fn probe_mysql(url: &str, tls: &TlsConfig) -> Result<String, String> {
    use mysql_async::prelude::Queryable as _;
    let opts = crate::triggers::mysql_binlog::build_opts(url, tls)?;
    let mut conn = mysql_async::Conn::new(opts)
        .await
        .map_err(|e| e.to_string())?;
    let version: Option<String> = conn
        .query_first("SELECT VERSION()")
        .await
        .map_err(|e| e.to_string())?;
    let _ = conn.disconnect().await;
    version.ok_or_else(|| "server returned no version row".into())
}

async fn probe_sqlite(url: &str) -> Result<String, String> {
    let url = url.to_string();
    tokio::task::spawn_blocking(move || {
        let version = |conn: &rusqlite::Connection| -> Result<String, String> {
            conn.query_row("SELECT sqlite_version()", [], |r| r.get::<_, String>(0))
                .map(|v| format!("SQLite {v}"))
                .map_err(|e| e.to_string())
        };
        if url.contains(":memory:") {
            let conn = rusqlite::Connection::open_in_memory().map_err(|e| e.to_string())?;
            return version(&conn);
        }
        let Some(path) = crate::triggers::sqlite_watch::sqlite_file_path(&url) else {
            return Err("unreadable sqlite url".into());
        };
        // Open WITHOUT the create flag: a probe must not leave a database
        // file behind. A missing file is still useful news — the pool
        // creates it (and any parent dirs) when the configuration is saved.
        if !path.exists() {
            return Err(format!(
                "file {} does not exist yet — it is created automatically when this \
                 configuration is saved",
                path.display()
            ));
        }
        let conn = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .map_err(|e| e.to_string())?;
        version(&conn)
    })
    .await
    .map_err(|e| format!("probe task failed: {e}"))?
}

/// Strip the url's username/password out of an error text — drivers quote
/// the connection string in some failure modes.
fn scrub_credentials(message: &str, url: &str) -> String {
    let mut out = message.to_string();
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some(password) = parsed.password() {
            if !password.is_empty() {
                out = out.replace(password, "***");
            }
        }
        if !parsed.username().is_empty() {
            out = out.replace(&format!("{}:", parsed.username()), "***:");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(url: &str) -> TestConnectionReq {
        TestConnectionReq {
            url: url.into(),
            tls: None,
            timeout_ms: None,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sqlite_memory_probes_ok_with_a_version() {
        let resp = handle(req("sqlite::memory:")).await.unwrap();
        assert!(resp.ok, "{:?}", resp.message);
        assert_eq!(resp.driver, "sqlite");
        assert!(resp.server_version.unwrap().starts_with("SQLite "));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sqlite_existing_file_probes_ok_and_missing_file_explains() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("probe.db");
        rusqlite::Connection::open(&path).unwrap();

        let resp = handle(req(&format!("sqlite:{}", path.display())))
            .await
            .unwrap();
        assert!(resp.ok, "{:?}", resp.message);

        let missing = dir.path().join("not-yet.db");
        let resp = handle(req(&format!("sqlite:{}", missing.display())))
            .await
            .unwrap();
        assert!(!resp.ok);
        assert!(resp.message.unwrap().contains("does not exist yet"));
        // The probe must not have created it.
        assert!(!missing.exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unknown_scheme_reports_without_erroring() {
        let resp = handle(req("mongodb://nope")).await.unwrap();
        assert!(!resp.ok);
        assert_eq!(resp.driver, "unknown");
        assert!(resp.message.unwrap().contains("unknown url scheme"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn refused_postgres_fails_fast_and_scrubs_credentials() {
        // Port 1 refuses instantly; the error must carry neither the
        // password nor the username from the probed url.
        let mut r = req("postgres://leaky_user:leaky_pass@127.0.0.1:1/db");
        r.tls = Some(TlsConfig {
            mode: crate::config::TlsMode::Disable,
            ..Default::default()
        });
        let resp = handle(r).await.unwrap();
        assert!(!resp.ok);
        assert_eq!(resp.driver, "postgres");
        let message = resp.message.unwrap();
        assert!(!message.contains("leaky_pass"), "{message}");
        assert!(!message.contains("leaky_user:"), "{message}");
    }

    #[test]
    fn scrub_replaces_userinfo_everywhere() {
        let scrubbed = scrub_credentials(
            "connect to postgres://u:sekret@h failed for u:sekret",
            "postgres://u:sekret@h/db",
        );
        assert!(!scrubbed.contains("sekret"), "{scrubbed}");
    }

    /// Live probes, gated like the pool tests.
    #[tokio::test(flavor = "multi_thread")]
    async fn live_postgres_and_mysql_probe_ok_when_configured() {
        for (env, prefix) in [
            ("TEST_POSTGRES_URL", "postgres"),
            ("TEST_MYSQL_URL", "mysql"),
        ] {
            let Some(url) = std::env::var(env).ok() else {
                eprintln!("skipping: {env} not set");
                continue;
            };
            let mut r = req(&url);
            r.tls = Some(TlsConfig {
                mode: crate::config::TlsMode::Disable,
                ..Default::default()
            });
            let resp = handle(r).await.unwrap();
            assert!(resp.ok, "{env}: {:?}", resp.message);
            assert_eq!(resp.driver, prefix);
            assert!(resp.server_version.is_some());
        }
    }
}
