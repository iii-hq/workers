//! The `database::row-changed` trigger type, for consumers that still bind
//! it: `registerTrigger` from a worker, `engine::register_trigger` (the CLI,
//! a script), harness subscriptions.
//!
//! The worker registers it on every engine, 0.24 and 0.25 alike, so all of
//! them are served exactly as before the migration. Each binding is
//! validated by the same code as an observer of `database::on-row-changed`
//! and filed in the same routing index, as a [`Sink::Trigger`]; it receives
//! the event in its pre-0.25 shape. The function declares no
//! `legacy_trigger_type`: the engine's legacy shim would serve only bindings
//! registered over a worker connection (an ownerless one made through
//! `engine::register_trigger` stays pending), and a live provider of the type
//! wins over a stand-in anyway.
//!
//! Delete this module, and declare `legacy_trigger_type`, in the release that
//! retires the trigger type.

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::runtime::TriggerTypeRef;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType};

use super::bus::{LegacyRowChangedEvent, RowChangedArgs, ROW_CHANGED_TYPE};
use super::capture::{NativeCapture, Sink};

pub const ROW_CHANGED_TYPE_DESCRIPTION: &str =
    "Fires after a row change commits, filtered by `db`, optional `table`, and optional `ops`. \
     Kept for consumers that still bind it; new code observes database::on-row-changed.";

/// A binding's id in the routing index. Bindings and observers are two id
/// spaces, each chosen by its registrant: kept apart, one never replaces the
/// other.
fn route_id(trigger_id: &str) -> String {
    format!("\0trigger:{trigger_id}")
}

/// Registers the old trigger type over `capture`'s index. Kept until the
/// trigger type is retired: old consumers bind it on 0.24 and 0.25 engines.
#[allow(deprecated)]
pub fn register_trigger_type(
    iii: &IIIClient,
    capture: Arc<NativeCapture>,
) -> TriggerTypeRef<RowChangedArgs, LegacyRowChangedEvent> {
    iii.register_trigger_type(
        RegisterTriggerType::new(
            ROW_CHANGED_TYPE,
            ROW_CHANGED_TYPE_DESCRIPTION,
            LegacyRowChanged { capture },
        )
        .trigger_request_format::<RowChangedArgs>()
        .call_request_format::<LegacyRowChangedEvent>(),
    )
}

/// The old `TriggerHandler`, now a thin adapter over [`NativeCapture`].
pub struct LegacyRowChanged {
    pub capture: Arc<NativeCapture>,
}

#[async_trait]
impl TriggerHandler for LegacyRowChanged {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let (args, install) = NativeCapture::legacy_args(&config.config)?;
        let installed = self.capture.admit(&args, install.as_deref()).await?;
        tracing::info!(
            instance = %config.id,
            function = %config.function_id,
            table = ?args.table,
            "row-changed trigger registered"
        );
        self.capture.file(
            route_id(&config.id),
            args,
            Sink::Trigger {
                function_id: config.function_id.clone(),
                metadata: config.metadata.clone(),
            },
            installed,
        );
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.capture.unfile(&route_id(&config.id));
        tracing::info!(instance = %config.id, "row-changed trigger unregistered");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkerConfig;
    use std::collections::HashMap;

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
    ) -> (LegacyRowChanged, Arc<tokio::sync::RwLock<WorkerConfig>>) {
        let config = Arc::new(tokio::sync::RwLock::new(config));
        let capture = Arc::new(NativeCapture::new(
            Arc::new(IIIClient::new("ws://127.0.0.1:9")),
            config.clone(),
            Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            100,
        ));
        (LegacyRowChanged { capture }, config)
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
                handlers.push(Arc::new(LegacyRowChanged {
                    capture: Arc::new(NativeCapture::new(
                        Arc::new(IIIClient::new("ws://127.0.0.1:9")),
                        Arc::new(tokio::sync::RwLock::new(worker_config.clone())),
                        Arc::new(tokio::sync::RwLock::new(HashMap::from([("p".into(), pool)]))),
                        100,
                    )),
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
            let subscribers: usize = handlers.iter().map(|handler| handler.capture.route_count()).sum();
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
        assert!(err.to_string().contains("requires args.table"), "{err}");

        let mut with_table = trigger("i2", "p");
        with_table.config = serde_json::json!({ "db": "p", "table": "orders" });
        let err = handler.register_trigger(with_table).await.unwrap_err();
        assert!(err.to_string().contains("no pool"), "{err}");
        assert_eq!(handler.capture.route_count(), 0);
    }

    /// A binding is filed apart from an observer with the same id.
    #[tokio::test]
    async fn bindings_do_not_replace_observers_with_their_id() {
        let (handler, _) = handler_with(WorkerConfig::default());
        let args = RowChangedArgs {
            db: "primary".into(),
            table: None,
            ops: None,
        };
        handler.capture.file(
            "shared-id".into(),
            args,
            Sink::Trigger {
                function_id: "app::observer-stand-in".into(),
                metadata: None,
            },
            None,
        );
        handler
            .register_trigger(trigger("shared-id", "primary"))
            .await
            .unwrap();
        assert_eq!(handler.capture.route_count(), 2);
        handler
            .unregister_trigger(trigger("shared-id", "primary"))
            .await
            .unwrap();
        assert_eq!(handler.capture.route_count(), 1);
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
        assert_eq!(handler.capture.route_count(), 2);
        assert!(handler.capture.is_listened());
    }
}
