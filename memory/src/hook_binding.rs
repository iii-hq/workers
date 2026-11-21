//! Memory-owned seam registration. Payload metadata is never identity.
use crate::observed::{Delivered, delivered_event};
use crate::{deps::Deps, error::MemoryError, hooks};
use iii_sdk::errors::Error;
use iii_sdk::runtime::IIIConnectionState;
use iii_sdk::{
    CallRequest, IIIClient, InvocationCtx, ObserverHandle, ObserverState, RegisterFunction,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const SOURCE: &str = "harness::hook::on-pre-generate";

/// Disabled in fixtures unless the actual SDK registration is installed.
#[derive(Default)]
pub struct HookBinding {
    /// This worker's seam observers, by target.
    active: Mutex<HashMap<&'static str, ObserverHandle>>,
    /// The background history imports, by session. One per session at a
    /// time: its every step delivers the history again, so the next step
    /// retries a skipped delivery. Another session's import never skips it.
    import: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
}
impl HookBinding {
    pub fn install(&self, target: &'static str, handle: ObserverHandle) {
        self.active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(target, handle);
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
    /// Delivered by this worker's active observer of the seam `ctx` calls,
    /// never a direct call (an agent's own call carries no activation).
    pub(crate) fn matches(&self, ctx: &InvocationCtx, iii: &IIIClient) -> bool {
        iii.get_connection_state() == IIIConnectionState::Connected
            && iii.engine_supports_observe() == Some(true)
            && self
                .active
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(ctx.function_id.as_str())
                .is_some_and(|h| {
                    h.state() == ObserverState::Active
                        && ctx
                            .activation
                            .as_ref()
                            .is_some_and(|a| a.observer_id == h.id())
                })
    }
    pub async fn authenticates(&self, iii: &IIIClient, ctx: Option<&InvocationCtx>) -> bool {
        let Some(ctx) = ctx.filter(|ctx| self.matches(ctx, iii)) else {
            return false;
        };
        caller_hosts(iii, ctx, SOURCE).await == Some(true) && self.matches(ctx, iii)
    }
}

/// Whether the engine-stamped caller is the connected worker hosting
/// `function_id`; `None` when the engine did not answer. No owner cache:
/// each call gets bounded, fresh engine-owned host data. The second lookup
/// includes the function in that worker's actual surface.
async fn caller_hosts(iii: &IIIClient, ctx: &InvocationCtx, function_id: &str) -> Option<bool> {
    let Some(id) = ctx
        .caller
        .as_ref()
        .and_then(|caller| caller.worker_id.as_deref())
        .filter(|id| !id.is_empty())
    else {
        return Some(false);
    };
    let namespace = iii.namespace().unwrap_or_else(|| "default".into());
    let lookup = async {
        let info = iii
            .call(
                CallRequest::new(
                    "engine::functions::info",
                    json!({
                        "function_id":function_id, "namespace":namespace
                    }),
                )
                .metadata(json!({}))
                .timeout(std::time::Duration::from_millis(400)),
            )
            .await
            .ok()?;
        let name = match info["worker_name"].as_str() {
            Some(name) if info["function_id"] == function_id && info["namespace"] == namespace => {
                name
            }
            _ => return Some(false),
        };
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
        let owner = host["worker"]["id"].as_str().filter(|id| !id.is_empty());
        Some(
            owner == Some(id)
                && host["worker"]["status"] == "connected"
                && host["worker"]["namespace"] == namespace
                && host["functions"].as_array().is_some_and(|functions| {
                    functions
                        .iter()
                        .any(|f| f["function_id"] == function_id && f["namespace"] == namespace)
                }),
        )
    };
    tokio::time::timeout(std::time::Duration::from_millis(900), lookup)
        .await
        .ok()
        .flatten()
}

/// A seam call its own delivery did not make.
fn not_delivered(function_id: &str) -> Error {
    tracing::warn!(function_id, "seam call refused: not its own delivery");
    MemoryError::Forbidden(format!(
        "`{function_id}` runs only on its own delivery (this worker's observer), never on a \
         direct call"
    ))
    .into()
}

/// The internal seams, registered like every other function so their
/// invocations show up in traces, but marked internal. Used by the binary
/// AND SDK-loopback tests. Each reads its delivery synchronously in the
/// registered closure, before any async boundary: a direct call could
/// extract a session into any bank, or drop any session's extraction state.
pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    register_pre_generate(iii, deps);

    let d = deps.clone();
    iii.register_function(
        hooks::TURN_COMPLETED_FN,
        RegisterFunction::new_async(move |delivered: Delivered<hooks::TurnCompletedInput>| {
            let observed = iii_sdk::current_invocation()
                .is_some_and(|ctx| d.hook_binding.matches(&ctx, &d.iii));
            let d = d.clone();
            let input = delivered_event(delivered).unwrap_or_default();
            async move {
                if !observed {
                    return Err(not_delivered(hooks::TURN_COMPLETED_FN));
                }
                hooks::turn_completed(&d, input).await
            }
        })
        .description(hooks::TURN_COMPLETED_DESC)
        .metadata(json!({ "internal": true })),
    );

    let d = deps.clone();
    iii.register_function(
        hooks::EXTRACT_JOB_FN,
        RegisterFunction::new_async(move |input: hooks::ExtractJobInput| {
            // Whoever delivers it (the function queue's worker, the topic
            // observer), it runs only a job this worker queued and holds.
            let d = d.clone();
            async move { hooks::extract_job(&d, input).await }
        })
        .description(hooks::EXTRACT_JOB_DESC)
        .metadata(json!({ "internal": true })),
    );

    let d = deps.clone();
    iii.register_function(
        hooks::SESSION_DELETED_FN,
        RegisterFunction::new_async(move |delivered: Delivered<hooks::SessionDeletedInput>| {
            let observed = iii_sdk::current_invocation()
                .is_some_and(|ctx| d.hook_binding.matches(&ctx, &d.iii));
            let d = d.clone();
            let input = delivered_event(delivered).unwrap_or_default();
            async move {
                if !observed {
                    return Err(not_delivered(hooks::SESSION_DELETED_FN));
                }
                hooks::session_deleted(&d, input).await
            }
        })
        .description(hooks::SESSION_DELETED_DESC)
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );
}

/// Capture task-local identity synchronously in the registered closure,
/// before any async boundary.
fn register_pre_generate(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    let deps = deps.clone();
    iii.register_function(
        hooks::PRE_GENERATE_FN,
        RegisterFunction::new_async(move |mut input: hooks::PreGenerateInput| {
            let invocation = iii_sdk::current_invocation();
            let deps = deps.clone();
            async move {
                let observed = invocation
                    .as_ref()
                    .is_some_and(|ctx| deps.hook_binding.matches(ctx, &deps.iii));
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
                hooks::pre_generate(&deps, input, observed).await
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
