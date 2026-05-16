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
use crate::delivery::{
    handle_ack_delivered, handle_consume_undelivered, handle_flush_delivered, handle_list_pending,
    handle_list_undelivered, handle_sweep_session,
};
use crate::intercept::handle_intercept;
use crate::resolve::{handle_lookup_record, handle_resolve};
use crate::rules;
use crate::state::{
    rule_for, unverified_marker_targets, FunctionExecutor, IiiFunctionExecutor, IiiStateBus,
    StateBus,
};
use crate::sweeper::{spawn_timeout_sweeper, write_event, write_hook_reply};
use crate::wire::{extract_call, pending_key, Denial};

/// The iii function ids registered by [`register`]. Operators must not
/// alias these on any classifier — the boot guard logs a warning when
/// a misconfiguration is detected, see [`register`].
pub const FN_RESOLVE: &str = "approval::resolve";
pub const FN_LIST_PENDING: &str = "approval::list_pending";
pub const FN_LIST_UNDELIVERED: &str = "approval::list_undelivered";
pub const FN_CONSUME_UNDELIVERED: &str = "approval::consume_undelivered";
pub const FN_ACK_DELIVERED: &str = "approval::ack_delivered";
pub const FN_FLUSH_DELIVERED: &str = "approval::flush_delivered";
pub const FN_SWEEP_SESSION: &str = "approval::sweep_session";
pub const FN_LOOKUP_RECORD: &str = "approval::lookup_record";

/// Default `approval_state_scope` (matches [`WorkerConfig::default`]).
pub const STATE_SCOPE: &str = "approvals";

/// Handles returned from [`register`]; holding them keeps every iii
/// function registration and the background sweeper task alive.
pub struct Refs {
    pub resolve: FunctionRef,
    pub list_pending: FunctionRef,
    pub list_undelivered: FunctionRef,
    pub consume_undelivered: FunctionRef,
    pub ack_delivered: FunctionRef,
    pub flush_delivered: FunctionRef,
    pub sweep_session: FunctionRef,
    pub lookup_record: FunctionRef,
    pub subscriber_fn: FunctionRef,
    pub subscriber_trigger: iii_sdk::Trigger,
    /// Background task that flips expired pending records to `timed_out` and
    /// emits the corresponding `approval_resolved` events. Kept alive by
    /// virtue of being held here; aborts when the worker shuts down.
    pub sweeper: tokio::task::JoinHandle<()>,
}

pub fn register(iii: &III, cfg: &WorkerConfig) -> anyhow::Result<Refs> {
    let rules: Arc<Vec<InterceptorRule>> = Arc::new(cfg.interceptors.clone());
    // Layered policy rules consulted before the per-function interceptor
    // flow. Wrapped in RwLock so a user reply with `always: true` on
    // `approval::resolve` can push a new Allow rule at runtime (see the
    // cascade in `handle_resolve`). See [`crate::rules`].
    let policy_rules: Arc<RwLock<rules::Ruleset>> = Arc::new(RwLock::new(cfg.rules.clone()));

    // Fail fast on honor-system markers: any interceptor that asks the gate
    // to inject `__from_approval` MUST also assert the target validates it.
    // Without that assertion the marker is purely decorative and the gate
    // has no way to know whether bypass-through-direct-trigger is contained.
    let unverified = unverified_marker_targets(rules.as_slice());
    if !unverified.is_empty() {
        return Err(anyhow::anyhow!(
            "approval-gate: refusing to start — interceptors with inject_approval_marker=true \
             must also set marker_target_verified=true (target is asserted to validate \
             __from_approval against approval::lookup_record). Unverified: {unverified:?}"
        ));
    }

    for rule in rules.iter() {
        if let Some(cid) = rule.classifier.as_deref() {
            if cid == FN_LOOKUP_RECORD
                || cid == FN_RESOLVE
                || cid == FN_LIST_PENDING
                || cid == FN_LIST_UNDELIVERED
                || cid == FN_ACK_DELIVERED
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
    let exec_for_resolve: Arc<dyn FunctionExecutor> = Arc::new(IiiFunctionExecutor {
        iii: iii.clone(),
        rules: rules.clone(),
    });
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

    let bus_for_list_undelivered = bus.clone();
    let scope_list_undelivered = state_scope.clone();
    let list_undelivered = iii.register_function((
        RegisterFunctionMessage::with_id(FN_LIST_UNDELIVERED.into()).with_description(
            "Return resolved approval records for a session that haven't yet been stitched \
                 into an LLM turn. Lazy-flips expired pendings to timed_out."
                .into(),
        ),
        move |payload: Value| {
            let bus = bus_for_list_undelivered.clone();
            let scope = scope_list_undelivered.clone();
            async move {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                Ok::<_, IIIError>(
                    handle_list_undelivered(bus.as_ref(), &scope, payload, now_ms).await,
                )
            }
        },
    ));

    let bus_for_consume = bus.clone();
    let scope_consume = state_scope.clone();
    let consume_undelivered = iii.register_function((
        RegisterFunctionMessage::with_id(FN_CONSUME_UNDELIVERED.into()).with_description(
            "Atomic list+ack of resolved approval records. Returns the same FIFO-capped \
             slice as list_undelivered AND stamps each entry with delivered_in_turn_id \
             before returning. Required payload: {session_id, turn_id, limit?}."
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
                    handle_consume_undelivered(bus.as_ref(), &scope, payload, now_ms).await,
                )
            }
        },
    ));

    let bus_for_ack = bus.clone();
    let scope_ack = state_scope.clone();
    let ack_delivered = iii.register_function((
        RegisterFunctionMessage::with_id(FN_ACK_DELIVERED.into()).with_description(
            "Stamp delivered_in_turn_id on resolved approvals so they aren't replayed \
                 in subsequent turns. Idempotent."
                .into(),
        ),
        move |payload: Value| {
            let bus = bus_for_ack.clone();
            let scope = scope_ack.clone();
            async move {
                Ok::<_, IIIError>(handle_ack_delivered(bus.as_ref(), &scope, payload).await)
            }
        },
    ));

    let bus_for_flush = bus.clone();
    let scope_flush = state_scope.clone();
    let flush_delivered = iii.register_function((
        RegisterFunctionMessage::with_id(FN_FLUSH_DELIVERED.into()).with_description(
            "Stamp every unacked terminal approval record in a session as \
             delivered. One-shot operator recovery for backlog accumulation. \
             Required payload: {session_id, turn_id}."
                .into(),
        ),
        move |payload: Value| {
            let bus = bus_for_flush.clone();
            let scope = scope_flush.clone();
            async move {
                Ok::<_, IIIError>(handle_flush_delivered(bus.as_ref(), &scope, payload).await)
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

    let sweeper = spawn_timeout_sweeper(
        iii.clone(),
        bus.clone(),
        state_scope.clone(),
        cfg.sweeper_interval_ms,
    );

    Ok(Refs {
        resolve,
        list_pending,
        list_undelivered,
        consume_undelivered,
        ack_delivered,
        flush_delivered,
        sweep_session,
        lookup_record,
        subscriber_fn,
        subscriber_trigger,
        sweeper,
    })
}
