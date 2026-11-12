//! Memory-owned pre-generate registration. Payload metadata is never identity.
use crate::observed::{delivered_event, Delivered};
use crate::{deps::Deps, hooks};
use iii_sdk::runtime::IIIConnectionState;
use iii_sdk::{
    CallRequest, IIIClient, InvocationCtx, ObserverHandle, ObserverState, RegisterFunction,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

pub const SOURCE: &str = "harness::hook::on-pre-generate";
pub const COMPLETION_SOURCE: &str = "harness::on-turn-completed";

/// Disabled in fixtures unless the actual SDK registration is installed.
#[derive(Default)]
pub struct HookBinding {
    active: Mutex<Option<ObserverHandle>>,
    completion: Mutex<Option<ObserverHandle>>,
    /// The background history import. One at a time: every step delivers
    /// the history again, so the next step retries a skipped delivery.
    import: Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl HookBinding {
    pub fn install(&self, handle: ObserverHandle) {
        *self.active.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
    }
    pub fn install_completion(&self, handle: ObserverHandle) {
        *self.completion.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
    }
    /// Wait for the background history import, if one runs.
    pub async fn import_settled(&self) {
        let import = self.import.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(import) = import {
            let _ = import.await;
        }
    }
    fn matches(&self, ctx: &InvocationCtx, iii: &IIIClient, completion: bool) -> bool {
        let (target, active) = if completion {
            (hooks::TURN_COMPLETED_FN, &self.completion)
        } else {
            (hooks::PRE_GENERATE_FN, &self.active)
        };
        iii.get_connection_state() == IIIConnectionState::Connected
            && iii.engine_supports_observe() == Some(true)
            && ctx.function_id == target
            && active
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
        self.authenticates_source(iii, ctx, false).await.is_some()
    }
    pub async fn authenticates_completion(
        &self,
        iii: &IIIClient,
        ctx: Option<&InvocationCtx>,
    ) -> Option<String> {
        self.authenticates_source(iii, ctx, true).await
    }
    async fn authenticates_source(
        &self,
        iii: &IIIClient,
        ctx: Option<&InvocationCtx>,
        completion: bool,
    ) -> Option<String> {
        let source = if completion {
            COMPLETION_SOURCE
        } else {
            SOURCE
        };
        let ctx = ctx.filter(|ctx| self.matches(ctx, iii, completion))?;
        let caller = ctx.caller.as_ref()?;
        // Captured calls are delivered by the engine on behalf of the source;
        // payload.caller is the source call's original caller, not its owner.
        // Manual pre-generate delivery still carries the actual source host.
        let id = if completion {
            if caller.worker_id.is_some()
                || caller.worker_name.as_deref() != Some("iii-engine")
                || caller.function_id.as_deref() != Some(source)
            {
                return None;
            }
            None
        } else {
            Some(caller.worker_id.as_deref().filter(|id| !id.is_empty())?)
        };
        let namespace = iii.namespace().unwrap_or_else(|| "default".into());
        let lookup = async {
            let info = iii
                .call(
                    CallRequest::new(
                        "engine::functions::info",
                        json!({
                            "function_id":source, "namespace":namespace
                        }),
                    )
                    .metadata(json!({}))
                    .timeout(std::time::Duration::from_millis(400)),
                )
                .await
                .ok()?;
            let name = info["worker_name"].as_str()?;
            if info["function_id"] != source
                || info["namespace"] != namespace
                || (completion && info["private"] != true)
            {
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
            (id.is_none_or(|id| owner == id)
                && host["worker"]["status"] == "connected"
                && host["worker"]["namespace"] == namespace
                && functions
                    .iter()
                    .any(|f| f["function_id"] == source && f["namespace"] == namespace))
            .then(|| owner.to_owned())
        };
        let owner = tokio::time::timeout(std::time::Duration::from_millis(900), lookup)
            .await
            .ok()
            .flatten()?;
        self.matches(ctx, iii, completion).then_some(owner)
    }
}

/// Fields read from Harness's actual TurnCompletedEvent; no invented digest or status.
#[derive(serde::Deserialize, schemars::JsonSchema)]
struct CompletionDelivery {
    #[serde(flatten)]
    input: hooks::TurnCompletedInput,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    terminal: Option<bool>,
}

/// The binary and loopback use this exact receiver. Invocation identity is SDK
/// task-local metadata, not data fields. Missing bindings remain disabled.
pub fn register_turn_completed(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    let deps = deps.clone();
    iii.register_function(
        hooks::TURN_COMPLETED_FN,
        RegisterFunction::new_async(move |delivered: Delivered<CompletionDelivery>| {
            let invocation = iii_sdk::current_invocation();
            let deps = deps.clone();
            async move {
                let Some(delivery) = delivered_event(delivered) else {
                    return Ok::<_, iii_sdk::errors::Error>(hooks::AckResponse { ok: true });
                };
                let input = delivery.input;
                // Preserve the existing sub-agent/empty-session path, including
                // its turn-bank cleanup, before any new authority lookup.
                // These claims suppress work; they never prove a trusted skip.
                if input.session_id.is_empty()
                    || input.parent.is_some()
                    || input.parent_session_id.is_some()
                {
                    return hooks::turn_completed(&deps, input).await;
                }
                let correlated = crate::extraction_proof::bounded(&input.session_id)
                    && input
                        .turn_id
                        .as_deref()
                        .is_some_and(crate::extraction_proof::bounded);
                let terminal = delivery.terminal == Some(true)
                    && matches!(
                        delivery.status.as_deref(),
                        Some("completed" | "cancelled" | "failed")
                    );
                // Same authoritative own SessionMeta lookup used by extraction;
                // parent links, caller role and input kind never select eligibility.
                let meta =
                    match crate::scope::session_meta(&deps.iii, input.session_id.clone()).await {
                        Ok(Some(meta)) => meta,
                        Ok(None) => return Ok(hooks::AckResponse { ok: true }),
                        // A failed lookup proves nothing: the job checks the kind
                        // again, and the queue retries it.
                        Err(_) => return hooks::turn_completed(&deps, input).await,
                    };
                if meta.get("kind").is_some_and(|kind| !kind.is_string()) {
                    return Ok(hooks::AckResponse { ok: true });
                }
                let kind = meta.get("kind").and_then(Value::as_str);
                let non_user = match kind {
                    Some("automation") => Some(crate::events::SkippedSessionKind::Automation),
                    Some("e2e") => Some(crate::events::SkippedSessionKind::E2e),
                    Some("user") | None if meta.is_object() => None,
                    _ => return Ok(hooks::AckResponse { ok: true }),
                };
                if let Some(kind) = non_user {
                    if let Some(turn) = input.turn_id.as_deref() {
                        crate::scope::take_turn_bank(turn);
                    }
                    if correlated
                        && terminal
                        && meta["session_id"] == input.session_id
                        && deps
                            .hook_binding
                            .authenticates_completion(&deps.iii, invocation.as_ref())
                            .await
                            .is_some()
                    {
                        deps.emitter
                            .extraction_skipped(
                                &input.session_id,
                                input.turn_id.as_deref().unwrap(),
                                kind,
                            )
                            .await;
                    }
                    return Ok(hooks::AckResponse { ok: true });
                }
                hooks::turn_completed(&deps, input).await
            }
        })
        .description(hooks::TURN_COMPLETED_DESC)
        .metadata(json!({"internal":true})),
    );
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
/// Rows the ledger already holds add nothing: no lookup, no path walk. A
/// failed import keeps earlier evidence.
fn spawn_import(
    deps: &Arc<Deps>,
    invocation: Option<InvocationCtx>,
    delivered: hooks::PreGenerateInput,
    rows: Vec<crate::extraction_proof::Row>,
) {
    let mut slot = deps
        .hook_binding
        .import
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if slot.as_ref().is_some_and(|import| !import.is_finished()) {
        return;
    }
    let deps = deps.clone();
    // shortcut: a row the cursor already passed is dropped, not held, so each
    // later delivery of it walks the path again until the harness stops
    // sending it; remember dismissed rows if that shows in engine load.
    *slot = Some(tokio::spawn(async move {
        let store = deps.store().await;
        let stored =
            crate::store::run_blocking(|| store.authenticated_history(&delivered.session_id))
                .await
                .unwrap_or_default();
        if rows.iter().all(|row| stored.contains(row))
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
    }));
}
