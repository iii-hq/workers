//! TriggerHandler implementations for `iii-database::query-poll` and
//! `iii-database::row-change`. Wired into the worker via
//! `iii.register_trigger_type` from main.rs.
//!
//! The engine routes `iii.registerTrigger({type: "iii-database::query-poll", ...})`
//! calls from any client (e.g. the test harness) back to the worker that
//! registered that trigger type. We spawn a per-instance polling loop on
//! `register_trigger` and cancel it on `unregister_trigger`.

use crate::handlers::AppState;
use crate::triggers::query_poll::{self, Dispatch, DispatchAck, DispatchedBatch, QueryPollConfig};
use async_trait::async_trait;
use iii_sdk::{protocol::TriggerRequest, IIIError, TriggerConfig, TriggerHandler, III};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// Dispatch impl that forwards a polled batch to the engine via `iii.trigger`.
/// The engine routes the invocation to whoever registered `function_id`.
struct EngineDispatch {
    iii: III,
    function_id: String,
}

#[async_trait]
impl Dispatch for EngineDispatch {
    async fn dispatch(&self, batch: DispatchedBatch) -> Result<DispatchAck, crate::error::DbError> {
        let payload =
            serde_json::to_value(&batch).map_err(|e| crate::error::DbError::DriverError {
                driver: "engine-dispatch".into(),
                code: None,
                message: format!("serialize batch: {e}"),
                failed_index: None,
            })?;
        let req = TriggerRequest {
            function_id: self.function_id.clone(),
            payload,
            action: None,
            timeout_ms: None,
        };
        match self.iii.trigger(req).await {
            Ok(value) => {
                // Three response shapes, three behaviors:
                //   1. null  → function returned void = successful completion;
                //              ack so the cursor advances.
                //   2. valid {ack?, commit_cursor?} → use as-is.
                //   3. anything else → malformed; fail-safe to ack=false so
                //              the next tick retries instead of silently
                //              dropping rows the function never processed.
                if value.is_null() {
                    Ok(DispatchAck {
                        ack: true,
                        commit_cursor: None,
                    })
                } else {
                    Ok(
                        serde_json::from_value::<DispatchAck>(value).unwrap_or(DispatchAck {
                            ack: false,
                            commit_cursor: None,
                        }),
                    )
                }
            }
            Err(e) => Err(crate::error::DbError::DriverError {
                driver: "engine-dispatch".into(),
                code: None,
                message: format!("trigger invocation failed: {e}"),
                failed_index: None,
            }),
        }
    }
}

/// `iii-database::query-poll` trigger handler. Spawns a polling loop per
/// registered trigger instance; cancels on unregister.
///
/// Tasks are tracked twice:
///   - by engine-assigned instance id (for unregister, which receives that id)
///   - by user-supplied `trigger_id` (so a re-registration with the same
///     trigger_id replaces the old task, which is essential for idempotent
///     re-runs of clients whose `unregister` is fire-and-forget across a
///     process exit).
pub struct QueryPollTrigger {
    state: AppState,
    iii: III,
    /// Map of trigger instance id → spawned task handle. Indexed for
    /// `unregister_trigger`.
    tasks: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
    /// Map of user-supplied `trigger_id` → engine-assigned instance id.
    /// Used to evict stale tasks when the same `trigger_id` is registered
    /// again before the prior `unregister` has reached us.
    by_trigger_id: Arc<Mutex<HashMap<String, String>>>,
}

impl QueryPollTrigger {
    pub fn new(state: AppState, iii: III) -> Self {
        Self {
            state,
            iii,
            tasks: Arc::new(Mutex::new(HashMap::new())),
            by_trigger_id: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

fn iii_err<T: serde::Serialize>(err: T) -> IIIError {
    IIIError::Handler(serde_json::to_string(&err).unwrap_or_else(|_| "{}".into()))
}

#[async_trait]
impl TriggerHandler for QueryPollTrigger {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), IIIError> {
        let mut cfg: QueryPollConfig =
            serde_json::from_value(config.config.clone()).map_err(|e| {
                iii_err(crate::error::DbError::ConfigError {
                    message: format!("query-poll config: {e}"),
                })
            })?;
        // If the user-provided trigger_id is empty (or absent — serde default),
        // fall back to the engine-assigned instance id so the cursor table
        // key is stable across restarts of the same instance.
        if cfg.trigger_id.is_empty() {
            cfg.trigger_id = config.id.clone();
        }
        cfg.validate().map_err(iii_err)?;

        let pool = self
            .state
            .pools
            .get(&cfg.db_name)
            .ok_or_else(|| {
                iii_err(crate::error::DbError::UnknownDb {
                    db: cfg.db_name.clone(),
                })
            })?
            .clone();

        let dispatch: Arc<dyn Dispatch> = Arc::new(EngineDispatch {
            iii: self.iii.clone(),
            function_id: config.function_id.clone(),
        });

        let trigger_id = cfg.trigger_id.clone();

        // Acquire the registry locks *before* spawning. Previously the spawn
        // ran first (outside any lock) and the JoinHandle was inserted into
        // `tasks` afterward — a concurrent `unregister_trigger(config.id)`
        // could fire in between, find `tasks` empty, return Ok, and the
        // newly-spawned poller would leak running forever (orphaned task
        // with no abort handle and no entry in either index).
        //
        // `tokio::spawn` returns synchronously (it just schedules the future
        // on the runtime), so holding the lock across it grows the critical
        // section by microseconds — well worth closing the TOCTOU race.
        //
        // Lock order `by_trigger_id` → `tasks` is canonical and matches
        // `unregister_trigger` below; reversing would deadlock on concurrent
        // calls.
        {
            let mut by_id = self.by_trigger_id.lock().await;
            let mut tasks = self.tasks.lock().await;

            // Evict stale instance for the same user-supplied trigger_id.
            let stale = by_id.insert(trigger_id.clone(), config.id.clone());
            if let Some(s) = stale {
                if let Some(old_task) = tasks.remove(&s) {
                    old_task.abort();
                    tracing::info!(
                        trigger_id = %trigger_id,
                        evicted_instance = %s,
                        "query-poll evicted stale task on re-registration"
                    );
                }
            }

            // Spawn under the lock so unregister_trigger(config.id) cannot
            // interleave between "task spawned" and "task in registry".
            let task = tokio::spawn(async move {
                query_poll::run_loop(pool, cfg, dispatch).await;
            });
            tasks.insert(config.id.clone(), task);
        }

        tracing::info!(
            trigger_instance = %config.id,
            trigger_id = %trigger_id,
            function_id = %config.function_id,
            "query-poll trigger registered"
        );
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), IIIError> {
        // Lock order: `by_trigger_id` → `tasks`, matching `register_trigger`.
        // Reverse ordering would deadlock against a concurrent register.
        let mut by_id = self.by_trigger_id.lock().await;
        let mut tasks = self.tasks.lock().await;
        if let Some(task) = tasks.remove(&config.id) {
            task.abort();
            tracing::info!(trigger_instance = %config.id, "query-poll trigger unregistered");
        }
        // Best-effort: drop any reverse-index entries that point at this
        // instance. If a different instance has since taken the trigger_id
        // slot, leave that mapping alone.
        by_id.retain(|_, instance| instance != &config.id);
        Ok(())
    }
}

/// `iii-database::row-change` trigger handler. v1.0 stubs the streaming decoder
/// pending an upstream tokio-postgres replication API release. `register_trigger`
/// returns Unsupported so callers see a clear error instead of silently never
/// receiving events.
pub struct RowChangeTrigger;

#[async_trait]
impl TriggerHandler for RowChangeTrigger {
    async fn register_trigger(&self, _config: TriggerConfig) -> Result<(), IIIError> {
        Err(iii_err(crate::error::DbError::Unsupported {
            op: "row-change".into(),
            driver: "postgres (pending tokio-postgres replication API release)".into(),
        }))
    }
    async fn unregister_trigger(&self, _config: TriggerConfig) -> Result<(), IIIError> {
        Ok(())
    }
}
