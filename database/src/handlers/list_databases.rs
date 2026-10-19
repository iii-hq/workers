//! `database::listDatabases` — config details for every registered database.
//! Config-only: no health checks, no live pool stats. Credentials are
//! scrubbed from the connection URL before it leaves the process; secret
//! values, passwords and key material are never part of the reply.

use super::AppState;
use crate::config::{DisplayConfig, TlsMode};
use crate::connector::Stage;
use crate::transaction::driver_system;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListDatabasesReq {}

/// Pool settings echoed back from config (no live stats).
#[derive(Debug, Serialize, JsonSchema)]
pub struct PoolInfo {
    pub max: u32,
    pub idle_timeout_ms: u64,
    pub acquire_timeout_ms: u64,
}

/// TLS settings. Certificates and keys are reported as presence booleans
/// only — never the path, which would leak filesystem layout.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TlsInfo {
    pub mode: TlsMode,
    pub ca_cert_present: bool,
    /// `client_cert` + `client_key` set (mutual TLS).
    pub client_cert_present: bool,
    pub trust_native: bool,
}

/// The SSH tunnel's server (no credentials).
#[derive(Debug, Serialize, JsonSchema)]
pub struct SshInfo {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// Whether `host_key` pins the server's key.
    pub host_key_pinned: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DatabaseInfo {
    /// Logical key (e.g. "primary").
    pub name: String,
    /// "postgres" | "mysql" | "sqlite".
    pub driver: String,
    /// Connection URL with credentials redacted (assembled from the fields
    /// for the structured form).
    pub url: String,
    /// Server host (postgres/mysql), from either form.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// User name (postgres/mysql), when configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Database (schema) name; the file path for sqlite.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<DisplayConfig>,
    /// Folder of `.sql` query files on the engine host.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh: Option<SshInfo>,
    pub pool: PoolInfo,
    pub tls: TlsInfo,
    /// Whether any value is a `secret://` / `env://` reference.
    pub uses_secrets: bool,
    /// Why the connection has no pool (an unresolvable reference, an SSH
    /// failure, an unreadable certificate…). Absent when it is usable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The step `error` happened at: `config`, `secrets`, `ssh`, `tcp`, `tls`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_stage: Option<Stage>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListDatabasesResp {
    pub databases: Vec<DatabaseInfo>,
    pub count: usize,
}

pub async fn handle(state: &AppState, _req: ListDatabasesReq) -> Result<ListDatabasesResp, String> {
    let cfg = state.config.read().await;
    let errors = state.runtime.errors();
    let mut databases: Vec<DatabaseInfo> = cfg
        .databases
        .iter()
        .map(|(name, db)| {
            let endpoint = db.server_endpoint();
            let user = match &db.url {
                Some(url) => url::Url::parse(url)
                    .ok()
                    .map(|u| u.username().to_string())
                    .filter(|u| !u.is_empty()),
                None => db.user.clone(),
            };
            let error = errors.get(name);
            DatabaseInfo {
                name: name.clone(),
                driver: driver_system(db.driver).to_string(),
                url: db.display_url(),
                host: endpoint.as_ref().map(|(h, _)| h.clone()),
                port: endpoint.map(|(_, p)| p),
                user,
                database: db.database_name().or_else(|| {
                    (db.driver == crate::config::DriverKind::Sqlite)
                        .then(|| {
                            db.url
                                .as_deref()
                                .and_then(|u| u.strip_prefix("sqlite:"))
                                .map(str::to_string)
                        })
                        .flatten()
                }),
                display: db.display.clone(),
                folder: db.folder.clone(),
                ssh: db.ssh.as_ref().map(|ssh| SshInfo {
                    host: ssh.host.clone(),
                    port: ssh.port,
                    user: ssh.user.clone(),
                    host_key_pinned: ssh.host_key.is_some(),
                }),
                pool: PoolInfo {
                    max: db.pool.max,
                    idle_timeout_ms: db.pool.idle_timeout_ms,
                    acquire_timeout_ms: db.pool.acquire_timeout_ms,
                },
                tls: TlsInfo {
                    mode: db.tls.mode,
                    ca_cert_present: db.tls.ca_cert.is_some(),
                    client_cert_present: db.tls.client_cert.is_some()
                        && db.tls.client_key.is_some(),
                    trust_native: db.tls.trust_native,
                },
                uses_secrets: db.uses_secrets(),
                error: error.map(|e| e.message.clone()),
                error_stage: error.map(|e| e.stage),
            }
        })
        .collect();
    // HashMap iteration order is nondeterministic; sort so output is stable.
    databases.sort_by(|a, b| a.name.cmp(&b.name));
    let count = databases.len();
    Ok(ListDatabasesResp { databases, count })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkerConfig;
    use crate::handle::HandleRegistry;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn state_from_yaml(yaml: &str) -> AppState {
        let cfg = WorkerConfig::from_yaml(yaml).unwrap();
        AppState {
            // The handler reads only `config`; pools are never touched.
            pools: Arc::new(RwLock::new(HashMap::new())),
            config: Arc::new(RwLock::new(cfg)),
            handles: Arc::new(HandleRegistry::new()),
            transactions: crate::transaction::TxRegistry::new(),
            log: iii_helpers::observability::Logger::new(),
            row_changes: None,
            runtime: Default::default(),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn returns_sqlite_primary_with_config_defaults() {
        // Arrange
        let st = state_from_yaml("databases:\n  primary:\n    url: \"sqlite::memory:\"\n");

        // Act
        let resp = handle(&st, ListDatabasesReq::default()).await.unwrap();

        // Assert
        assert_eq!(resp.count, 1);
        let db = &resp.databases[0];
        assert_eq!(db.name, "primary");
        assert_eq!(db.driver, "sqlite");
        assert_eq!(db.pool.max, 10);
        assert_eq!(db.pool.idle_timeout_ms, 30_000);
        assert_eq!(db.pool.acquire_timeout_ms, 5_000);
        assert_eq!(db.tls.mode, TlsMode::Require);
        assert!(!db.tls.ca_cert_present);
        assert!(db.tls.trust_native);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn redacts_postgres_password() {
        // Arrange
        let st = state_from_yaml(
            "databases:\n  main:\n    url: \"postgres://user:secret@host:5432/db\"\n",
        );

        // Act
        let resp = handle(&st, ListDatabasesReq::default()).await.unwrap();

        // Assert
        let db = &resp.databases[0];
        assert_eq!(db.driver, "postgres");
        assert_eq!(db.url, "postgres://***@host:5432/db");
        assert!(!db.url.contains("secret"));
        assert!(!db.url.contains("user:"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn redacts_mysql_password() {
        // Arrange
        let st = state_from_yaml(
            "databases:\n  main:\n    url: \"mysql://admin:pw@127.0.0.1:3306/test\"\n",
        );

        // Act
        let resp = handle(&st, ListDatabasesReq::default()).await.unwrap();

        // Assert
        let db = &resp.databases[0];
        assert_eq!(db.driver, "mysql");
        assert_eq!(db.url, "mysql://***@127.0.0.1:3306/test");
        assert!(!db.url.contains("pw@"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sqlite_url_passes_through_without_credentials() {
        // Arrange
        let st = state_from_yaml("databases:\n  mem:\n    url: \"sqlite::memory:\"\n");

        // Act
        let resp = handle(&st, ListDatabasesReq::default()).await.unwrap();

        // Assert
        assert_eq!(resp.databases[0].url, "sqlite::memory:");
        assert_eq!(resp.databases[0].driver, "sqlite");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sorts_databases_by_name_and_counts() {
        // Arrange
        let st = state_from_yaml(
            "databases:\n  zeta:\n    url: \"sqlite::memory:\"\n  alpha:\n    url: \"sqlite::memory:\"\n  mid:\n    url: \"sqlite::memory:\"\n",
        );

        // Act
        let resp = handle(&st, ListDatabasesReq::default()).await.unwrap();

        // Assert
        assert_eq!(resp.count, 3);
        let names: Vec<&str> = resp.databases.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["alpha", "mid", "zeta"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reports_tls_overrides_without_leaking_cert_path() {
        // Arrange
        let st = state_from_yaml(
            "databases:\n  main:\n    url: \"postgres://u@host/db\"\n    tls:\n      mode: verify-full\n      ca_cert: /etc/ssl/private-ca.pem\n      trust_native: false\n",
        );

        // Act
        let resp = handle(&st, ListDatabasesReq::default()).await.unwrap();

        // Assert
        let json = serde_json::to_value(&resp).unwrap();
        let db = &json["databases"][0];
        assert_eq!(db["tls"]["mode"], "verify-full");
        assert_eq!(db["tls"]["ca_cert_present"], true);
        assert_eq!(db["tls"]["trust_native"], false);
        assert!(!json.to_string().contains("private-ca.pem"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn response_envelope_matches_sibling_convention() {
        // Arrange
        let st = state_from_yaml("databases:\n  primary:\n    url: \"sqlite::memory:\"\n");

        // Act
        let resp = handle(&st, ListDatabasesReq::default()).await.unwrap();

        // Assert
        let json = serde_json::to_value(&resp).unwrap();
        let mut keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["count", "databases"]);
        assert!(json["databases"].is_array());
        assert!(json["count"].is_number());
    }
}
