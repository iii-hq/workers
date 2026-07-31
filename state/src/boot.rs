//! Boot sequence: builtin guard → build adapter → trigger table → register the
//! `state` trigger type → register the six state::* functions.

use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterTriggerType};
use tokio::sync::RwLock;

use crate::TRIGGER_TYPE;
use crate::adapters::{self, StateAdapter};
use crate::config::StateConfig;
use crate::events::{Invoker, SdkInvoker};
use crate::functions::{self, ConfigCell, StateCtx};
use crate::trigger::{StateTriggerHandler, StateTriggerSpec, TriggerTable};

const LIST_WORKERS_FUNCTION_ID: &str = "engine::workers::list";
const BUILTIN_III_STATE_WORKER_ID: &str = "iii-state";

pub type ApplyLock = Arc<tokio::sync::Mutex<()>>;

pub struct BootHandle {
    pub ctx: Arc<StateCtx>,
    pub triggers: TriggerTable,
    pub config: ConfigCell,
    pub apply_lock: ApplyLock,
}

impl BootHandle {
    pub async fn shutdown(&self) {
        // Store parity with the builtin's destroy(): kv flushes on the save
        // loop (nothing extra to do — pending dirty entries flush on the next
        // tick; a hard kill loses at most one save window, same as the engine).
        let _ = self.ctx.adapter.destroy().await;
    }
}

pub async fn start(iii: Arc<IIIClient>, config: StateConfig) -> anyhow::Result<BootHandle> {
    guard_against_builtin_state(&iii).await?;

    let config = config.normalized();
    let adapter: Arc<dyn StateAdapter> = adapters::build_adapter(&config).await?;

    let handler = StateTriggerHandler::new();
    let triggers = handler.triggers.clone();

    // Registering the trigger type makes the engine deliver every `state`
    // trigger binding through `handler`, which populates the fan-out table.
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(TRIGGER_TYPE, "State trigger", handler)
            .trigger_request_format::<StateTriggerSpec>(),
    );

    // Private namespaces are CLAIMED at runtime (`state::claim-namespace`),
    // not configured: this starts empty and is refilled from the persisted
    // ledger below.
    let private = Arc::new(functions::PrivateNamespaces::default());
    let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(config)));
    let invoker: Arc<dyn Invoker> = Arc::new(SdkInvoker { iii: iii.clone() });
    let ctx = Arc::new(StateCtx {
        adapter,
        triggers: triggers.clone(),
        config: cell.clone(),
        invoker,
        private,
    });
    functions::register_functions(&iii, ctx.clone());
    // Re-arm namespaces claimed by earlier runs BEFORE any tenant asks, so a
    // state restart restores `<prefix>::state::*` on its own.
    functions::restore_persisted_claims(&iii, &ctx).await;

    // Injectable console UI: content function + console:script trigger.
    // Ordering doesn't matter for the console side (the engine parks the
    // registration until a console owns the type), but the content function
    // must exist before the trigger names it.
    crate::ui::register(&iii);

    Ok(BootHandle {
        ctx,
        triggers,
        config: cell,
        apply_lock: Arc::new(tokio::sync::Mutex::new(())),
    })
}

async fn guard_against_builtin_state(iii: &Arc<IIIClient>) -> anyhow::Result<()> {
    let workers_list = iii
        .trigger(TriggerRequest {
            function_id: LIST_WORKERS_FUNCTION_ID.to_string(),
            payload: serde_json::json!({}),
            action: None,
            timeout_ms: Some(5000),
        })
        .await
        .map_err(|e| anyhow::anyhow!("failed to query {LIST_WORKERS_FUNCTION_ID}: {e}"))?;

    if builtin_iii_state_active(&workers_list) {
        anyhow::bail!(
            "cannot start the state worker: the built-in iii-state worker is active and owns the \
             'state' trigger type and the state::* functions. Remove iii-state from the engine \
             config (a config.yaml that doesn't list it won't run it), then start this worker. \
             If the builtin used a file_based kv store, point this worker's adapter at the same \
             file_path — the on-disk format is identical."
        );
    }
    Ok(())
}

fn builtin_iii_state_active(workers_list: &serde_json::Value) -> bool {
    workers_list
        .get("workers")
        .and_then(|w| w.as_array())
        .is_some_and(|workers| {
            workers.iter().any(|worker| {
                worker.get("id").and_then(|v| v.as_str()) == Some(BUILTIN_III_STATE_WORKER_ID)
                    || worker.get("name").and_then(|v| v.as_str())
                        == Some(BUILTIN_III_STATE_WORKER_ID)
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_builtin_by_id() {
        let v = serde_json::json!({"workers": [{"id": "iii-state"}]});
        assert!(builtin_iii_state_active(&v));
    }

    #[test]
    fn detects_builtin_by_name() {
        let v = serde_json::json!({"workers": [{"id": "x", "name": "iii-state"}]});
        assert!(builtin_iii_state_active(&v));
    }

    #[test]
    fn absent_builtin_passes() {
        let v = serde_json::json!({"workers": [{"id": "iii-http"}]});
        assert!(!builtin_iii_state_active(&v));
    }

    #[test]
    fn empty_list_passes() {
        assert!(!builtin_iii_state_active(
            &serde_json::json!({"workers": []})
        ));
    }

    #[test]
    fn missing_key_passes() {
        assert!(!builtin_iii_state_active(&serde_json::json!({})));
    }
}
