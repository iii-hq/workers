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

use crate::config::WorkerConfig;
use crate::delivery::{
    handle_consume, handle_list_pending, handle_sweep_session, handle_tick_timeouts,
};
use crate::intercept::handle_intercept;
use crate::resolve::{handle_lookup_record, handle_resolve};
use crate::rules::{self, LayeredRules};
use crate::state::{FunctionExecutor, IiiFunctionExecutor, IiiStateBus, StateBus};
use crate::wire::{parse_call, pending_key, state_error_reply, ExtractCallError, IncomingCall};

/// The iii function ids registered by [`register`].
pub const FN_RESOLVE: &str = "approval::resolve";
pub const FN_LIST_PENDING: &str = "approval::list_pending";
pub const FN_CONSUME: &str = "approval::consume";
pub const FN_SWEEP_SESSION: &str = "approval::sweep_session";
pub const FN_LOOKUP_RECORD: &str = "approval::lookup_record";
/// Watchdog surface that reclaims expired Pending rows and stale InFlight
/// rows across all sessions, then nudges the orchestrator via `run::resume`
/// so the LLM's `pending_approval` placeholders get replaced with a
/// `timed_out` outcome (finding #4). Intended to be driven by a periodic
/// timer; can also be called on demand.
pub const FN_TICK_TIMEOUTS: &str = "approval::tick_timeouts";

/// Default `approval_state_scope` (matches [`WorkerConfig::default`]).
pub const STATE_SCOPE: &str = "approvals";

/// Handles returned from [`register`]; holding them keeps every iii
/// function registration and the optional timeout tick task alive for
/// the worker's lifetime.
pub struct Refs {
    pub resolve: FunctionRef,
    pub list_pending: FunctionRef,
    pub consume: FunctionRef,
    pub sweep_session: FunctionRef,
    pub lookup_record: FunctionRef,
    pub tick_timeouts: FunctionRef,
    pub subscriber_fn: FunctionRef,
    pub subscriber_trigger: iii_sdk::Trigger,
    /// Background loop that fires `handle_tick_timeouts` on a fixed
    /// interval (finding #4 auto-tick). `None` when
    /// `cfg.tick_interval_ms == 0` (tests disable the loop so they can
    /// drive ticks manually). Dropped with `Refs` → loop aborts.
    pub tick_task: Option<TickTaskGuard>,
}

/// RAII guard around the auto-tick `tokio::task::JoinHandle`. Aborting on
/// drop keeps tests from leaking background tasks between cases.
pub struct TickTaskGuard(tokio::task::JoinHandle<()>);

impl Drop for TickTaskGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub fn register(iii: &III, cfg: &WorkerConfig) -> anyhow::Result<Refs> {
    // Layered policy ruleset: a YAML-loaded global baseline + a per-session
    // overlay updated at runtime by `cascade_allow_for_session`. Wrapped
    // in RwLock so the cascade can push session-scoped Allow rules
    // without blocking concurrent intercepts (see finding #2).
    let policy_rules: Arc<RwLock<LayeredRules>> =
        Arc::new(RwLock::new(LayeredRules::from_global(cfg.rules.clone())));

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
                        // 1. Originator's `approval_resolved` event.
                        emit_approval_resolved(
                            &iii,
                            bus.as_ref(),
                            &scope_resolve,
                            session_id,
                            call_id,
                        )
                        .await;

                        // 2. One `approval_resolved` per cascade-resolved
                        //    call (finding #2-cascade follow-up). Without
                        //    these the UI cards for auto-resolved rows
                        //    stay clickable until something else refreshes
                        //    the pending list.
                        if let Some(ids) = resp.get("cascaded_call_ids").and_then(Value::as_array) {
                            for cid in ids.iter().filter_map(Value::as_str) {
                                emit_approval_resolved(
                                    &iii,
                                    bus.as_ref(),
                                    &scope_resolve,
                                    session_id,
                                    cid,
                                )
                                .await;
                            }
                        }

                        // 3. Wake the orchestrator so it drains the resolved
                        // record into the next assistant turn. Pending
                        // results carry `terminate: true`, so without this
                        // re-kick the conversation would stall after the
                        // operator clicks allow/deny.
                        publish_turn_step(&IIIWaker { iii: &iii }, session_id).await;
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
                Ok::<_, IIIError>(handle_consume(bus.as_ref(), &scope, payload, now_ms).await)
            }
        },
    ));

    let bus_for_sweep = bus.clone();
    let scope_sweep = state_scope.clone();
    let sweep_session =
        iii.register_function((
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
    let lookup_record =
        iii.register_function((
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

    // Watchdog: reclaim expired Pending + stale InFlight rows and wake
    // the orchestrator for any affected sessions (finding #4). The
    // function is callable on demand AND fired periodically by the
    // background loop spawned below (when tick_interval_ms > 0).
    let bus_for_tick = bus.clone();
    let scope_tick = state_scope.clone();
    let iii_for_tick = iii.clone();
    let tick_timeouts = iii.register_function((
        RegisterFunctionMessage::with_id(FN_TICK_TIMEOUTS.into()).with_description(
            "Watchdog tick: flips expired Pending rows and stale InFlight rows to \
             Done(TimedOut), then calls run::resume for each affected session so \
             the orchestrator stitches the timed_out outcome into the LLM turn. \
             Fires automatically every `tick_interval_ms` (config); also callable \
             on demand. Payload: {} or {scope_prefix: \"<session_id>/\"} for \
             session-scoped ticks. Response: {ok, reclaimed: N, sessions_woken: \
             [\"sess_a\", ...]}."
                .into(),
        ),
        move |payload: Value| {
            let bus = bus_for_tick.clone();
            let scope = scope_tick.clone();
            let iii = iii_for_tick.clone();
            async move {
                let waker = IIIWaker { iii: &iii };
                Ok::<_, IIIError>(run_tick(&waker, bus.as_ref(), &scope, payload).await)
            }
        },
    ));

    // Auto-tick loop. Calls `run_tick` directly (NOT via iii.trigger) so
    // it keeps working if the bus is the thing that's down, and so it
    // doesn't show up as orchestrator-driven traffic in observability.
    let tick_task = if cfg.tick_interval_ms > 0 {
        let interval = std::time::Duration::from_millis(cfg.tick_interval_ms);
        let bus_for_loop = bus.clone();
        let scope_for_loop = state_scope.clone();
        let iii_for_loop = iii.clone();
        let handle = tokio::spawn(async move {
            // First fire after one interval — give the worker time to
            // finish boot before adding bus traffic.
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await; // first tick fires immediately; absorb it
            loop {
                ticker.tick().await;
                let waker = IIIWaker { iii: &iii_for_loop };
                let _ = run_tick(&waker, bus_for_loop.as_ref(), &scope_for_loop, json!({})).await;
            }
        });
        Some(TickTaskGuard(handle))
    } else {
        tracing::info!("approval-gate: tick_interval_ms=0 — auto-tick disabled");
        None
    };

    let iii_for_sub = iii.clone();
    let bus_for_sub = bus.clone();
    let subscriber_scope = state_scope.clone();
    let policy_rules_for_sub = policy_rules.clone();
    let subscriber_fn = iii.register_function((
        RegisterFunctionMessage::with_id("policy::approval_gate".into())
            .with_description("Apply approval-gate rules to before-function-call hooks.".into()),
        move |envelope: Value| {
            let iii = iii_for_sub.clone();
            let bus = bus_for_sub.clone();
            let sc = subscriber_scope.clone();
            let policy_rules = policy_rules_for_sub.clone();
            async move {
                let call = match parse_call(&envelope) {
                    Ok(call) => call,
                    Err(err) => {
                        let (reply, routing) = build_parse_error_reply(err);
                        if let Some((reply_stream, event_id)) = routing {
                            write_hook_reply(&iii, &reply_stream, &event_id, &reply).await;
                        }
                        return Ok::<_, IIIError>(reply);
                    }
                };
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);

                // Take a session-specific snapshot under the read lock,
                // then drop the guard before any .await. std::sync::RwLock
                // is not async-safe to hold across suspension points, and
                // a held guard would block every concurrent intercept.
                //
                // Per-session snapshot (finding #2): `allow + always` rules
                // pushed for OTHER sessions stay out of this call's view.
                let rules_snapshot: rules::RulesSnapshot = {
                    let guard = policy_rules
                        .read()
                        .expect("approval-gate policy rules lock poisoned");
                    guard.snapshot_for(&call.session_id)
                };

                // One decision call. Verdict::Allow → {block:false}.
                // Verdict::Deny → {block:true, denial:ApprovalRuleDenied{...}}.
                // Verdict::Ask → write Pending + reply {block:true, status:pending}.
                let mut reply = handle_intercept(
                    bus.as_ref(),
                    &sc,
                    &call,
                    &rules_snapshot,
                    now_ms,
                    timeout_ms,
                )
                .await;

                mark_approval_gate_reply(&mut reply);

                if reply.get("status").and_then(Value::as_str) == Some("pending") {
                    let event = build_approval_requested_event(
                        &call,
                        &reply,
                        now_ms.saturating_add(timeout_ms),
                    );
                    write_event(&iii, &call.session_id, &event).await;
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
        tick_timeouts,
        subscriber_fn,
        subscriber_trigger,
        tick_task,
    })
}

/// Two-method abstraction over the iii calls `publish_turn_step` makes
/// when waking an orchestrator. Production code uses [`IIIWaker`];
/// integration tests inject a fake to spy on `run::resume` calls and
/// drive the wake-failure branch deterministically.
#[async_trait::async_trait]
pub trait OrchestratorWaker: Send + Sync {
    /// Ask the orchestrator to resume the given session. `Ok` is the
    /// trigger response; `Err` is the string-formatted trigger error.
    async fn resume(&self, session_id: &str) -> Result<Value, String>;
    /// Surface a wake failure to the operator's stream so a stuck
    /// orchestrator doesn't look like "approval went through, nothing
    /// happened". Fire-and-forget; the persisted record is the source
    /// of truth.
    async fn write_wake_failed(&self, session_id: &str, error: &str);
}

/// Production [`OrchestratorWaker`]: drives the iii client. Constructed
/// per-call by the auto-tick loop and the registered `tick_timeouts`
/// function — both ultimately funnel into [`publish_turn_step`].
pub struct IIIWaker<'a> {
    pub iii: &'a III,
}

#[async_trait::async_trait]
impl<'a> OrchestratorWaker for IIIWaker<'a> {
    async fn resume(&self, session_id: &str) -> Result<Value, String> {
        self.iii
            .trigger(TriggerRequest {
                function_id: "run::resume".into(),
                payload: json!({ "session_id": session_id }),
                action: None,
                timeout_ms: Some(5_000),
            })
            .await
            .map_err(|e| e.to_string())
    }
    async fn write_wake_failed(&self, session_id: &str, error: &str) {
        write_event(
            self.iii,
            session_id,
            &json!({
                "type": "approval_wake_failed",
                "error": error,
            }),
        )
        .await;
    }
}

/// Run one watchdog cycle and wake every session that had a row
/// reclaimed. Shared by the registered function and the auto-tick loop
/// so on-demand and scheduled ticks behave identically.
///
/// Public so integration tests can drive it through a fake
/// [`OrchestratorWaker`] and assert the wake side-effects without
/// booting a real iii engine.
pub async fn run_tick(
    waker: &dyn OrchestratorWaker,
    bus: &dyn StateBus,
    state_scope: &str,
    payload: Value,
) -> Value {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let tick = handle_tick_timeouts(bus, state_scope, payload, now_ms).await;
    if let Some(sessions) = tick.get("sessions_woken").and_then(Value::as_array) {
        for session in sessions.iter().filter_map(Value::as_str) {
            publish_turn_step(waker, session).await;
        }
    }
    tick
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

fn mark_approval_gate_reply(reply: &mut Value) {
    if let Some(obj) = reply.as_object_mut() {
        obj.insert("subscriber".into(), json!("approval-gate"));
        obj.insert("approval_gate".into(), json!(true));
    }
}

fn build_parse_error_reply(err: ExtractCallError) -> (Value, Option<(String, String)>) {
    let mut reply = state_error_reply(err.phase, &err.error);
    reply["status"] = json!("denied");
    mark_approval_gate_reply(&mut reply);
    let routing = err.reply_stream.zip(err.event_id);
    (reply, routing)
}

fn build_approval_requested_event(call: &IncomingCall, reply: &Value, expires_at: u64) -> Value {
    json!({
        "type": "approval_requested",
        "function_call_id": &call.function_call_id,
        "tool_call_id": &call.function_call_id,
        "function_id": &call.function_id,
        "tool_name": &call.function_id,
        "args": &call.args,
        "expires_at": expires_at,
        "rule": reply.get("rule").cloned().unwrap_or(Value::Null),
        "reason": reply.get("reason").cloned().unwrap_or(Value::Null),
    })
}

/// Emit an `approval_resolved` stream event for one `(session_id, call_id)`
/// pair. Reads the post-write record from the state bus, derives the
/// event payload via [`build_approval_resolved_event`], then writes it
/// to the agent::events stream. Used both for the originator of a
/// resolve and for each cascade-resolved call so the operator's view
/// stays consistent across allow+always.
pub(crate) async fn emit_approval_resolved(
    iii: &III,
    bus: &dyn StateBus,
    state_scope: &str,
    session_id: &str,
    call_id: &str,
) {
    let key = pending_key(session_id, call_id);
    let Some(final_rec) = bus.get(state_scope, &key).await else {
        return;
    };
    let evt = build_approval_resolved_event(call_id, &final_rec);
    write_event(iii, session_id, &evt).await;
}

/// Build the `approval_resolved` stream event payload from a terminal
/// Record JSON. Pure helper — unit-tested below.
///
/// **Bug fix (P2):** the previous implementation read `final_rec.status`
/// (always `"done"` for terminal records — see `Record::Status` in
/// `record.rs`) and tried to match it against `"executed"|"approved"`,
/// so every resolved approval emitted `decision: "deny"`. The actual
/// outcome lives at `final_rec.outcome.kind` (one of `executed`,
/// `failed`, `denied`, `timed_out`). This helper reads the right field.
///
/// **Wire shape change:** the redundant flat `status`/`result`/`error`/
/// `denial` fields are dropped in favor of a single nested `outcome:
/// { kind, detail }` carrying full fidelity for downstream consumers.
/// UI consumers (`harness/web/src/reducer.ts`) only read
/// `function_call_id`, so dropping the flats is safe.
pub fn build_approval_resolved_event(call_id: &str, final_rec: &Value) -> Value {
    let mut evt = json!({
        "type": "approval_resolved",
        "function_call_id": call_id,
        "tool_call_id": call_id,
    });
    let outcome = final_rec.get("outcome");
    let outcome_kind = outcome.and_then(|o| o.get("kind")).and_then(Value::as_str);
    let decision = match outcome_kind {
        Some("executed" | "failed") => "allow",
        _ => "deny",
    };
    evt["decision"] = json!(decision);
    if let Some(o) = outcome {
        evt["outcome"] = o.clone();
    }
    if let Some(rule) = final_rec.get("rule") {
        evt["rule"] = rule.clone();
    }
    if let Some(reason) = final_rec.get("reason") {
        evt["reason"] = reason.clone();
    }
    evt
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

/// Wake the turn-orchestrator after an approval resolves so the Done
/// record drains into the next assistant turn via `approval::consume`.
///
/// Calls `run::resume` rather than publishing `turn::step_requested`
/// directly. The reason: a `pending_approval` prefilled result ends the
/// turn with `terminate: true`, leaving the record in `Stopped`. A bare
/// step-kick against a terminal record is a no-op in the orchestrator's
/// subscriber, so the resolved approval would never surface. `run::resume`
/// resets a terminal record to `Provisioning` and re-kicks; against a
/// still-running session it just re-kicks (idempotent).
pub(crate) async fn publish_turn_step(waker: &dyn OrchestratorWaker, session_id: &str) {
    if session_id.is_empty() {
        return;
    }
    tracing::info!(%session_id, "approval-gate: calling run::resume after approval resolve");
    match waker.resume(session_id).await {
        Ok(v) => {
            tracing::info!(%session_id, response = %v, "approval-gate: run::resume returned");
        }
        Err(e) => {
            tracing::warn!(error = %e, %session_id, "approval-gate: run::resume failed");
            // Surface the wake failure to the UI so a stuck orchestrator
            // doesn't look like "approval went through, nothing happened".
            // Without this the operator clicks allow, sees the card vanish,
            // and the conversation silently stalls.
            waker.write_wake_failed(session_id, &e).await;
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_gate_hook_replies_are_marked_for_required_subscriber_check() {
        let mut reply = json!({ "block": false });
        mark_approval_gate_reply(&mut reply);
        assert_eq!(reply["subscriber"], "approval-gate");
        assert_eq!(reply["approval_gate"], true);
    }

    #[test]
    fn malformed_hook_reply_is_marked_and_preserves_reply_routing() {
        let err = parse_call(&json!({
            "event_id": "evt-1",
            "reply_stream": "rs-1",
            "payload": { "session_id": "s1" }
        }))
        .unwrap_err();

        let (reply, routing) = build_parse_error_reply(err);

        assert_eq!(reply["block"], true);
        assert_eq!(reply["status"], "denied");
        assert_eq!(reply["subscriber"], "approval-gate");
        assert_eq!(reply["approval_gate"], true);
        assert_eq!(reply["denial"]["kind"], "state_error");
        assert_eq!(
            routing,
            Some(("rs-1".to_string(), "evt-1".to_string())),
            "subscriber must write the fail-closed state_error reply when hook routing is available",
        );
    }

    #[test]
    fn malformed_hook_reply_without_routing_still_hard_blocks_locally() {
        let err = parse_call(&json!({
            "payload": {
                "session_id": "s1",
                "function_call": { "id": "tc-1", "function_id": "shell::exec", "arguments": {} }
            }
        }))
        .unwrap_err();

        let (reply, routing) = build_parse_error_reply(err);

        assert_eq!(reply["block"], true);
        assert_eq!(reply["status"], "denied");
        assert_eq!(reply["subscriber"], "approval-gate");
        assert_eq!(reply["approval_gate"], true);
        assert_eq!(routing, None);
    }

    #[test]
    fn approval_requested_event_preserves_rule_metadata() {
        let call = IncomingCall {
            session_id: "s1".into(),
            function_call_id: "tc-1".into(),
            function_id: "shell::exec".into(),
            args: json!({ "command": "git", "args": ["push"] }),
            approval_required: Vec::new(),
            event_id: "evt-1".into(),
            reply_stream: "rs-1".into(),
        };
        let reply = json!({
            "block": true,
            "status": "pending",
            "rule": {
                "source": "global",
                "index": 2,
                "permission": "shell::exec",
                "pattern": "*",
                "action": "ask",
                "reason": "shell commands require review"
            },
            "reason": "shell commands require review"
        });

        let event = build_approval_requested_event(&call, &reply, 61_000);

        assert_eq!(event["type"], "approval_requested");
        assert_eq!(event["function_call_id"], "tc-1");
        assert_eq!(event["tool_call_id"], "tc-1");
        assert_eq!(event["function_id"], "shell::exec");
        assert_eq!(event["tool_name"], "shell::exec");
        assert_eq!(event["expires_at"], 61_000);
        assert_eq!(event["rule"]["permission"], "shell::exec");
        assert_eq!(event["reason"], "shell commands require review");
    }

    #[test]
    fn approval_resolved_event_preserves_rule_metadata() {
        let event = build_approval_resolved_event(
            "tc-1",
            &json!({
                "outcome": { "kind": "executed", "detail": { "result": { "ok": true } } },
                "rule": {
                    "source": "global",
                    "index": 2,
                    "permission": "shell::exec",
                    "pattern": "*",
                    "action": "ask",
                    "reason": "shell commands require review"
                },
                "reason": "shell commands require review"
            }),
        );
        assert_eq!(event["decision"], "allow");
        assert_eq!(event["rule"]["permission"], "shell::exec");
        assert_eq!(event["reason"], "shell commands require review");
    }
}
