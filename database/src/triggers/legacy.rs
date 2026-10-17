//! The pre-0.25 fallback: the `database::row-changed` trigger type, for an
//! engine without observe.
//!
//! Against an engine that speaks `observe.v1` nothing here runs: the function
//! declares `legacy_trigger_type: database::row-changed`, and the engine's
//! legacy shim turns old `registerTrigger` calls into observers. An older
//! engine has no shim, and the SDK registers the function as a plain one
//! (its observe hooks never run), so old consumers would hear nothing. For
//! that engine the worker registers the old trigger type as before; each
//! binding is validated by the same code as an observer and filed in the
//! same routing index, as a [`Sink::Trigger`].
//!
//! Delete this module once the minimum supported engine is 0.25.

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::runtime::TriggerTypeRef;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType};
use serde_json::json;

use super::bus::{LegacyRowChangedEvent, RowChangedArgs, ROW_CHANGED_TYPE};
use super::capture::{NativeCapture, Sink, ROW_CHANGED_FN};

pub const ROW_CHANGED_TYPE_DESCRIPTION: &str =
    "Fires after a row change commits, filtered by `db`, optional `table`, and optional `ops`. \
     Kept for engines older than 0.25; observe database::on-row-changed instead.";

/// Whether the engine on the current connection speaks observe. An engine
/// older than 0.25 has no `engine::observers::list`. Anything but
/// `function_not_found` (a denial, a timeout) is read as "yes": the old type
/// is only for engines that cannot serve the function's observers.
pub async fn engine_supports_observe(iii: &IIIClient) -> bool {
    let probe = iii
        .trigger(TriggerRequest::new(
            "engine::observers::list",
            json!({ "function_id": ROW_CHANGED_FN }),
        ))
        .await;
    !matches!(probe, Err(Error::Remote { ref code, .. }) if code == "function_not_found")
}

/// Registers the old trigger type over `capture`'s index.
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
            "row-changed trigger registered (pre-0.25 engine)"
        );
        self.capture.file(
            config.id.clone(),
            args,
            Sink::Trigger {
                function_id: config.function_id.clone(),
                metadata: config.metadata.clone(),
            },
            true,
            installed,
        );
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.capture.unfile(&config.id);
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
