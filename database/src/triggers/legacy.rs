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
//! [`Fallback`] follows the engine on the current connection, as the SDK
//! reports it (`IIIClient::on_engine_supports_observe`), so an engine
//! upgraded or rolled back under the running worker is handled without a
//! restart.
//!
//! Delete this module once the minimum supported engine is 0.25.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::runtime::TriggerTypeRef;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType};

use super::bus::{LegacyRowChangedEvent, RowChangedArgs, ROW_CHANGED_TYPE};
use super::capture::{NativeCapture, Sink};

pub const ROW_CHANGED_TYPE_DESCRIPTION: &str =
    "Fires after a row change commits, filtered by `db`, optional `table`, and optional `ops`. \
     Kept for engines older than 0.25; observe database::on-row-changed instead.";

/// A binding's id in the routing index. Kept apart from observer ids: the
/// shim gives the observer it makes from a binding the binding's own id, and
/// while the worker is on an engine without observe the SDK still holds that
/// observer. Its route must survive the binding's: an engine with observe
/// that kept the observer while the worker was away acks it again without
/// calling the hooks (see [`NativeCapture::set_engine_observes`]).
fn route_id(trigger_id: &str) -> String {
    format!("\0trigger:{trigger_id}")
}

/// The old trigger type, registered while the engine on the current
/// connection lacks observe.
pub struct Fallback {
    iii: IIIClient,
    capture: Arc<NativeCapture>,
    registered: Mutex<Option<TriggerTypeRef<RowChangedArgs, LegacyRowChangedEvent>>>,
}

impl Fallback {
    pub fn new(iii: IIIClient, capture: Arc<NativeCapture>) -> Self {
        Self {
            iii,
            capture,
            registered: Mutex::new(None),
        }
    }

    /// Registers the trigger type for an engine without observe, and
    /// unregisters it for one with observe. Idempotent; called with each
    /// value `IIIClient::on_engine_supports_observe` reports.
    ///
    /// Unregistering covers an engine upgraded under the running worker: the
    /// SDK replays the type on reconnect, and a live old-style provider wins
    /// over the shim. Retired, the type's bindings park, the engine hands them
    /// to the function's shim as observers, and the index drops their trigger
    /// routes. A rollback registers the type again and the parked bindings
    /// return to it.
    pub fn follow(&self, engine_observes: bool) {
        let mut registered = self.registered.lock().unwrap_or_else(|e| e.into_inner());
        self.capture.set_engine_observes(engine_observes);
        match (engine_observes, registered.is_some()) {
            (false, false) => {
                tracing::warn!(
                    "the engine does not support observe; registering the deprecated \
                     database::row-changed trigger type for old consumers"
                );
                *registered = Some(register_trigger_type(&self.iii, self.capture.clone()));
            }
            (true, true) => {
                *registered = None;
                // Kept until the minimum engine is 0.25.
                #[allow(deprecated)]
                self.iii.unregister_trigger_type(ROW_CHANGED_TYPE);
                let bindings = self.capture.unfile_trigger_sinks();
                tracing::info!(
                    bindings,
                    "the engine supports observe; unregistered the database::row-changed \
                     trigger type, the engine's legacy shim serves old consumers"
                );
            }
            _ => {}
        }
    }

    /// Whether the trigger type is registered.
    pub fn is_registered(&self) -> bool {
        self.registered
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
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
            route_id(&config.id),
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

    /// The type is registered only while the engine lacks observe: an
    /// upgrade under the worker unregisters it and drops its bindings' routes
    /// (the engine's shim takes them over), a rollback registers it again.
    #[tokio::test]
    async fn the_fallback_follows_the_engines_observe_support() {
        let (handler, _) = handler_with(WorkerConfig::default());
        let fallback = Fallback::new(IIIClient::new("ws://127.0.0.1:9"), handler.capture.clone());
        fallback.follow(true);
        assert!(!fallback.is_registered());

        fallback.follow(false);
        fallback.follow(false);
        assert!(fallback.is_registered());
        handler
            .register_trigger(trigger("binding", "primary"))
            .await
            .unwrap();
        assert_eq!(handler.capture.route_count(), 1);
        assert!(handler.capture.is_listened());

        fallback.follow(true);
        assert!(!fallback.is_registered());
        assert_eq!(handler.capture.route_count(), 0);
        assert!(!handler.capture.is_listened());
        fallback.follow(true);
        assert!(!fallback.is_registered());

        fallback.follow(false);
        assert!(fallback.is_registered());
    }

    /// A binding is filed apart from an observer with its id: the shim's
    /// observer for a binding has the binding's id.
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
            true,
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
