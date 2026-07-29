//! `database::listDatabases` — config details for every registered database.
//! Config-only: no health checks, no live pool stats. Credentials are
//! scrubbed from the connection URL before it leaves the process.

use super::AppState;
use crate::config::{redact_url, TlsMode};
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

/// TLS settings. `ca_cert` is reported as a presence boolean only — never
/// the path, which would leak filesystem layout.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TlsInfo {
    pub mode: TlsMode,
    pub ca_cert_present: bool,
    pub trust_native: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DatabaseInfo {
    /// Logical key (e.g. "primary").
    pub name: String,
    /// "postgres" | "mysql" | "sqlite".
    pub driver: String,
    /// Connection URL with credentials redacted.
    pub url: String,
    pub pool: PoolInfo,
    pub tls: TlsInfo,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListDatabasesResp {
    pub databases: Vec<DatabaseInfo>,
    pub count: usize,
}

pub async fn handle(state: &AppState, _req: ListDatabasesReq) -> Result<ListDatabasesResp, String> {
    let cfg = state.config.read().await;
    let mut databases: Vec<DatabaseInfo> = cfg
        .databases
        .iter()
        .map(|(name, db)| DatabaseInfo {
            name: name.clone(),
            driver: driver_system(db.driver).to_string(),
            url: redact_url(&db.url),
            pool: PoolInfo {
                max: db.pool.max,
                idle_timeout_ms: db.pool.idle_timeout_ms,
                acquire_timeout_ms: db.pool.acquire_timeout_ms,
            },
            tls: TlsInfo {
                mode: db.tls.mode,
                ca_cert_present: db.tls.ca_cert.is_some(),
                trust_native: db.tls.trust_native,
            },
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
