//! Subscriptions — register an ephemeral iii trigger and be notified when it
//! fires, instead of polling (harness.md § Subscriptions). The agent calls
//! `engine::register_trigger` / `engine::unregister_trigger`; the harness
//! intercepts those calls (see [`invoke`]) so the trusted owning session,
//! `harness::notify_agent` target, and subscription metadata are injected, and
//! teardown stays owner-checked — the agent can never supply those.

use iii_sdk::{TriggerAction, TriggerRequest};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::clients::EngineClient;
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::policy::CompiledPolicy;
use crate::subscriptions::{self, NOTIFY_AGENT_ID};
use crate::trigger::{self, ResultData};
use crate::types::content::ContentBlock;

/// The engine function the agent calls to subscribe. The harness intercepts it
/// (the agent never reaches the raw engine registrar) so it can stamp the
/// trusted session and bind the trigger to `harness::notify_agent`.
pub const REGISTER_TRIGGER_ID: &str = "engine::register_trigger";

/// The engine function the agent calls to unsubscribe. The harness intercepts it
/// so it resolves the caller's subscription, enforces ownership, and unregisters
/// the underlying engine trigger.
pub const UNREGISTER_TRIGGER_ID: &str = "engine::unregister_trigger";

/// Agent-facing subscription contract.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(rename = "SubscribeArgs")]
pub struct SubscribeRequest {
    /// The iii trigger type to listen on: `cron`, `state`, `stream`, or another
    /// worker's custom trigger type (e.g. `approval::pending-resolved`). For an
    /// ad-hoc signal, subscribe to `state` on a key and have the signaller call
    /// `state::set` on it (no dedicated emit needed — the engine fans the trigger
    /// out to every subscriber).
    pub trigger_type: String,
    /// The trigger config, passed verbatim to the engine — e.g.
    /// `{ "expression": "0 */5 * * * *" }` for cron, or a `state` scope/key.
    #[serde(default)]
    pub config: Value,
    /// A short human label echoed back in the notification text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Auto-unsubscribe after the first delivered notification. Defaults to true
    /// for one-shot-ish types (state / stream / custom trigger types), false for
    /// recurring `cron`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubscribeResponse {
    pub subscription_id: String,
    /// The effective `once` flag applied (after the per-type default).
    pub once: bool,
}

/// The single per-call invocation chokepoint. Subscription control calls
/// (`engine::register_trigger` / `engine::unregister_trigger`) are handled inline
/// with the trusted owning session injected — the model can never widen the
/// target; everything else invokes the target normally. Every call site (the
/// turn loop, `harness::function::trigger`, and the hook-held release path) routes
/// through here so the trusted injection can't be bypassed.
pub async fn invoke(
    deps: &Deps,
    engine: &EngineClient,
    policy: &CompiledPolicy,
    function_id: &str,
    arguments: &Value,
    session_id: &str,
) -> ResultData {
    match function_id {
        REGISTER_TRIGGER_ID => intercept_register(deps, arguments, session_id).await,
        UNREGISTER_TRIGGER_ID => intercept_unregister(deps, arguments, session_id).await,
        _ => trigger::invoke_target(engine, policy, function_id, arguments).await,
    }
}

/// Whether a trigger type recurs by nature (so `once` defaults to false).
fn defaults_recurring(trigger_type: &str) -> bool {
    trigger_type == "cron"
}

/// Run an agent `engine::register_trigger` call as a subscription: deserialize
/// the agent args, then bind to `harness::notify_agent` with the trusted owning
/// session stored in the local registry.
async fn intercept_register(deps: &Deps, args: &Value, session_id: &str) -> ResultData {
    let req: SubscribeRequest = match serde_json::from_value(args.clone()) {
        Ok(r) => r,
        Err(e) => return error_result(format!("invalid subscribe arguments: {e}")),
    };

    match handle(deps, req, session_id).await {
        Ok(resp) => ok_result(&resp),
        Err(e) => error_result(e.to_string()),
    }
}

/// Run an agent `engine::unregister_trigger` call as a subscription teardown.
/// The `id` is the `subscription_id` the agent received from subscribing; it is
/// owner-checked and mapped to the underlying engine trigger (so the agent can
/// never unregister an arbitrary engine trigger). Idempotent.
async fn intercept_unregister(deps: &Deps, args: &Value, session_id: &str) -> ResultData {
    let Some(id) = args.get("id").and_then(Value::as_str) else {
        return error_result("engine::unregister_trigger requires an `id`".to_string());
    };

    // Owner check: only the owning session may tear down its subscription.
    if let Some(owner) = deps.subscriptions.session_of(id) {
        if owner != session_id {
            return error_result("subscription belongs to a different session".to_string());
        }
    }

    // Drop the local entry; unregister the engine trigger (which also drops its
    // proxy). Idempotent: a missing entry reports removed: false.
    let removed = match deps.subscriptions.take(id) {
        Some((_session, trigger_id)) => {
            if let Some(trigger_id) = trigger_id {
                unregister_engine_trigger(deps, &trigger_id).await;
            }
            true
        }
        None => false,
    };
    ok_result(&json!({ "removed": removed }))
}

async fn handle(
    deps: &Deps,
    req: SubscribeRequest,
    session_id: &str,
) -> Result<SubscribeResponse, HarnessError> {
    if subscriptions::is_forbidden_trigger_type(&req.trigger_type) {
        return Err(HarnessError::InvalidRequest(format!(
            "cannot bind harness-internal trigger type `{}` (self-notification guard)",
            req.trigger_type
        )));
    }

    let once = req.once.unwrap_or(!defaults_recurring(&req.trigger_type));

    let sub_id = format!("sub_{}", uuid::Uuid::new_v4().simple());

    // Enforce the per-session cap and reserve the local entry BEFORE binding, so
    // an immediate fire still finds it. Atomic check-and-insert (no race).
    deps.subscriptions
        .try_insert(
            &sub_id,
            session_id,
            once,
            req.label.clone(),
            subscriptions::MAX_SUBSCRIPTIONS_PER_SESSION,
        )
        .map_err(|_| {
            HarnessError::InvalidRequest(format!(
                "subscription cap reached ({} active for this session); unsubscribe first",
                subscriptions::MAX_SUBSCRIPTIONS_PER_SESSION
            ))
        })?;

    // The engine binds the trigger to the shared `harness::notify_agent` via a
    // proxy that round-trips this metadata into the fired payload.
    let metadata = json!({
        "subscription_id": sub_id,
    });

    let resp = deps
        .iii
        .trigger(TriggerRequest {
            function_id: REGISTER_TRIGGER_ID.to_string(),
            payload: json!({
                "trigger_type": req.trigger_type,
                "function_id": NOTIFY_AGENT_ID,
                "config": req.config,
                "metadata": metadata,
            }),
            action: None,
            timeout_ms: Some(deps.cfg().await.dispatch_timeout_ms),
        })
        .await;

    match resp
        .ok()
        .and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_string))
    {
        Some(trigger_id) => {
            // Attach the engine-returned id. If the entry is already gone (a
            // `once` fire won the bind window), unregister the orphan trigger.
            if !deps.subscriptions.set_trigger_id(&sub_id, &trigger_id) {
                unregister_engine_trigger(deps, &trigger_id).await;
            }
        }
        None => {
            deps.subscriptions.take(&sub_id); // unwind the reserved entry
            return Err(HarnessError::Dependency(format!(
                "{REGISTER_TRIGGER_ID} `{}` failed",
                req.trigger_type
            )));
        }
    }

    Ok(SubscribeResponse {
        subscription_id: sub_id,
        once,
    })
}

pub async fn unregister_engine_trigger(deps: &Deps, trigger_id: &str) {
    if let Err(e) = deps
        .iii
        .trigger(TriggerRequest {
            function_id: UNREGISTER_TRIGGER_ID.to_string(),
            payload: json!({ "id": trigger_id }),
            action: Some(TriggerAction::Void),
            timeout_ms: None,
        })
        .await
    {
        tracing::warn!(trigger_id, error = %e, "subscription trigger unregister failed");
    }
}

/// A normalised success result whose `details` is `value` rendered as JSON text.
fn ok_result<T: Serialize>(value: &T) -> ResultData {
    let details = serde_json::to_value(value).unwrap_or(Value::Null);
    ResultData {
        content: vec![ContentBlock::text(
            serde_json::to_string(&details).unwrap_or_default(),
        )],
        is_error: false,
        details,
    }
}

fn error_result(msg: String) -> ResultData {
    ResultData {
        content: vec![ContentBlock::text(msg.clone())],
        is_error: true,
        details: json!({ "error": msg }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn once_defaults_one_shot_for_non_recurring() {
        assert!(!defaults_recurring("state"));
        assert!(!defaults_recurring("approval::pending-resolved"));
        assert!(defaults_recurring("cron"));
    }

    #[test]
    fn forbids_binding_harness_internal_trigger_types() {
        assert!(subscriptions::is_forbidden_trigger_type(
            "harness::turn-completed"
        ));
        assert!(!subscriptions::is_forbidden_trigger_type("state"));
        assert!(!subscriptions::is_forbidden_trigger_type("cron"));
    }
}
