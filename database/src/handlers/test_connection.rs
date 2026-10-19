//! `database::testConnection` — probe a CANDIDATE database config.
//!
//! Exists for connection editors: the operator fills in an entry, presses
//! "test connection", and learns whether it works *before* saving. The probe
//! takes the entry straight from the request (the same url or structured
//! form `saveConnection` takes; the legacy `{url, tls}` still works), opens
//! one throwaway connection outside every pool — through a throwaway SSH
//! tunnel when the entry has one — runs the smallest possible query, and
//! reports the outcome as data (`ok: false` is a normal response, not a
//! handler error) with the `stage` that failed.
//!
//! `"__redacted__"` values (what `getConnection` shows for stored literals)
//! are taken from the stored entry `name`. `secret://` / `env://` values are
//! resolved through the secrets worker like a configured connection's.
//!
//! Error texts are returned verbatim except that every credential the probe
//! used is scrubbed. That is safe here where it would not be elsewhere:
//! everything echoed back originates from the caller's own request (or the
//! entry it named), so nothing cross-tenant can leak — but a driver error
//! that quotes a password must not turn it into log/UI text.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};

use super::AppState;
use crate::config::{detect_driver, TlsConfig};
use crate::connector::{self, ConnectError, Connector, Material, Stage};
use crate::ssh::HostKeyInfo;
use crate::transaction::driver_system;

const DEFAULT_TIMEOUT_MS: u64 = 5_000;
const MAX_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct TestConnectionReq {
    /// The candidate entry: `{url, tls?, ssh?}` or the structured form
    /// (`driver`, `host`, `port`, `user`, `password`, `database`, `tls`,
    /// `ssh`). `"__redacted__"` takes that value from the stored entry
    /// `name`.
    #[serde(default)]
    pub connection: Option<Value>,
    /// The stored connection `"__redacted__"` values come from.
    #[serde(default)]
    pub name: Option<String>,
    /// Legacy: a connection url to probe (`postgres://…`, `mysql://…`,
    /// `sqlite:…`) when `connection` is absent.
    #[serde(default)]
    pub url: Option<String>,
    /// Legacy: TLS settings for `url`. Defaults like a configured database
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
    /// The step that failed: `config`, `secrets`, `ssh`, `tcp`, `tls`,
    /// `auth`, `database`. Absent on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<Stage>,
    /// The SSH server's host key as the tunnel saw it (also on a mismatch),
    /// for a client to pin as `ssh.host_key`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_host_key: Option<HostKeyInfo>,
}

impl TestConnectionResp {
    fn failed(driver: &str, stage: Stage, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            driver: driver.to_string(),
            latency_ms: 0,
            server_version: None,
            message: Some(message.into()),
            stage: Some(stage),
            ssh_host_key: None,
        }
    }
}

/// The candidate entry as JSON, from either request shape.
fn candidate(req: &TestConnectionReq) -> Result<Value, TestConnectionResp> {
    if let Some(connection) = &req.connection {
        return Ok(connection.clone());
    }
    let Some(url) = &req.url else {
        return Err(TestConnectionResp::failed(
            "unknown",
            Stage::Config,
            "send `connection` (the entry to test) or `url`",
        ));
    };
    if detect_driver(url).is_none() {
        return Err(TestConnectionResp::failed(
            "unknown",
            Stage::Config,
            "unknown url scheme — expected sqlite:, postgres:// / postgresql://, or mysql://",
        ));
    }
    let mut entry = serde_json::json!({ "url": url });
    if let Some(tls) = &req.tls {
        entry["tls"] = serde_json::to_value(tls).unwrap_or(Value::Null);
    }
    Ok(entry)
}

pub async fn handle(
    state: &AppState,
    req: TestConnectionReq,
) -> Result<TestConnectionResp, crate::error::DbError> {
    let started = Instant::now();
    let budget = Duration::from_millis(
        req.timeout_ms
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(1, MAX_TIMEOUT_MS),
    );
    let mut entry = match candidate(&req) {
        Ok(entry) => entry,
        Err(resp) => return Ok(resp),
    };
    let label = req.name.clone().unwrap_or_else(|| "candidate".into());
    let driver_hint = entry
        .get("driver")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            entry
                .get("url")
                .and_then(Value::as_str)
                .and_then(detect_driver)
                .map(|d| driver_system(d).to_string())
        })
        .unwrap_or_else(|| "unknown".into());

    if crate::redaction::contains_redacted(&entry) {
        let stored = match &req.name {
            Some(name) => match super::connections::stored_entry(state, name).await {
                Ok(stored) => stored,
                Err(_) => None,
            },
            None => None,
        };
        if let Err(e) = crate::redaction::restore_entry(&mut entry, stored.as_ref()) {
            let hint = if req.name.is_none() {
                " (pass `name` to use the stored connection's values)"
            } else {
                ""
            };
            return Ok(TestConnectionResp::failed(
                &driver_hint,
                Stage::Config,
                format!("{e}{hint}"),
            ));
        }
    }

    let db = match super::connections::parse_entry(&label, &entry) {
        Ok(db) => db,
        Err(e) => return Ok(TestConnectionResp::failed(&driver_hint, Stage::Config, e)),
    };
    let driver = driver_system(db.driver).to_string();

    let outcome = tokio::time::timeout(budget, run_probe(state, &label, &db)).await;
    let latency_ms = started.elapsed().as_millis() as u64;
    let url = db.url.clone().unwrap_or_default();
    Ok(match outcome {
        Ok(Ok((version, ssh_host_key))) => TestConnectionResp {
            ok: true,
            driver,
            latency_ms,
            server_version: Some(version),
            message: None,
            stage: None,
            ssh_host_key,
        },
        Ok(Err(failure)) => TestConnectionResp {
            ok: false,
            driver,
            latency_ms,
            server_version: None,
            message: Some(scrub(&failure.error.message, &url, &failure.credentials)),
            stage: Some(failure.error.stage),
            ssh_host_key: failure.host_key,
        },
        Err(_) => TestConnectionResp {
            ok: false,
            driver,
            latency_ms,
            server_version: None,
            message: Some(format!("timed out after {}ms", budget.as_millis())),
            stage: Some(if db.ssh.is_some() {
                Stage::Ssh
            } else {
                Stage::Tcp
            }),
            ssh_host_key: None,
        },
    })
}

struct ProbeFailure {
    error: ConnectError,
    host_key: Option<HostKeyInfo>,
    /// Values to scrub from the message.
    credentials: Vec<String>,
}

async fn run_probe(
    state: &AppState,
    label: &str,
    db: &crate::config::DatabaseConfig,
) -> Result<(String, Option<HostKeyInfo>), ProbeFailure> {
    let fail = |error, host_key, credentials| ProbeFailure {
        error,
        host_key,
        credentials,
    };
    state
        .runtime
        .secrets
        .ensure(connector::references(db))
        .await;
    let material = Material::resolve(db, |key| state.runtime.secrets.lookup(key))
        .map_err(|e| fail(e, None, Vec::new()))?;
    let credentials = material.credentials();
    let connector =
        Connector::build(label, db, &material).map_err(|e| fail(e, None, credentials.clone()))?;
    let host_key = || connector.tunnel().and_then(|t| t.host_key());
    if let Err(e) = connector.check_tunnel().await {
        return Err(fail(e, host_key(), credentials));
    }
    let probe_started = tokio::time::Instant::now();
    match probe(&connector).await {
        Ok(version) => Ok((version, host_key())),
        Err(mut e) => {
            // Through a tunnel the driver only sees a closed socket; the
            // tunnel knows whether the far end was unreachable.
            if let Some(tunnel_error) = connector
                .tunnel()
                .and_then(|t| t.error_since(probe_started))
            {
                e = ConnectError::new(
                    match tunnel_error.kind {
                        crate::ssh::TunnelErrorKind::Forward => Stage::Tcp,
                        _ => Stage::Ssh,
                    },
                    tunnel_error.message,
                );
            }
            Err(fail(e, host_key(), credentials))
        }
    }
}

async fn probe(connector: &Connector) -> Result<String, ConnectError> {
    if let Some((config, tls)) = connector.pg() {
        return probe_postgres(config, tls).await;
    }
    if connector.mysql().is_some() {
        return probe_mysql(connector).await;
    }
    let url = connector.sqlite_url().unwrap_or_default().to_string();
    probe_sqlite(&url)
        .await
        .map_err(|e| ConnectError::new(Stage::Database, e))
}

async fn probe_postgres(
    config: &tokio_postgres::Config,
    tls: Option<&tokio_postgres_rustls::MakeRustlsConnect>,
) -> Result<String, ConnectError> {
    async fn ping(client: tokio_postgres::Client) -> Result<String, ConnectError> {
        let row = client
            .query_one("SELECT version()", &[])
            .await
            .map_err(|e| pg_error(&e))?;
        Ok(row.get::<_, String>(0))
    }
    match tls {
        Some(connector) => {
            let (client, conn) = config
                .connect(connector.clone())
                .await
                .map_err(|e| pg_error(&e))?;
            tokio::spawn(async move {
                let _ = conn.await;
            });
            ping(client).await
        }
        None => {
            let (client, conn) = config
                .connect(tokio_postgres::NoTls)
                .await
                .map_err(|e| pg_error(&e))?;
            tokio::spawn(async move {
                let _ = conn.await;
            });
            ping(client).await
        }
    }
}

fn pg_error(e: &tokio_postgres::Error) -> ConnectError {
    let stage = match e.code().map(|c| c.code()) {
        Some(code) if code == "3D000" => Stage::Database,
        _ => match crate::pool::postgres::classify_pg_error(e) {
            "tls" => Stage::Tls,
            "auth" => Stage::Auth,
            "network" => Stage::Tcp,
            _ => Stage::Database,
        },
    };
    // The Display of a connect error is terse ("error connecting to
    // server: …"); keep its source chain, which says why.
    let mut message = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        let text = s.to_string();
        if !message.contains(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        source = s.source();
    }
    if message.contains("server does not support TLS") {
        message.push_str(" (tls.mode prefer or disable connects without it)");
    }
    ConnectError::new(stage, message)
}

async fn probe_mysql(connector: &Connector) -> Result<String, ConnectError> {
    use mysql_async::prelude::Queryable as _;
    let mut conn = connector
        .mysql_connect()
        .await
        .map_err(|e| mysql_error(&e))?;
    let version: Option<String> = conn
        .query_first("SELECT VERSION()")
        .await
        .map_err(|e| mysql_error(&e))?;
    let _ = conn.disconnect().await;
    version.ok_or_else(|| ConnectError::new(Stage::Database, "server returned no version row"))
}

fn mysql_error(e: &mysql_async::Error) -> ConnectError {
    use mysql_async::{DriverError, Error, IoError};
    let (stage, extra) = match e {
        Error::Io(IoError::Tls(_)) => (Stage::Tls, ""),
        Error::Io(IoError::Io(_)) => (Stage::Tcp, ""),
        Error::Driver(DriverError::NoClientSslFlagFromServer) => (
            Stage::Tls,
            " (the server offers no TLS; tls.mode prefer or disable connects without it)",
        ),
        Error::Driver(DriverError::ConnectionClosed) => (Stage::Tcp, ""),
        Error::Server(s) if matches!(s.code, 1045 | 1129 | 1130 | 1156 | 1251 | 1698) => {
            (Stage::Auth, "")
        }
        Error::Server(s) if s.code == 1049 => (Stage::Database, ""),
        _ => (Stage::Database, ""),
    };
    ConnectError::new(stage, format!("{e}{extra}"))
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

/// Strip every credential the probe used out of an error text — drivers
/// quote the connection string (and so its password) in some failure modes.
fn scrub(message: &str, url: &str, credentials: &[String]) -> String {
    let mut out = scrub_credentials(message, url);
    for secret in credentials {
        if secret.len() >= 3 {
            out = out.replace(secret.as_str(), "***");
        }
    }
    out
}

/// Strip the url's username/password out of an error text.
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
            url: Some(url.into()),
            ..Default::default()
        }
    }

    fn st() -> AppState {
        AppState {
            pools: Default::default(),
            config: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::config::WorkerConfig::default(),
            )),
            handles: std::sync::Arc::new(crate::handle::HandleRegistry::new()),
            transactions: crate::transaction::TxRegistry::new(),
            log: iii_helpers::observability::Logger::new(),
            row_changes: None,
            runtime: Default::default(),
        }
    }

    async fn handle(req: TestConnectionReq) -> Result<TestConnectionResp, crate::error::DbError> {
        super::handle(&st(), req).await
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
