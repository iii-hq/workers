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
                let client = pg.acquire().await.map_err(|e| {
                    config_error(format!("db `{}`: acquiring connection: {e}", cfg.db))
                })?;
                client.batch_execute(&sql).await.map_err(|e| {
                    let text = e.to_string();
                    config_error(format!(
                        "installing native capture triggers on `{table}`: {text}{}",
                        pg_install_hint(&text, table)
                    ))
                })?;
            }
            Some(Pool::Sqlite(sq)) => {
                let sql = super::sqlite_watch::install_sql(table).map_err(config_error)?;
                let conn = sq.acquire().await.map_err(|e| {
                    config_error(format!("db `{}`: acquiring connection: {e}", cfg.db))
                })?;
                let table_for_err = table.to_string();
                tokio::task::spawn_blocking(move || conn.with(|c| c.execute_batch(&sql)))
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
