//! Memory-owned pre-generate registration. Payload metadata is never identity.
use crate::{deps::Deps, hooks};
use iii_sdk::runtime::IIIConnectionState;
use iii_sdk::{
    CallRequest, IIIClient, InvocationCtx, ObserverHandle, ObserverState, RegisterFunction,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const SOURCE: &str = "harness::hook::on-pre-generate";

/// Disabled in fixtures unless the actual SDK registration is installed.
#[derive(Default)]
pub struct HookBinding {
    active: Mutex<Option<ObserverHandle>>,
    /// The background history imports, by session. One per session at a
    /// time: its every step delivers the history again, so the next step
    /// retries a skipped delivery. Another session's import never skips it.
    import: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
}
impl HookBinding {
    pub fn install(&self, handle: ObserverHandle) {
        *self.active.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
    }
    /// Wait for every background history import.
    pub async fn import_settled(&self) {
        let imports: Vec<_> = self
            .import
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .map(|(_, import)| import)
            .collect();
        for import in imports {
            let _ = import.await;
        }
    }
    /// Wait for this session's history import, if one runs. It never takes
    /// the session's extraction pass lock, so a pass may wait under it.
    pub(crate) async fn session_import_settled(&self, session_id: &str) {
        let import = self
            .import
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session_id);
        if let Some(import) = import {
            let _ = import.await;
        }
    }
    fn matches(&self, ctx: &InvocationCtx, iii: &IIIClient) -> bool {
        iii.get_connection_state() == IIIConnectionState::Connected
            && iii.engine_supports_observe() == Some(true)
            && ctx.function_id == hooks::PRE_GENERATE_FN
            && self
                .active
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .is_some_and(|h| {
                    h.state() == ObserverState::Active
                        && ctx
                            .activation
                            .as_ref()
                            .is_some_and(|a| a.observer_id == h.id())
                })
    }
    /// No owner cache: each delivery gets bounded, fresh engine-owned host data.
    /// The second lookup includes the function in that worker's actual surface.
    pub async fn authenticates(&self, iii: &IIIClient, ctx: Option<&InvocationCtx>) -> bool {
        let Some(ctx) = ctx.filter(|ctx| self.matches(ctx, iii)) else {
            return false;
        };
        let Some(id) = ctx
            .caller
            .as_ref()
            .and_then(|caller| caller.worker_id.as_deref())
            .filter(|id| !id.is_empty())
        else {
            return false;
        };
        let namespace = iii.namespace().unwrap_or_else(|| "default".into());
        let lookup = async {
            let info = iii
                .call(
                    CallRequest::new(
                        "engine::functions::info",
                        json!({
                            "function_id":SOURCE, "namespace":namespace
                        }),
                    )
                    .metadata(json!({}))
                    .timeout(std::time::Duration::from_millis(400)),
                )
                .await
                .ok()?;
            let name = info["worker_name"].as_str()?;
            if info["function_id"] != SOURCE || info["namespace"] != namespace {
                return None;
            }
            let host = iii
                .call(
                    CallRequest::new(
                        "engine::workers::info",
                        json!({
                            "name":name, "namespace":namespace
                        }),
                    )
                    .metadata(json!({}))
                    .timeout(std::time::Duration::from_millis(400)),
                )
                .await
                .ok()?;
            let functions = host["functions"].as_array()?;
            let owner = host["worker"]["id"].as_str().filter(|id| !id.is_empty())?;
            Some(
                owner == id
                    && host["worker"]["status"] == "connected"
                    && host["worker"]["namespace"] == namespace
                    && functions
                        .iter()
                        .any(|f| f["function_id"] == SOURCE && f["namespace"] == namespace),
            )
        };
        tokio::time::timeout(std::time::Duration::from_millis(900), lookup)
            .await
            .ok()
            .flatten()
            .unwrap_or(false)
            && self.matches(ctx, iii)
    }
}

/// Used by the binary AND SDK-loopback tests. Capture task-local identity
/// synchronously in the registered closure, before any async boundary.
pub fn register_pre_generate(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    let deps = deps.clone();
    iii.register_function(
        hooks::PRE_GENERATE_FN,
        RegisterFunction::new_async(move |mut input: hooks::PreGenerateInput| {
            let invocation = iii_sdk::current_invocation();
            let deps = deps.clone();
            async move {
                // Only a finished failed or cancelled request can add to the
                // ledger; any other delivery costs this step nothing.
                let rows = crate::extraction_proof::recoverable(&input);
                if !rows.is_empty() {
                    let delivered = hooks::PreGenerateInput {
                        session_id: input.session_id.clone(),
                        turn_id: input.turn_id.clone(),
                        recall_context: input.recall_context.clone(),
                        ..Default::default()
                    };
                    spawn_import(&deps, invocation, delivered, rows);
                }
                // The hook never reads delivered history: only the persisted
                // ledger is authority.
                if let Some(context) = input.recall_context.as_mut().and_then(Value::as_object_mut)
                {
                    context.remove("extraction_history");
                }
                hooks::pre_generate(&deps, input).await
            }
        })
        .description(hooks::PRE_GENERATE_DESC)
        .metadata(json!({"internal":true})),
    );
}

/// Authenticate a delivery once and import its history off the hook path.
/// Rows the ledger already holds, or the last import dismissed because the
/// cursor had passed them, add nothing: no lookup, no path walk. A failed
/// import keeps earlier evidence.
// shortcut: a dismissed row is not re-checked if the cursor later leaves the
// active path (a branch switch), so it is not re-imported then; key the record
// on the cursor entry if that case shows up.
fn spawn_import(
    deps: &Arc<Deps>,
    invocation: Option<InvocationCtx>,
    delivered: hooks::PreGenerateInput,
    rows: Vec<crate::extraction_proof::Row>,
) {
    let mut imports = deps
        .hook_binding
        .import
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    imports.retain(|_, import| !import.is_finished());
    if imports.contains_key(&delivered.session_id) {
        return;
    }
    let session_id = delivered.session_id.clone();
    let deps = deps.clone();
    let import = tokio::spawn(async move {
        let store = deps.store().await;
        let held = crate::store::run_blocking(|| {
            let mut held = store.authenticated_history(&delivered.session_id)?;
            held.extend(store.dismissed_history(&delivered.session_id)?);
            Ok(held)
        })
        .await
        .unwrap_or_default();
        if rows.iter().all(|row| held.contains(row))
            || !deps
                .hook_binding
                .authenticates(&deps.iii, invocation.as_ref())
                .await
        {
            return;
        }
        let import = tokio::time::timeout(
            std::time::Duration::from_millis(1500),
            crate::extraction_proof::import(&deps, &delivered),
        )
        .await
        .unwrap_or_else(|_| Err("timed out".into()));
        if let Err(error) = import {
            tracing::warn!(session_id = %delivered.session_id, %error, "extraction history import failed");
        }
    });
    imports.insert(session_id, import);
}
