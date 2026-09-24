//! The `database::row-changed` TriggerHandler.
//!
//! Thin by design: the engine hands a registration here, this validates the
//! config and files it in the [`RowChangeBus`]; the mutating handlers do the
//! emitting. Registration fails loudly for a database that is not configured —
//! a binding on a typo'd handle would otherwise sit there listening to nothing.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};

use super::bus::{RowChangeBus, RowChangedConfig};
use super::native;
use crate::config::{CaptureMode, WorkerConfig};
use crate::pool::Pool;

pub struct RowChangedHandler {
    pub bus: Arc<RowChangeBus>,
    /// Live configuration, swapped together with the pools on hot reload.
    pub config: Arc<tokio::sync::RwLock<WorkerConfig>>,
    /// Live pools — a `capture: native` binding installs its database
    /// triggers through the bound database's pool at registration time.
    pub pools: Arc<tokio::sync::RwLock<HashMap<String, Pool>>>,
}

fn config_error(message: String) -> Error {
    Error::Handler(serde_json::json!({ "code": "CONFIG_ERROR", "message": message }).to_string())
}

/// Pick the guidance appended to a postgres trigger-install failure. The
/// hint must match the actual failure: dressing a `does not exist` error in
/// privilege advice sent a real operator chasing grants when the problem
/// was table-name casing (native bindings quote the name verbatim, and
/// quoted postgres identifiers are case-sensitive).
fn pg_install_hint(error_text: &str, table: &str) -> String {
    if error_text.contains("does not exist") {
        format!(
            ". Note: the binding's table name is quoted verbatim into DDL and \
             quoted postgres identifiers are case-sensitive — `{table}` must \
             match the table's actual spelling"
        )
    } else if error_text.contains("permission denied") || error_text.contains("must be owner") {
        ". The configured role needs TRIGGER privilege on the table (or ownership) \
         and CREATE on its schema"
            .to_string()
    } else {
        String::new()
    }
}

#[async_trait]
impl TriggerHandler for RowChangedHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let cfg: RowChangedConfig = serde_json::from_value(config.config.clone())
            .map_err(|e| config_error(format!("row-changed config: {e}")))?;

        let live = self.config.read().await;
        let Some(db_cfg) = live.databases.get(&cfg.db) else {
            let mut known = live.databases.keys().cloned().collect::<Vec<_>>();
            known.sort();
            return Err(config_error(format!(
                "unknown db `{}`; available: [{}]",
                cfg.db,
                known.join(", ")
            )));
        };
        let native = db_cfg.capture == CaptureMode::Native;
        drop(live);

        if native {
            self.install_native_triggers(&cfg).await?;
        }

        let table = cfg.table.clone();
        self.bus.register(
            config.id.clone(),
            config.function_id.clone(),
            config.metadata.clone(),
            cfg,
        );
        tracing::info!(
            instance = %config.id,
            function = %config.function_id,
            table = ?table,
            "row-changed trigger registered"
        );
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        // ponytail: native-capture triggers stay installed on unregister —
        // an orphan pg_notify per write statement is near-free and idempotent
        // to reinstall; add DDL teardown when someone actually needs it.
        self.bus.unregister(&config.id);
        tracing::info!(instance = %config.id, "row-changed trigger unregistered");
        Ok(())
    }
}

impl RowChangedHandler {
    /// Install the NOTIFY function and per-table triggers for a native
    /// binding. Fails loudly — a binding whose DDL did not land would sit
    /// there hearing nothing, which is the failure mode this worker refuses.
    async fn install_native_triggers(&self, cfg: &RowChangedConfig) -> Result<(), Error> {
        let Some(table) = cfg.table.as_deref() else {
            return Err(config_error(format!(
                "db `{}` uses `capture: native`, which requires this binding to \
                 name a `table` — per-table database triggers are what make \
                 external writes visible",
                cfg.db
            )));
        };
        let pool = self.pools.read().await.get(&cfg.db).cloned();
        match pool {
            Some(Pool::Postgres(pg)) => {
                let sql = native::install_sql(table).map_err(config_error)?;
                let mut client = pg.acquire().await.map_err(|e| {
                    config_error(format!("db `{}`: acquiring connection: {e}", cfg.db))
                })?;
                let install_error = |error: tokio_postgres::Error| {
                    let text = error
                        .as_db_error()
                        .map(|error| error.message().to_string())
                        .unwrap_or_else(|| error.to_string());
                    config_error(format!(
                        "installing native capture triggers on `{table}`: {text}{}",
                        pg_install_hint(&text, table)
                    ))
                };
                // Every binding replaces the SAME notify function, including
                // bindings for different tables. Concurrent CREATE OR REPLACE
                // can fail with `tuple concurrently updated`. A DB-scoped lock
                // coordinates separate pools/processes too (not just this bus).
                // Acquire in its own statement so READ COMMITTED refreshes the
                // DDL snapshot after a competing installer has committed.
                let transaction = client
                    .build_transaction()
                    .isolation_level(tokio_postgres::IsolationLevel::ReadCommitted)
                    .start()
                    .await
                    .map_err(install_error)?;
                // Stable two-int advisory namespace reserved for iii native
                // row-capture installation. No user-controlled lock key or SQL.
                transaction
                    .batch_execute("SELECT pg_advisory_xact_lock(1768515886, 1919907683)")
                    .await
                    .map_err(install_error)?;
                transaction
                    .batch_execute(&sql)
                    .await
                    .map_err(install_error)?;
                transaction.commit().await.map_err(install_error)?;
                // On any error, transaction drop rolls back both the DDL and
                // the lock before the connection can return to the pool.
            }
            Some(Pool::Sqlite(sq)) => {
                let conn = sq.acquire().await.map_err(|e| {
                    config_error(format!("db `{}`: acquiring connection: {e}", cfg.db))
                })?;
                let table_owned = table.to_string();
                let table_for_err = table.to_string();
                // Changelog shape first (an older changelog lacks the `key`
                // column the new triggers write), then the table's primary
                // key, then the triggers — one blocking hop for all three.
                tokio::task::spawn_blocking(move || {
                    conn.with(|c| -> Result<(), String> {
                        super::sqlite_watch::ensure_changelog(c).map_err(|e| e.to_string())?;
                        let pk = super::sqlite_watch::pk_columns(c, &table_owned)
                            .map_err(|e| e.to_string())?;
                        let sql = super::sqlite_watch::install_sql(&table_owned, &pk)?;
                        c.execute_batch(&sql).map_err(|e| e.to_string())
                    })
                })
                .await
                .map_err(|e| config_error(format!("sqlite DDL join: {e}")))?
                .map_err(|e| {
                    config_error(format!(
                        "installing native capture triggers on `{table_for_err}`: {e}"
                    ))
                })?;
            }
            Some(Pool::Mysql(my)) => {
                // Binlog capture installs nothing — but a binding on a server
                // that cannot be streamed would sit silent forever. Verify
                // the prerequisites here, where the failure is actionable.
                use mysql_async::prelude::Queryable as _;
                let mut conn = my.acquire().await.map_err(|e| {
                    config_error(format!("db `{}`: acquiring connection: {e}", cfg.db))
                })?;
                let settings: Option<(i64, String)> = conn
                    .query_first("SELECT @@log_bin, @@binlog_format")
                    .await
                    .map_err(|e| config_error(format!("db `{}`: {e}", cfg.db)))?;
                match settings {
                    Some((1, format)) if format.eq_ignore_ascii_case("ROW") => {}
                    Some((1, format)) => {
                        return Err(config_error(format!(
                            "db `{}`: binlog_format is {format}; native capture needs ROW \
                             (SET GLOBAL binlog_format = 'ROW', the 8.x default)",
                            cfg.db
                        )));
                    }
                    _ => {
                        return Err(config_error(format!(
                            "db `{}`: the server runs without a binary log (log_bin=OFF); \
                             native capture reads the binlog and cannot work here",
                            cfg.db
                        )));
                    }
                }
                // Doubles as the privilege probe: needs REPLICATION CLIENT,
                // and the stream itself needs REPLICATION SLAVE.
                super::mysql_binlog::binlog_position(&mut conn)
                    .await
                    .map_err(|e| config_error(format!("db `{}`: {e}", cfg.db)))?;
                // Row identity needs column names and key flags in the
                // binlog's table maps, which only `binlog_row_metadata=FULL`
                // provides (the 8.x default is MINIMAL). Warn, don't refuse:
                // existing bindings keep working across the upgrade and the
                // events simply stay count-only until the operator flips it.
                // `.ok().flatten()` also covers MariaDB, which lacks the variable.
                let meta: Option<String> = conn
                    .query_first("SELECT @@binlog_row_metadata")
                    .await
                    .ok()
                    .flatten();
                if !meta
                    .as_deref()
                    .is_some_and(|m| m.eq_ignore_ascii_case("FULL"))
                {
                    tracing::warn!(
                        db = %cfg.db,
                        table = %table,
                        "binlog_row_metadata is not FULL; row-changed events on this database \
                         carry no primary-key values (SET GLOBAL binlog_row_metadata = 'FULL', \
                         or binlog_row_metadata=FULL in my.cnf)"
                    );
                }
            }
            None => {
                // Every driver supports native capture, so reaching this arm
                // means exactly one thing: config and pools drifted, which
                // apply_config forbids.
                return Err(config_error(format!(
                    "db `{}`: no pool available for native capture",
                    cfg.db
                )));
            }
        }
        tracing::info!(db = %cfg.db, table = %table, "native capture triggers installed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trigger(id: &str, db: &str) -> TriggerConfig {
        TriggerConfig {
            id: id.into(),
            function_id: "app::on-change".into(),
            config: serde_json::json!({ "db": db }),
            metadata: None,
            namespace: None,
        }
    }

    fn handler_with(
        config: WorkerConfig,
    ) -> (RowChangedHandler, Arc<tokio::sync::RwLock<WorkerConfig>>) {
        let config = Arc::new(tokio::sync::RwLock::new(config));
        let handler = RowChangedHandler {
            bus: Arc::new(RowChangeBus::new(
                Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:9")),
                100,
            )),
            config: config.clone(),
            pools: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        };
        (handler, config)
    }

    /// Concurrent bindings share the notify function across tables and pools.
    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_postgres_capture_registrations_keep_every_subscriber() {
        let Ok(url) = std::env::var("TEST_POSTGRES_URL") else {
            eprintln!("skipping: TEST_POSTGRES_URL not set");
            return;
        };
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let (admin, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
            let connection = tokio::spawn(async move { connection.await.unwrap() });
            let schema = format!("iii_capture_race_{}", uuid::Uuid::new_v4().simple());
            admin.batch_execute(&format!(
                "CREATE SCHEMA {schema}; CREATE TABLE {schema}.a (id int); CREATE TABLE {schema}.b (id int);"
            )).await.unwrap();
            let mut scoped_url = url::Url::parse(&url).unwrap();
            scoped_url.query_pairs_mut().append_pair("options", &format!("-csearch_path={schema}"));
            let worker_config = WorkerConfig::from_yaml(&format!(
                "databases:\n  p:\n    url: {}\n    capture: native\n    pool:\n      max: 8\n    tls:\n      mode: disable\n", scoped_url
            )).unwrap();
            let mut handlers = Vec::new();
            // Independent pools also cover separate worker instances.
            for _ in 0..2 {
                let pool = crate::pool::build("p", &worker_config.databases["p"]).await.unwrap();
                handlers.push(Arc::new(RowChangedHandler {
                    bus: Arc::new(RowChangeBus::new(Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:9")), 100)),
                    config: Arc::new(tokio::sync::RwLock::new(worker_config.clone())),
                    pools: Arc::new(tokio::sync::RwLock::new(HashMap::from([("p".into(), pool)]))),
                }));
            }
            let barrier = Arc::new(tokio::sync::Barrier::new(16));
            let mut registrations = tokio::task::JoinSet::new();
            for index in 0..16 {
                let handler = handlers[index % handlers.len()].clone();
                let barrier = barrier.clone();
                let mut binding = trigger(&format!("concurrent-{index}"), "p");
                binding.config = serde_json::json!({ "db": "p", "table": if index % 4 < 2 { "a" } else { "b" } });
                registrations.spawn(async move {
                    barrier.wait().await;
                    handler.register_trigger(binding).await
                });
            }
            let mut errors = Vec::new();
            while let Some(result) = registrations.join_next().await {
                if let Err(error) = result.unwrap() { errors.push(error.to_string()); }
            }
            let subscribers: usize = handlers.iter().map(|handler| handler.bus.subscriber_count()).sum();
            let installed: i64 = admin.query_one(
                "SELECT count(*) FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND NOT t.tgisinternal",
                &[&schema],
            ).await.unwrap().get(0);
            // A failed install must release its lock and leave the pool usable.
            let mut missing = trigger("missing-table", "p");
            missing.config = serde_json::json!({ "db": "p", "table": "absent" });
            assert!(handlers[0].register_trigger(missing).await.is_err());
            let mut recovery = trigger("recovery", "p");
            recovery.config = serde_json::json!({ "db": "p", "table": "a" });
            let recovered = handlers[0].register_trigger(recovery).await;
            admin.batch_execute(&format!("DROP SCHEMA {schema} CASCADE")).await.unwrap();
            connection.abort();
            assert!(errors.is_empty(), "concurrent registration failures: {errors:?}");
            assert_eq!(subscribers, 16);
            assert_eq!(installed, 6, "three capture triggers on each table");
            recovered.expect("registration remains usable after a failed install");
        }).await.expect("concurrent registration/cleanup deadline");
    }

    #[test]
    fn pg_install_hint_matches_the_failure_shape() {
        // The bug this pins: a `does not exist` failure wrapped in privilege
        // advice reads as a grants problem and hides the real cause (casing).
        let hint = pg_install_hint(
            r#"relation "III_TRIGGER_TEST" does not exist"#,
            "III_TRIGGER_TEST",
        );
        assert!(hint.contains("case-sensitive"), "{hint}");
        assert!(!hint.contains("TRIGGER privilege"), "{hint}");

        let hint = pg_install_hint("permission denied for table orders", "orders");
        assert!(hint.contains("TRIGGER privilege"), "{hint}");
        let hint = pg_install_hint("must be owner of relation orders", "orders");
        assert!(hint.contains("TRIGGER privilege"), "{hint}");

        // Anything else gets the raw error only — no guessed guidance.
        assert_eq!(pg_install_hint("connection reset by peer", "orders"), "");
    }

    #[tokio::test]
    async fn native_bindings_must_name_a_table() {
        let cfg = WorkerConfig::from_yaml(
            "databases:\n  p:\n    url: postgres://u@h/db\n    capture: native\n",
        )
        .unwrap();
        let (handler, _) = handler_with(cfg);

        let err = handler
            .register_trigger(trigger("i1", "p"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("name a `table`"), "{err}");

        // With a table but no live pool the DDL cannot land; registration
        // still fails loudly instead of listening to nothing.
        let mut with_table = trigger("i2", "p");
        with_table.config = serde_json::json!({ "db": "p", "table": "orders" });
        let err = handler.register_trigger(with_table).await.unwrap_err();
        assert!(err.to_string().contains("no pool"), "{err}");
        assert_eq!(handler.bus.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn registration_uses_the_live_database_config() {
        let (handler, config) = handler_with(WorkerConfig::default());

        handler
            .register_trigger(trigger("initial", "primary"))
            .await
            .unwrap();

        let mut live = config.write().await;
        let db = live.databases.remove("primary").unwrap();
        live.databases.insert("analytics".into(), db);
        drop(live);

        assert!(handler
            .register_trigger(trigger("removed", "primary"))
            .await
            .is_err());
        handler
            .register_trigger(trigger("added", "analytics"))
            .await
            .unwrap();
    }
}
