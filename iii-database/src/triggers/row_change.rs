//! row-change trigger — Postgres logical replication via pgoutput.
//!
//! v1.0 scope:
//!   - Create a publication for the configured tables (idempotent, real impl).
//!   - Create a logical replication slot with output plugin `pgoutput` (idempotent, real impl).
//!   - Stream events; decode INSERT/UPDATE/DELETE for the configured tables.
//!   - Advance LSN only on caller `ack: true`.
//!
//! IMPLEMENTATION STATUS: setup is complete (publication + slot creation are
//! tested). The streaming decode loop is a STUB — see `run_loop` and
//! `connect_replication`. The decoder belongs in this same module and consumes
//! `postgres_protocol::message::backend::LogicalReplicationMessage` from a
//! `client.copy_both_simple()` stream over a *replication-mode* connection.
//!
//! The currently-pinned `tokio-postgres = "0.7.17"` does not expose the
//! replication API (the unreleased master branch on github does). When that
//! API ships, replace `connect_replication`'s stub with a real implementation
//! and fill in `run_loop`. Reference:
//! https://github.com/sfackler/rust-postgres/blob/master/tokio-postgres/tests/test/replication.rs

// Pre-staged setup code (`connect_for_setup`, `ensure_publication_and_slot`,
// `RowChangeConfig::validate`) is exercised by gated integration tests but
// has no production caller until the streaming decode loop ships. Allow
// dead code at the module level so the lib build is clean; the items will
// become live when `run_loop` is wired up.
#![allow(dead_code)]

use crate::error::DbError;
use serde::{Deserialize, Serialize};
use tokio_postgres::{Client, Config, NoTls};

#[derive(Debug, Clone, Deserialize)]
pub struct RowChangeConfig {
    pub trigger_id: String,
    #[serde(rename = "db")]
    pub db_name: String,
    #[serde(default = "default_schema")]
    pub schema: String,
    pub tables: Vec<String>,
    #[serde(default)]
    pub slot_name: Option<String>,
    #[serde(default)]
    pub publication_name: Option<String>,
}

fn default_schema() -> String {
    "public".into()
}

#[derive(Debug, Clone, Serialize)]
pub struct RowChangeEvent {
    pub db: String,
    pub schema: String,
    pub table: String,
    pub op: String, // "INSERT" | "UPDATE" | "DELETE"
    pub new: Option<serde_json::Value>,
    pub old: Option<serde_json::Value>,
    pub committed_at: chrono::DateTime<chrono::Utc>,
    pub lsn: String,
}

pub fn derive_names(cfg: &RowChangeConfig) -> (String, String) {
    // Sanitize trigger_id for use in a Postgres identifier (slot/publication
    // names accept `[A-Za-z0-9_]` only). Distinct trigger_ids can sanitize to
    // the same form (`Orders.v1` and `orders-v1` both become `orders_v1`); if
    // we used the sanitized form alone, two registrations would silently
    // share one replication slot and consume each other's events. Append an
    // FNV-1a-32 hash of the *original* trigger_id so distinct inputs always
    // produce distinct outputs while collision-free identifiers stay readable.
    //
    // Truncate the sanitized prefix at 40 chars so the final name fits inside
    // Postgres' 63-byte slot_name limit: `iii_slot_` (9) + sanitized (≤40)
    // + `_` + 8 hex chars = 58.
    let sanitized: String = cfg
        .trigger_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    let h = fnv1a_32(cfg.trigger_id.as_bytes());
    let slot = cfg
        .slot_name
        .clone()
        .unwrap_or_else(|| format!("iii_slot_{sanitized}_{h:08x}"));
    let pubname = cfg
        .publication_name
        .clone()
        .unwrap_or_else(|| format!("iii_pub_{sanitized}_{h:08x}"));
    (slot, pubname)
}

fn fnv1a_32(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for &b in bytes {
        hash ^= b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Open a normal (non-replication-mode) connection. Suitable for setup
/// (publication + slot creation). NOT suitable for the streaming decode loop.
pub async fn connect_for_setup(
    url: &str,
    tls_cfg: &crate::config::TlsConfig,
) -> Result<Client, DbError> {
    // Don't echo the underlying parse error — tokio_postgres's error message
    // can include the offending URL, which would leak any embedded password
    // into logs. Surface a generic message instead.
    let cfg: Config = url
        .parse()
        .map_err(|_: tokio_postgres::Error| DbError::ConfigError {
            message: "postgres url parse failed; check the configured url".into(),
        })?;
    // Same connector as `pool::postgres`. `disable` falls back to NoTls.
    let client_and_conn = match crate::pool::tls::make_pg_connector(tls_cfg)? {
        Some(connector) => cfg
            .connect(connector)
            .await
            .map(|(c, conn)| (c, futures_util::future::Either::Left(conn))),
        None => cfg
            .connect(NoTls)
            .await
            .map(|(c, conn)| (c, futures_util::future::Either::Right(conn))),
    }
    .map_err(crate::driver::postgres::map_err)?;
    let (client, conn) = client_and_conn;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::error!(error = ?e, "row-change setup connection terminated");
        }
    });
    Ok(client)
}

/// STUB: open a replication-mode connection. The crates.io `tokio-postgres
/// = 0.7.17` doesn't expose the replication API. When upstream cuts a
/// release with `Config::replication_mode`, replace this stub.
#[allow(dead_code)]
pub async fn connect_replication(_url: &str) -> Result<Client, DbError> {
    Err(DbError::Unsupported {
        op: "connect_replication".into(),
        driver: "postgres (pending tokio-postgres replication API release)".into(),
    })
}

impl RowChangeConfig {
    /// Validate operator-supplied identifiers that flow into `format!()`
    /// SQL strings: `slot_name`, `publication_name`, `schema`, and each
    /// element of `tables` (split on `.` for qualified names). Validation
    /// uses the strict ASCII identifier rule from `crate::config`.
    pub fn validate(&self) -> Result<(), DbError> {
        let cfg_err = |e: String| DbError::ConfigError { message: e };
        crate::config::validate_sql_identifier(&self.schema)
            .map_err(|e| cfg_err(format!("row-change schema: {e}")))?;
        if let Some(slot) = &self.slot_name {
            crate::config::validate_sql_identifier(slot)
                .map_err(|e| cfg_err(format!("row-change slot_name: {e}")))?;
        }
        if let Some(pubname) = &self.publication_name {
            crate::config::validate_sql_identifier(pubname)
                .map_err(|e| cfg_err(format!("row-change publication_name: {e}")))?;
        }
        for t in &self.tables {
            // Qualified names allowed (`schema.table`); validate each part.
            for part in t.split('.') {
                crate::config::validate_sql_identifier(part)
                    .map_err(|e| cfg_err(format!("row-change tables entry `{t}`: {e}")))?;
            }
        }
        Ok(())
    }
}

pub async fn ensure_publication_and_slot(
    client: &mut Client,
    cfg: &RowChangeConfig,
) -> Result<(), DbError> {
    cfg.validate()?;
    let (slot, pubname) = derive_names(cfg);
    let pub_exists = client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_publication WHERE pubname = $1) AS ex",
            &[&pubname],
        )
        .await
        .map_err(crate::driver::postgres::map_err)?
        .get::<_, bool>("ex");

    if !pub_exists {
        let qualified: Vec<String> = cfg
            .tables
            .iter()
            .map(|t| {
                if t.contains('.') {
                    t.clone()
                } else {
                    format!("{}.{t}", cfg.schema)
                }
            })
            .collect();
        let stmt = format!(
            "CREATE PUBLICATION {pubname} FOR TABLE {}",
            qualified.join(", ")
        );
        client
            .simple_query(&stmt)
            .await
            .map_err(crate::driver::postgres::map_err)?;
    }

    let slot_exists = client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_replication_slots WHERE slot_name = $1) AS ex",
            &[&slot],
        )
        .await
        .map_err(crate::driver::postgres::map_err)?
        .get::<_, bool>("ex");

    if !slot_exists {
        let stmt =
            format!("SELECT * FROM pg_create_logical_replication_slot('{slot}', 'pgoutput')");
        match client.simple_query(&stmt).await {
            Ok(_) => {
                tracing::info!(slot = %slot, publication = %pubname, "created replication artifacts");
            }
            Err(e) => {
                if e.to_string().contains("already exists") {
                    return Err(DbError::ReplicationSlotExists { slot });
                } else {
                    return Err(crate::driver::postgres::map_err(e));
                }
            }
        }
    }
    Ok(())
}

#[async_trait::async_trait]
pub trait QueryPollLikeDispatcher: Send + Sync {
    async fn dispatch(&self, ev: RowChangeEvent) -> Result<bool, DbError>;
}

/// STUB: streaming decoder loop. Requires a replication-mode `Client` from
/// `connect_replication` (also currently a stub). When the upstream replication
/// API ships, fill this in using `client.copy_both_simple` and
/// `postgres_protocol::message::backend::LogicalReplicationMessage::parse`.
pub async fn run_loop(
    _client: Client,
    _cfg: RowChangeConfig,
    _dispatch: std::sync::Arc<dyn QueryPollLikeDispatcher>,
) -> Result<(), DbError> {
    tracing::warn!(
        "row-change run_loop is a stub — pgoutput decode requires the unreleased \
         tokio-postgres replication API. See module-level docstring for status."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url() -> Option<String> {
        std::env::var("TEST_POSTGRES_URL").ok()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn slot_and_publication_names_use_sanitized_trigger_id() {
        let cfg = RowChangeConfig {
            trigger_id: "my:trigger.id-with/funky chars".into(),
            db_name: "primary".into(),
            schema: "public".into(),
            tables: vec!["orders".into()],
            slot_name: None,
            publication_name: None,
        };
        let (slot, pubname) = derive_names(&cfg);
        assert!(slot.starts_with("iii_slot_"));
        assert!(pubname.starts_with("iii_pub_"));
        // No characters that aren't [a-z0-9_].
        assert!(slot
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'));
        assert!(pubname
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'));
        // 63-byte slot_name limit on postgres.
        assert!(slot.len() <= 63, "slot name too long: {} bytes", slot.len());
        assert!(
            pubname.len() <= 63,
            "publication name too long: {} bytes",
            pubname.len()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn distinct_trigger_ids_produce_distinct_slot_names() {
        // Regression: prior versions sanitized trigger_id to lower-alnum-with-
        // underscores; `Orders.v1`, `orders-v1`, `orders_v1`, `orders v1` all
        // collapsed to `orders_v1` and silently shared one replication slot,
        // letting one trigger consume another's events. Distinct trigger_ids
        // must produce distinct slot/publication names.
        let mk = |id: &str| RowChangeConfig {
            trigger_id: id.into(),
            db_name: "primary".into(),
            schema: "public".into(),
            tables: vec!["orders".into()],
            slot_name: None,
            publication_name: None,
        };
        let ids = [
            "Orders.v1",
            "orders-v1",
            "orders_v1",
            "orders v1",
            "ORDERS_V1",
        ];
        let mut slots = std::collections::HashSet::new();
        let mut pubs = std::collections::HashSet::new();
        for id in ids {
            let (s, p) = derive_names(&mk(id));
            assert!(slots.insert(s.clone()), "slot collision on `{id}`: {s}");
            assert!(pubs.insert(p.clone()), "pub collision on `{id}`: {p}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn long_trigger_id_truncates_to_postgres_limit() {
        // Even a pathological trigger_id must produce a valid postgres slot
        // name (≤ 63 bytes). Hash suffix preserves uniqueness across the
        // truncation boundary.
        let cfg = RowChangeConfig {
            trigger_id: "a".repeat(100),
            db_name: "primary".into(),
            schema: "public".into(),
            tables: vec!["orders".into()],
            slot_name: None,
            publication_name: None,
        };
        let (slot, pubname) = derive_names(&cfg);
        assert!(slot.len() <= 63, "slot too long: {}", slot.len());
        assert!(
            pubname.len() <= 63,
            "publication too long: {}",
            pubname.len()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn explicit_slot_and_publication_names_bypass_derivation() {
        let cfg = RowChangeConfig {
            trigger_id: "anything".into(),
            db_name: "primary".into(),
            schema: "public".into(),
            tables: vec!["orders".into()],
            slot_name: Some("custom_slot".into()),
            publication_name: Some("custom_pub".into()),
        };
        let (slot, pubname) = derive_names(&cfg);
        assert_eq!(slot, "custom_slot");
        assert_eq!(pubname, "custom_pub");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn create_slot_and_publication_idempotent() {
        let Some(u) = url() else { return };
        let cfg = RowChangeConfig {
            trigger_id: "test_idem".into(),
            db_name: "primary".into(),
            schema: "public".into(),
            tables: vec!["public.test_idem_t".into()],
            slot_name: Some("iii_slot_test_idem".into()),
            publication_name: Some("iii_pub_test_idem".into()),
        };
        let tls = crate::config::TlsConfig {
            mode: crate::config::TlsMode::Disable,
            ca_cert: None,
        };
        let mut client = connect_for_setup(&u, &tls).await.unwrap();
        // Cleanup from prior run
        let _ = client
            .simple_query("SELECT pg_drop_replication_slot('iii_slot_test_idem')")
            .await;
        let _ = client
            .simple_query("DROP PUBLICATION IF EXISTS iii_pub_test_idem")
            .await;
        let _ = client
            .simple_query("DROP TABLE IF EXISTS public.test_idem_t")
            .await;
        client
            .simple_query("CREATE TABLE public.test_idem_t (id SERIAL PRIMARY KEY, n INT)")
            .await
            .unwrap();

        ensure_publication_and_slot(&mut client, &cfg)
            .await
            .unwrap();
        // Running again is idempotent.
        ensure_publication_and_slot(&mut client, &cfg)
            .await
            .unwrap();
    }
}
