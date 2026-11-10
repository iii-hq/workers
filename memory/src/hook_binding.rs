//! Memory-owned pre-generate registration. Payload metadata is never identity.
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
}
impl HookBinding {
    pub fn install(&self, handle: ObserverHandle) {
        *self.active.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
    }
    pub fn install_completion(&self, handle: ObserverHandle) {
        *self.completion.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
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
        self.authenticates_source(iii, ctx, false).await
    }
    pub async fn authenticates_completion(
        &self,
        iii: &IIIClient,
        ctx: Option<&InvocationCtx>,
    ) -> bool {
        self.authenticates_source(iii, ctx, true).await
    }
    async fn authenticates_source(
        &self,
        iii: &IIIClient,
        ctx: Option<&InvocationCtx>,
        completion: bool,
    ) -> bool {
        let source = if completion {
            COMPLETION_SOURCE
        } else {
            SOURCE
        };
        let Some(ctx) = ctx.filter(|ctx| self.matches(ctx, iii, completion)) else {
            return false;
        };
        let Some(caller) = ctx.caller.as_ref() else {
            return false;
        };
        let Some(id) = caller.worker_id.as_deref().filter(|id| !id.is_empty()) else {
            return false;
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
            (host["worker"]["id"] == id
                && host["worker"]["status"] == "connected"
                && host["worker"]["namespace"] == namespace
                && functions
                    .iter()
                    .any(|f| f["function_id"] == source && f["namespace"] == namespace))
            .then_some(())
        };
        tokio::time::timeout(std::time::Duration::from_millis(900), lookup)
            .await
            .ok()
            .flatten()
            .is_some()
            && self.matches(ctx, iii, completion)
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
        RegisterFunction::new_async(move |delivery: CompletionDelivery| {
            let invocation = iii_sdk::current_invocation();
            let deps = deps.clone();
            async move {
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
                let trusted = correlated
                    && terminal
                    && deps
                        .hook_binding
                        .authenticates_completion(&deps.iii, invocation.as_ref())
                        .await;
                // Same authoritative own SessionMeta lookup used by extraction;
                // parent links, caller role and input kind never select eligibility.
                let meta = crate::scope::session_meta(&deps.iii, input.session_id.clone()).await;
                let Ok(Some(meta)) = meta else {
                    return Ok::<_, iii_sdk::errors::Error>(hooks::AckResponse { ok: true });
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
                    if trusted
                        && meta["session_id"] == input.session_id
                        && deps
                            .hook_binding
                            .authenticates_completion(&deps.iii, invocation.as_ref())
                            .await
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
                if deps
                    .hook_binding
                    .authenticates(&deps.iii, invocation.as_ref())
                    .await
                {
                    // Validation/persistence failures cannot acknowledge imported history.
                    if !matches!(
                        tokio::time::timeout(
                            std::time::Duration::from_millis(1500),
                            crate::extraction_proof::import(&deps, &input, invocation.as_ref())
                        )
                        .await,
                        Ok(Ok(()))
                    ) {
                        if let Some(context) =
                            input.recall_context.as_mut().and_then(Value::as_object_mut)
                        {
                            context.remove("extraction_history");
                        }
                    }
                } else if let Some(context) =
                    input.recall_context.as_mut().and_then(Value::as_object_mut)
                {
                    context.remove("extraction_history");
                }
                // The ordinary hook does not consume public history, even after
                // successful import. Only the persisted typed ledger is authority.
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
