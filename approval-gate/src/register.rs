//! iii function/trigger wiring.
//!
//! [`register`] is the entry point the binary calls at startup. It
//! constructs the shared `StateBus` + `FunctionExecutor`, hooks every
//! `approval::*` function id, registers the `policy::approval_gate`
//! subscriber on the configured topic, spawns the timeout sweeper, and
//! returns a [`Refs`] handle whose contents keep all the function
//! registrations and the sweeper task alive for the worker's lifetime.
//!
//! The subscriber closure is the only piece of non-trivial logic in
//! this module — it composes the three decision layers documented in
//! [`crate::intercept`] and writes the resulting hook reply onto the
//! envelope's reply stream.

use std::sync::{Arc, RwLock};

use iii_sdk::{
    FunctionRef, IIIError, RegisterFunctionMessage, RegisterTriggerInput, TriggerRequest, III,
};
use serde_json::{json, Value};

use crate::config::{InterceptorRule, WorkerConfig};
use crate::delivery::{handle_consume, handle_list_pending, handle_sweep_session};
use crate::intercept::handle_intercept;
use crate::resolve::{handle_lookup_record, handle_resolve};
use crate::rules;
use crate::state::{FunctionExecutor, IiiFunctionExecutor, IiiStateBus, StateBus};
use crate::wire::{extract_call, pending_key};

/// The iii function ids registered by [`register`].
pub const FN_RESOLVE: &str = "approval::resolve";
pub const FN_LIST_PENDING: &str = "approval::list_pending";
pub const FN_CONSUME: &str = "approval::consume";
pub const FN_SWEEP_SESSION: &str = "approval::sweep_session";
pub const FN_LOOKUP_RECORD: &str = "approval::lookup_record";

/// Default `approval_state_scope` (matches [`WorkerConfig::default`]).
pub const STATE_SCOPE: &str = "approvals";

/// Handles returned from [`register`]; holding them keeps every iii
/// function registration alive for the worker's lifetime. The 2-second
/// background sweeper task is gone — timeouts now flip lazily on read.
pub struct Refs {
    pub resolve: FunctionRef,
    pub list_pending: FunctionRef,
    pub consume: FunctionRef,
    pub sweep_session: FunctionRef,
    pub lookup_record: FunctionRef,
    pub subscriber_fn: FunctionRef,
    pub subscriber_trigger: iii_sdk::Trigger,
}

pub fn register(iii: &III, cfg: &WorkerConfig) -> anyhow::Result<Refs> {
    // Layered policy ruleset, wrapped in RwLock so cascade-on-`always:true`
    // can push a runtime Allow rule (see resolve.rs::cascade_allow_for_session).
    let policy_rules: Arc<RwLock<rules::Ruleset>> = Arc::new(RwLock::new(cfg.rules.clone()));

    // No-op alias-warning loop kept as a no-op for backward source
    // compatibility (no interceptors are configured anymore). Empty vec
    // so the loop body never runs.
    let rules: Arc<Vec<InterceptorRule>> = Arc::new(Vec::new());
    for rule in rules.iter() {
        if let Some(cid) = rule.classifier.as_deref() {
            if cid == FN_LOOKUP_RECORD
                || cid == FN_RESOLVE
                || cid == FN_LIST_PENDING
                || cid == FN_CONSUME
                || cid == FN_SWEEP_SESSION
            {
                tracing::warn!(
                    "approval-gate: interceptor for {:?} uses classifier {:?} which aliases an approval endpoint; fix config",
                    rule.function_id,
                    cid
                );
            }
        }
    }

    let bus: Arc<dyn StateBus> = Arc::new(IiiStateBus(iii.clone()));
    let timeout_ms = cfg.default_timeout_ms;
    let topic = cfg.topic.clone();
    let state_scope = cfg.approval_state_scope.clone();

    let bus_for_resolve = bus.clone();
    let scope_resolve = state_scope.clone();
    let exec_for_resolve: Arc<dyn FunctionExecutor> =
        Arc::new(IiiFunctionExecutor { iii: iii.clone() });
    let iii_for_resolve = iii.clone();
    let policy_rules_for_resolve = policy_rules.clone();
    let resolve = iii.register_function((
        RegisterFunctionMessage::with_id(FN_RESOLVE.into()).with_description(
            "Resolve a pending approval. On allow, invokes the underlying function; \
                     on deny, records the denial. With `always: true` on an allow reply, \
                     a runtime rule is added so future calls to this function id auto-allow, \
                     and the session's other pending calls newly matching are cascade-resolved. \
                     The result is stitched into the agent's next turn as a system message."
                .into(),
        ),
        move |payload: Value| {
            let bus = bus_for_resolve.clone();
            let exec = exec_for_resolve.clone();
            let scope_resolve = scope_resolve.clone();
            let iii = iii_for_resolve.clone();
            let policy_rules = policy_rules_for_resolve.clone();
            async move {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let resp = handle_resolve(
                    bus.as_ref(),
                    exec.as_ref(),
                    &scope_resolve,
                    &policy_rules,
                    payload.clone(),
                    now_ms,
                )
                .await;

                if resp.get("ok").and_then(Value::as_bool) == Some(true) {
                    let session_id = payload
                        .get("session_id")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let call_id = payload
                        .get("function_call_id")
                        .or_else(|| payload.get("tool_call_id"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if !session_id.is_empty() && !call_id.is_empty() {
                        let key = pending_key(session_id, call_id);
                        if let Some(final_rec) = bus.get(&scope_resolve, &key).await {
                            let mut evt = json!({
                                "type": "approval_resolved",
                                "function_call_id": call_id,
                                "tool_call_id": call_id,
                            });
                            if let Some(status) = final_rec.get("status").and_then(Value::as_str) {
                                evt["decision"] = match status {
                                    "executed" | "approved" => json!("allow"),
                                    _ => json!("deny"),
                                };
                                evt["status"] = json!(status);
                            }
                            if let Some(r) = final_rec.get("result") {
                                evt["result"] = json!(r);
                            }
                            if let Some(e) = final_rec.get("error") {
                                evt["error"] = json!(e);
                            }
                            if let Some(denial) = final_rec.get("denial") {
                                evt["denial"] = denial.clone();
                            }
                            write_event(&iii, session_id, &evt).await;
                        }
                    }
                }
                Ok::<_, IIIError>(resp)
            }
        },
    ));

    let bus_for_list = bus.clone();
    let scope_list = state_scope.clone();
    let list_pending = iii.register_function((
        RegisterFunctionMessage::with_id(FN_LIST_PENDING.into())
            .with_description("Return pending approvals for a session.".into()),
        move |payload: Value| {
            let bus = bus_for_list.clone();
            let scope_list = scope_list.clone();
            async move {
                Ok::<_, IIIError>(handle_list_pending(bus.as_ref(), &scope_list, payload).await)
            }
        },
    ));

    let bus_for_consume = bus.clone();
    let scope_consume = state_scope.clone();
    let consume = iii.register_function((
        RegisterFunctionMessage::with_id(FN_CONSUME.into()).with_description(
            "Atomic drain: returns Done rows for a session and deletes them in the \
             same call. Pending and InFlight rows stay in state. Pending rows past \
             expires_at are lazy-flipped to Done(TimedOut) before return. \
             Required payload: {session_id, limit?}. Response: {ok, entries, omitted}."
                .into(),
        ),
        move |payload: Value| {
            let bus = bus_for_consume.clone();
            let scope = scope_consume.clone();
            async move {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                Ok::<_, IIIError>(
                    handle_consume(bus.as_ref(), &scope, payload, now_ms).await,
                )
            }
        },
    ));

    let bus_for_sweep = bus.clone();
    let scope_sweep = state_scope.clone();
    let sweep_session = iii.register_function((
        RegisterFunctionMessage::with_id(FN_SWEEP_SESSION.into()).with_description(
            "Sweep all pending approvals for a session to timed_out. \
                 Called when a session is deleted."
                .into(),
        ),
        move |payload: Value| {
            let bus = bus_for_sweep.clone();
            let scope = scope_sweep.clone();
            async move {
                Ok::<_, IIIError>(handle_sweep_session(bus.as_ref(), &scope, payload).await)
            }
        },
    ));

    let bus_for_lookup = bus.clone();
    let scope_lookup = state_scope.clone();
    let lookup_record = iii.register_function((
        RegisterFunctionMessage::with_id(FN_LOOKUP_RECORD.into()).with_description(
            "Return the approval state-store record for a session/function_call_id pair; \
                 null when absent. Used by shell bypass validation."
                .into(),
        ),
        move |payload: Value| {
            let bus = bus_for_lookup.clone();
            let scope = scope_lookup.clone();
            async move {
                Ok::<_, IIIError>(handle_lookup_record(bus.as_ref(), &scope, payload).await)
            }
        },
    ));

    let iii_for_sub = iii.clone();
    let bus_for_sub = bus.clone();
    let subscriber_scope = state_scope.clone();
    let rules_for_sub = rules.clone();
    let policy_rules_for_sub = policy_rules.clone();
    let subscriber_fn = iii.register_function((
        RegisterFunctionMessage::with_id("policy::approval_gate".into())
            .with_description("Pause function calls listed in approval_required.".into()),
        move |envelope: Value| {
            let iii = iii_for_sub.clone();
            let bus = bus_for_sub.clone();
            let sc = subscriber_scope.clone();
            let intercept_rules = rules_for_sub.clone();
            let policy_rules = policy_rules_for_sub.clone();
            async move {
                let Some(call) = extract_call(&envelope) else {
                    return Ok::<_, IIIError>(json!({ "block": false }));
                };
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);

                // Take a snapshot of the rules ruleset under the read lock,
                // then drop the guard before any .await. std::sync::RwLock
                // is not async-safe to hold across suspension points, and
                // a held guard would block every concurrent intercept.
                let rules_snapshot: rules::Ruleset = {
                    let guard = policy_rules
                        .read()
                        .expect("approval-gate policy rules lock poisoned");
                    guard.clone()
                };

                // One decision call. Verdict::Allow → {block:false}.
                // Verdict::Deny → {block:true, denial:Policy{...}}.
                // Verdict::Ask → write Pending + reply {block:true, status:pending}.
                let reply = handle_intercept(
                    bus.as_ref(),
                    &sc,
                    &call,
                    &rules_snapshot,
                    now_ms,
                    timeout_ms,
                ).await;

                if reply.get("status").and_then(Value::as_str) == Some("pending") {
                    write_event(
                        &iii,
                        &call.session_id,
                        &json!({
                            "type": "approval_requested",
                            "function_call_id": call.function_call_id,
                            "tool_call_id": call.function_call_id,
                            "function_id": call.function_id,
                            "tool_name": call.function_id,
                            "args": call.args,
                            "expires_at": now_ms.saturating_add(timeout_ms),
                        }),
                    )
                    .await;
                }
                write_hook_reply(&iii, &call.reply_stream, &call.event_id, &reply).await;
                Ok(reply)
            }
        },
    ));

    let subscriber_trigger = iii
        .register_trigger(RegisterTriggerInput {
            trigger_type: "durable:subscriber".into(),
            function_id: "policy::approval_gate".into(),
            config: json!({ "topic": topic }),
            metadata: None,
        })
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    Ok(Refs {
        resolve,
        list_pending,
        consume,
        sweep_session,
        lookup_record,
        subscriber_fn,
        subscriber_trigger,
    })
}

// ─────────────────────────────────────────────────────────────────────────
// Inline stream helpers (used by the subscriber to write the
// `approval_requested` stream frame and the hook reply). These used to
// live in `sweeper.rs` but that file is gone now that the background
// polling task is deleted; the helpers move here as their only consumer.
// ─────────────────────────────────────────────────────────────────────────

pub(crate) fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0);
    let n = C.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{t:x}-{n:x}")
}

/// Append `event` to the `agent::events` stream for `session_id`. Fire-
/// and-forget: errors are swallowed because the persisted record is the
/// source of truth — orchestrators re-derive state from
/// `approval::consume` if a frame is lost.
pub(crate) async fn write_event(iii: &III, session_id: &str, event: &Value) {
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "stream::set".into(),
            payload: json!({
                "stream_name": "agent::events",
                "group_id": session_id,
                "item_id": format!("approval-{}", uuid_like()),
                "data": event,
            }),
            action: None,
            timeout_ms: None,
        })
        .await;
}

/// Append a hook reply onto `stream_name` keyed by `event_id`. No-op when
/// either id is empty so a malformed envelope can't crash the gate.
pub(crate) async fn write_hook_reply(iii: &III, stream_name: &str, event_id: &str, reply: &Value) {
    if stream_name.is_empty() || event_id.is_empty() {
        return;
    }
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "stream::set".into(),
            payload: json!({
                "stream_name": stream_name,
                "group_id": event_id,
                "item_id": uuid_like(),
                "data": reply,
            }),
            action: None,
            timeout_ms: None,
        })
        .await;
}
