//! Approval gate. Subscribes to `agent::before_function_call` and blocks calls
//! whose `function_call.function_id` appears in the run's `approval_required` list,
//! waiting for the UI to call `approval::resolve` (or for a timeout).

pub mod config;
pub mod lifecycle;
pub mod manifest;
pub mod record;
pub mod rules;
pub mod state;
pub mod wire;

pub use config::{InterceptorRule, WorkerConfig};
pub use lifecycle::{
    build_pending_record, collect_timed_out_for_sweep, is_terminal_status, maybe_flip_timed_out,
    transition_record, transition_record_with_now,
};
pub use record::{Next, Record, Status};
pub use state::{
    unverified_marker_targets, FunctionExecutor, IiiFunctionExecutor, IiiStateBus, StateBus,
};
pub use wire::{
    block_reply_for, extract_call, pending_key, Decision, Denial, IncomingCall, WireDecision,
};
use state::rule_for;
#[cfg(test)]
use state::merge_from_approval_marker_if_needed;

use std::sync::{Arc, RwLock};

use iii_sdk::{
    FunctionRef, IIIError, RegisterFunctionMessage, RegisterTriggerInput, TriggerRequest, III,
};
use serde_json::{json, Value};

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

/// What the subscriber should do with an incoming call. Decided by the
/// matching interceptor rule (authoritative) with a fallback to the run's
/// `approval_required` list when no rule exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InterceptAction {
    /// No rule, no `approval_required` listing — let the call through.
    Pass,
    /// Pause and create a pending record; no classifier consulted.
    Pause,
    /// Run the classifier first; on `ask`, pause; on `auto`, pass; on `deny`, block.
    Classify {
        classifier_fn: String,
        classifier_timeout_ms: u64,
    },
}

/// Pure decision: given a matching rule (or none) and whether the run
/// explicitly listed this function id in `approval_required`, what should
/// the subscriber do? Interceptor rules are authoritative — an operator
/// who registered a rule meant for every call to go through it, regardless
/// of per-run opt-in.
pub(crate) fn decide_intercept_action(
    rule: Option<&InterceptorRule>,
    requires_approval: bool,
) -> InterceptAction {
    match rule {
        Some(r) if r.classifier.as_ref().is_some_and(|s| !s.is_empty()) => {
            InterceptAction::Classify {
                classifier_fn: r.classifier.clone().unwrap(),
                classifier_timeout_ms: r.classifier_timeout_ms,
            }
        }
        Some(_) => InterceptAction::Pause,
        None if requires_approval => InterceptAction::Pause,
        None => InterceptAction::Pass,
    }
}

/// Outcome of the policy-rules pre-check that runs before the per-function
/// [`config::InterceptorRule`] flow. `Allow` and `Deny` short-circuit the
/// subscriber with a final reply; `FallThrough` defers to the existing
/// interceptor logic (classifier or pause).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PolicyOutcome {
    Allow,
    Deny {
        rule_permission: String,
        rule_pattern: String,
    },
    FallThrough,
}

/// Apply the layered policy rules to an incoming function id. Pure
/// function — no I/O, no clock. Extracted from [`register`]'s subscriber
/// closure so the decision branch can be unit-tested independently.
pub(crate) fn apply_policy_rules(rules: &rules::Ruleset, function_id: &str) -> PolicyOutcome {
    match rules::evaluate(function_id, "*", rules) {
        Some(rule) => match rule.action {
            rules::Action::Allow => PolicyOutcome::Allow,
            rules::Action::Deny => PolicyOutcome::Deny {
                rule_permission: rule.permission.clone(),
                rule_pattern: rule.pattern.clone(),
            },
            rules::Action::Ask => PolicyOutcome::FallThrough,
        },
        None => PolicyOutcome::FallThrough,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClassifierDecision {
    Auto,
    Deny(Denial),
    Ask,
}

/// Parse classifier JSON (`decision` tag: auto | deny | ask). On `deny`
/// the reply may carry `reason` (free-form classifier text) and optionally
/// `classifier_fn` — both get folded into a [`Denial::Policy`].
pub(crate) fn interpret_classifier_reply(
    value: &Value,
    classifier_fn: &str,
) -> Result<ClassifierDecision, ()> {
    let tag = value.get("decision").and_then(Value::as_str).ok_or(())?;
    match tag {
        "auto" => Ok(ClassifierDecision::Auto),
        "deny" => {
            let classifier_reason = value
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("denied")
                .to_string();
            Ok(ClassifierDecision::Deny(Denial::Policy {
                classifier_reason,
                classifier_fn: classifier_fn.to_string(),
            }))
        }
        "ask" => Ok(ClassifierDecision::Ask),
        _ => Err(()),
    }
}

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

/// Decide whether a call is gated; if so, write a pending record and return
/// the structured pending hook reply. If not gated, return `{block: false}`
/// and do nothing.
///
/// Stamps `session_id` onto the persisted record so the timeout sweeper can
/// emit `approval_resolved` to the right session stream without consulting
/// the storage layer's keys.
///
/// State-write failure is treated as fail-closed: the gate replies
/// `{block:true, status:"denied"}` so a transient kv outage cannot silently
/// bypass an approval check.
pub async fn handle_intercept(
    bus: &dyn StateBus,
    state_scope: &str,
    call: &IncomingCall,
    now_ms: u64,
    timeout_ms: u64,
    force_pending: bool,
) -> Value {
    if !force_pending && !call.requires_approval() {
        return json!({ "block": false });
    }

    // Defense in depth: if a record for this (session, call_id) already
    // exists, don't blow it away. Re-intercept of an already-decided call
    // would otherwise revert a terminal record back to `pending`, losing
    // the audit trail and any `delivered_in_turn_id` stamp. Surfaced by
    // the state-machine proptest in tests::state_machine_invariants.
    let key = pending_key(&call.session_id, &call.function_call_id);
    if let Some(existing) = bus.get(state_scope, &key).await {
        let status = existing
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if is_terminal_status(&status) {
            // Replay of an already-resolved call: the prior status carries
            // the meaning. No fresh Denial is synthesized — consumers that
            // need to render the historical decision read the persisted
            // record via approval::lookup_record.
            return json!({
                "block": true,
                "status": status,
                "replay": "already_resolved",
                "call_id": call.function_call_id,
                "function_id": call.function_id,
            });
        }
        if status == "pending" || status == "approved" {
            // Replay of an in-flight intercept — keep the existing row,
            // re-emit the pending reply. No state churn.
            return json!({
                "block": true,
                "status": "pending",
                "replay": "in_flight",
                "call_id": call.function_call_id,
                "function_id": call.function_id,
            });
        }
    }

    let mut record = build_pending_record(
        &call.function_call_id,
        &call.function_id,
        &call.args,
        now_ms,
        timeout_ms,
    );
    if let Some(obj) = record.as_object_mut() {
        obj.insert("session_id".into(), Value::String(call.session_id.clone()));
    }
    if let Err(err) = bus
        .set(
            state_scope,
            &pending_key(&call.session_id, &call.function_call_id),
            record,
        )
        .await
    {
        tracing::error!(
            "approval-gate: failed to write pending record for {}/{}: {err} — failing closed",
            call.session_id,
            call.function_call_id
        );
        let denial = Denial::StateError {
            phase: "intercept_write_pending".to_string(),
            error: err.to_string(),
        };
        return json!({
            "block": true,
            "denial": denial,
            "status": "denied",
            "call_id": call.function_call_id,
            "function_id": call.function_id,
        });
    }
    json!({
        "block": true,
        "status": "pending",
        "call_id": call.function_call_id,
        "function_id": call.function_id,
    })
}

/// Lookup a single approval record by session + call id (for shell bypass validation).
pub async fn handle_lookup_record(bus: &dyn StateBus, state_scope: &str, payload: Value) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let function_call_id = payload
        .get("function_call_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() || function_call_id.is_empty() {
        return Value::Null;
    }
    let key = pending_key(session_id, function_call_id);
    bus.get(state_scope, &key).await.unwrap_or(Value::Null)
}

pub async fn handle_resolve(
    bus: &dyn StateBus,
    exec: &dyn FunctionExecutor,
    state_scope: &str,
    policy_rules: &RwLock<rules::Ruleset>,
    payload: Value,
    now_ms: u64,
) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let function_call_id = payload
        .get("function_call_id")
        .or_else(|| payload.get("tool_call_id"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() || function_call_id.is_empty() {
        return json!({ "ok": false, "error": "missing_id" });
    }
    let decision: WireDecision = match payload.get("decision").cloned() {
        Some(v) => match serde_json::from_value(v) {
            Ok(d) => d,
            Err(_) => return json!({ "ok": false, "error": "bad_decision" }),
        },
        None => return json!({ "ok": false, "error": "bad_decision" }),
    };
    let key = pending_key(session_id, function_call_id);
    let Some(existing) = bus.get(state_scope, &key).await else {
        return json!({ "ok": false, "error": "not_found" });
    };

    // Lazy timeout flip (covered by Task 7 tests).
    let existing = match maybe_flip_timed_out(&existing, now_ms) {
        Some(flipped) => {
            let _ = bus.set(state_scope, &key, flipped.clone()).await;
            return json!({ "ok": false, "error": "timed_out" });
        }
        None => existing,
    };

    if existing.get("status").and_then(Value::as_str) != Some("pending") {
        return json!({ "ok": false, "error": "already_resolved" });
    }

    match decision {
        WireDecision::Deny => {
            // Caller supplies a structured Denial. Accepted shapes:
            //   { "decision": "deny", "denial": { "kind": "user_rejected", ... } }
            //   { "decision": "deny", "denial": { "kind": "user_corrected", "detail": { "feedback": "..." } } }
            // Missing `denial` is treated as a bare UserRejected (no feedback)
            // so the simplest UI flow stays one-click.
            let denial = match payload.get("denial").cloned() {
                Some(v) => match serde_json::from_value::<Denial>(v) {
                    Ok(d) => d,
                    Err(_) => return json!({ "ok": false, "error": "bad_denial" }),
                },
                None => Denial::UserRejected,
            };
            let denied = transition_record(&existing, "denied", None, None, Some(denial));
            if let Err(e) = bus.set(state_scope, &key, denied).await {
                tracing::error!("approval-gate: failed to write denied record: {e}");
                return json!({ "ok": false, "error": "state_write_failed" });
            }
            json!({ "ok": true })
        }
        WireDecision::Allow => {
            if let Err(err) = approve_and_execute(
                bus,
                exec,
                state_scope,
                &existing,
                session_id,
                function_call_id,
            )
            .await
            {
                tracing::error!(
                    "approval-gate: failed to execute approved call: {err}"
                );
                return json!({ "ok": false, "error": "state_write_failed" });
            }

            // Optional cascade: when `always: true` is set on an allow
            // reply, add a runtime Allow rule for this call's function id
            // and resolve every other pending record in the same session
            // that the new rule covers. v1 scope is function-id-only —
            // the cascade rule's `pattern` is "*" to match the v1 rules
            // surface. See [`crate::rules`].
            let cascaded = if payload
                .get("always")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                let function_id = existing
                    .get("function_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                cascade_allow_for_session(
                    bus,
                    exec,
                    state_scope,
                    policy_rules,
                    session_id,
                    function_call_id,
                    &function_id,
                )
                .await
            } else {
                0
            };

            if cascaded > 0 {
                json!({ "ok": true, "cascaded": cascaded })
            } else {
                json!({ "ok": true })
            }
        }
    }
}

/// Push an Allow rule for `function_id` into the shared policy ruleset,
/// then resolve every pending record in `session_id` (other than the one
/// just resolved by the caller) that the new rule covers. Returns the
/// number of records auto-resolved.
///
/// The function id rule is appended once; if the user clicks "always
/// allow X" twice for the same X within a session, the second push is a
/// duplicate but harmless (last-wins still picks Allow). State-write
/// failures inside the loop are logged and skipped so a single bad
/// record can't prevent the rest of the cascade.
async fn cascade_allow_for_session(
    bus: &dyn StateBus,
    exec: &dyn FunctionExecutor,
    state_scope: &str,
    policy_rules: &RwLock<rules::Ruleset>,
    session_id: &str,
    originator_call_id: &str,
    originator_function_id: &str,
) -> u64 {
    // Push the new Allow rule under the write lock. Hold the guard only
    // for the mutation, not across the .await in the sweep below.
    {
        let mut guard = policy_rules
            .write()
            .expect("approval-gate policy rules lock poisoned");
        guard.push(rules::Rule {
            permission: originator_function_id.to_string(),
            pattern: "*".to_string(),
            action: rules::Action::Allow,
        });
    }

    // Snapshot the session's pending records and re-evaluate each one
    // against the now-updated rules. Use a read-clone so we don't hold
    // the lock across .await.
    let prefix = format!("{session_id}/");
    let session_records = bus.list_prefix(state_scope, &prefix).await;
    let mut cascaded = 0u64;
    for rec in session_records {
        let rec_call_id = match rec.get("function_call_id").and_then(Value::as_str) {
            Some(s) => s.to_string(),
            None => continue,
        };
        if rec_call_id == originator_call_id {
            continue;
        }
        if rec.get("status").and_then(Value::as_str) != Some("pending") {
            continue;
        }
        let fn_id = rec
            .get("function_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let outcome = {
            let guard = policy_rules
                .read()
                .expect("approval-gate policy rules lock poisoned");
            apply_policy_rules(&guard, &fn_id)
        };
        if !matches!(outcome, PolicyOutcome::Allow) {
            continue;
        }
        if let Err(err) =
            approve_and_execute(bus, exec, state_scope, &rec, session_id, &rec_call_id).await
        {
            tracing::warn!(
                session_id,
                call_id = %rec_call_id,
                "approval-gate: cascade auto-resolve failed: {err}"
            );
            continue;
        }
        cascaded += 1;
    }
    cascaded
}

/// Drive a pending record through the approved → invoke → executed/failed
/// flow. Pure plumbing — does not consult policy rules, does not check
/// the original status (caller must have verified it's pending). Used by
/// both the user-driven [`handle_resolve`] allow path and the
/// cascade-on-`always` sweep so the state transitions stay in one place.
///
/// Returns `Err` only when a state write fails; the invocation result
/// itself (success or function-error) is captured on the record. The
/// caller decides how to surface a state-write failure (the existing
/// handlers map it to `{ok:false, error:"state_write_failed"}`).
pub(crate) async fn approve_and_execute(
    bus: &dyn StateBus,
    exec: &dyn FunctionExecutor,
    state_scope: &str,
    pending: &Value,
    session_id: &str,
    function_call_id: &str,
) -> Result<(), String> {
    let function_id = pending
        .get("function_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let args = pending.get("args").cloned().unwrap_or(json!({}));
    let key = pending_key(session_id, function_call_id);
    let approved = transition_record(pending, "approved", None, None, None);
    // Best-effort intermediate write; if it fails we still try to invoke
    // so the user-visible behavior matches the pre-extraction allow path.
    let _ = bus.set(state_scope, &key, approved.clone()).await;
    match exec
        .invoke(&function_id, args, function_call_id, session_id)
        .await
    {
        Ok(result) => {
            let executed = transition_record(&approved, "executed", Some(result), None, None);
            bus.set(state_scope, &key, executed)
                .await
                .map_err(|e| e.to_string())
        }
        Err(error) => {
            let failed = transition_record(&approved, "failed", None, Some(error), None);
            bus.set(state_scope, &key, failed)
                .await
                .map_err(|e| e.to_string())
        }
    }
}

pub async fn handle_list_pending(bus: &dyn StateBus, state_scope: &str, payload: Value) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() {
        return json!({ "pending": [] });
    }
    let prefix = format!("{session_id}/");
    let all = bus.list_prefix(state_scope, &prefix).await;
    let pending: Vec<Value> = all
        .into_iter()
        .filter(|v| v.get("status").and_then(Value::as_str) == Some("pending"))
        .collect();
    json!({ "pending": pending })
}

/// Default cap for `handle_list_undelivered` responses. A single LLM turn
/// should never be asked to ingest more than this many stitched approval
/// messages; older entries beyond the cap stay unacked and are reported via
/// the `omitted` counter so the caller can render a summary line.
pub const LIST_UNDELIVERED_DEFAULT_LIMIT: usize = 50;

/// Return terminal-status records for a session that haven't been stamped
/// with `delivered_in_turn_id`. Lazy timeout: pending records past
/// `expires_at` (as observed at `now_ms`) are flipped to `timed_out` before
/// the filter so they surface here in the same call.
///
/// Sorted oldest-first by `resolved_at` (records missing `resolved_at` sort
/// last as `u64::MAX`). Capped at `limit` (default
/// [`LIST_UNDELIVERED_DEFAULT_LIMIT`]); the response always includes an
/// `omitted` field counting entries left behind.
pub async fn handle_list_undelivered(
    bus: &dyn StateBus,
    state_scope: &str,
    payload: Value,
    now_ms: u64,
) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() {
        return json!({ "entries": [], "omitted": 0 });
    }
    let limit = payload
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(LIST_UNDELIVERED_DEFAULT_LIMIT);
    let prefix = format!("{session_id}/");
    let all = bus.list_prefix(state_scope, &prefix).await;
    let mut entries: Vec<Value> = Vec::new();
    for rec in all {
        // Defensive scope: some bus backends ignore the prefix and return
        // every record in `state_scope`. Drop anything not stamped with
        // the session_id we're listing for. Orphan records lacking a
        // session_id stamp are dropped (cannot be attributed); the
        // migration path that used to recover them no longer exists.
        match rec.get("session_id").and_then(Value::as_str) {
            Some(sid) if sid == session_id => {}
            _ => continue,
        }
        let rec = if let Some(flipped) = maybe_flip_timed_out(&rec, now_ms) {
            let call_id = flipped
                .get("function_call_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            let _ = bus
                .set(
                    state_scope,
                    &pending_key(session_id, call_id),
                    flipped.clone(),
                )
                .await;
            flipped
        } else {
            rec
        };
        let status = rec.get("status").and_then(Value::as_str).unwrap_or("");
        if !is_terminal_status(status) {
            continue;
        }
        if rec
            .get("delivered_in_turn_id")
            .is_some_and(|v| !v.is_null())
        {
            continue;
        }
        entries.push(rec);
    }
    entries.sort_by_key(|e| {
        e.get("resolved_at")
            .and_then(Value::as_u64)
            .unwrap_or(u64::MAX)
    });
    let total = entries.len();
    let omitted = total.saturating_sub(limit);
    entries.truncate(limit);
    json!({ "entries": entries, "omitted": omitted })
}

/// Stamp `delivered_in_turn_id` on terminal-status records named in
/// `call_ids` for the given session. Idempotent: records already stamped
/// (non-null `delivered_in_turn_id`) are not overwritten. Unknown call ids
/// are silently skipped.
pub async fn handle_ack_delivered(bus: &dyn StateBus, state_scope: &str, payload: Value) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let turn_id = payload.get("turn_id").and_then(Value::as_str).unwrap_or("");
    let call_ids: Vec<String> = payload
        .get("call_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if session_id.is_empty() || turn_id.is_empty() || call_ids.is_empty() {
        return json!({ "ok": true, "stamped": 0 });
    }
    let mut stamped = 0_u64;
    for cid in call_ids {
        let key = pending_key(session_id, &cid);
        let Some(rec) = bus.get(state_scope, &key).await else {
            continue;
        };
        if rec
            .get("delivered_in_turn_id")
            .is_some_and(|v| !v.is_null())
        {
            continue;
        }
        let mut next = rec;
        next.as_object_mut().unwrap().insert(
            "delivered_in_turn_id".into(),
            Value::String(turn_id.to_string()),
        );
        if bus.set(state_scope, &key, next).await.is_ok() {
            stamped += 1;
        }
    }
    json!({ "ok": true, "stamped": stamped })
}

/// Atomic list+ack: returns the same entries `handle_list_undelivered` would
/// surface (subject to the same FIFO+cap rules) and stamps each one with
/// `delivered_in_turn_id` before returning. Eliminates the list→LLM→ack
/// race window: if the caller crashes after receiving the response, the
/// entries are still considered delivered and will not resurface, which is
/// acceptable because terminal records are informational (the side-effect
/// already executed inside the gate).
///
/// Required payload: `{ session_id, turn_id, limit? }`.
pub async fn handle_consume_undelivered(
    bus: &dyn StateBus,
    state_scope: &str,
    payload: Value,
    now_ms: u64,
) -> Value {
    let turn_id = payload.get("turn_id").and_then(Value::as_str).unwrap_or("");
    if turn_id.is_empty() {
        return json!({ "ok": false, "error": "missing_turn_id", "entries": [], "omitted": 0 });
    }
    let listed = handle_list_undelivered(bus, state_scope, payload.clone(), now_ms).await;
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let entries = listed["entries"].as_array().cloned().unwrap_or_default();
    let omitted = listed["omitted"].as_u64().unwrap_or(0);
    for rec in &entries {
        let cid = rec
            .get("function_call_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        if cid.is_empty() {
            continue;
        }
        let key = pending_key(session_id, cid);
        let mut stamped = rec.clone();
        stamped.as_object_mut().unwrap().insert(
            "delivered_in_turn_id".into(),
            Value::String(turn_id.to_string()),
        );
        let _ = bus.set(state_scope, &key, stamped).await;
    }
    json!({ "ok": true, "entries": entries, "omitted": omitted })
}

/// One-shot drain: stamp every terminal-status record in `session_id` that
/// lacks `delivered_in_turn_id`. Intended for operator recovery after a
/// large backlog accumulates (e.g. when the orchestrator was offline or
/// `consume_undelivered` was unreachable). Pending records are untouched —
/// use `sweep_session` if you want to expire them first.
pub async fn handle_flush_delivered(bus: &dyn StateBus, state_scope: &str, payload: Value) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let turn_id = payload.get("turn_id").and_then(Value::as_str).unwrap_or("");
    if session_id.is_empty() || turn_id.is_empty() {
        return json!({ "ok": false, "error": "missing_session_or_turn_id", "stamped": 0 });
    }
    let prefix = format!("{session_id}/");
    let all = bus.list_prefix(state_scope, &prefix).await;
    let mut stamped = 0_u64;
    for rec in all {
        let status = rec.get("status").and_then(Value::as_str).unwrap_or("");
        if !is_terminal_status(status) {
            continue;
        }
        if rec
            .get("delivered_in_turn_id")
            .is_some_and(|v| !v.is_null())
        {
            continue;
        }
        let cid = rec
            .get("function_call_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_default();
        if cid.is_empty() {
            continue;
        }
        let mut next = rec;
        next.as_object_mut().unwrap().insert(
            "delivered_in_turn_id".into(),
            Value::String(turn_id.to_string()),
        );
        if bus
            .set(state_scope, &pending_key(session_id, &cid), next)
            .await
            .is_ok()
        {
            stamped += 1;
        }
    }
    json!({ "ok": true, "stamped": stamped })
}

/// Sweep all still-pending approvals for a session to timed_out.
///
/// The `timed_out` status is self-describing per the Denial refactor —
/// callers no longer pass (or get back) a reason string. If you need to
/// distinguish *why* a session was swept (delete vs. abort vs. timeout),
/// the calling worker already has that context and should log it there.
pub async fn handle_sweep_session(bus: &dyn StateBus, state_scope: &str, payload: Value) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() {
        return json!({ "ok": false, "error": "missing_session_id", "swept": 0 });
    }
    let prefix = format!("{session_id}/");
    let all = bus.list_prefix(state_scope, &prefix).await;
    let mut swept = 0_u64;
    for rec in all {
        if rec.get("status").and_then(Value::as_str) != Some("pending") {
            continue;
        }
        let call_id = rec
            .get("function_call_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        if call_id.is_empty() {
            continue;
        }
        let flipped = transition_record(&rec, "timed_out", None, None, None);
        if bus
            .set(state_scope, &pending_key(session_id, call_id), flipped)
            .await
            .is_ok()
        {
            swept += 1;
        }
    }
    json!({ "ok": true, "swept": swept })
}

fn uuid_like() -> String {
    // Lightweight unique-ish id without pulling uuid in: ns timestamp + counter.
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0);
    let n = C.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{t:x}-{n:x}")
}

async fn write_event(iii: &III, session_id: &str, event: &Value) {
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

/// Build the `approval_resolved` event a sweeper emits when it auto-flips an
/// expired pending record. Pure — caller pumps the result onto the stream.
fn timeout_resolved_event(function_call_id: &str) -> Value {
    // Timed-out approvals carry no Denial — the `status: "timed_out"` is
    // self-describing per the Denial refactor. Consumers (turn-orchestrator
    // stitching, UIs) render the timeout from the status alone.
    json!({
        "type": "approval_resolved",
        "function_call_id": function_call_id,
        "tool_call_id": function_call_id,
        "decision": "deny",
        "status": "timed_out",
    })
}

/// Spawn the periodic timeout sweeper. The task ticks every `interval_ms`,
/// scans the configured state scope, and for any pending record whose
/// `expires_at` is in the past: writes the flipped record back and emits an
/// `approval_resolved` (status=timed_out) frame on `agent::events/<session>`.
///
/// The previous design relied on lazy timeout flips during
/// `handle_resolve`/`handle_list_undelivered`. Operators who never opened the
/// UI for a session would leave its pending rows in `pending` forever and
/// the paused turn-orchestrator would never see a decision. Active sweeping
/// closes that hole.
fn spawn_timeout_sweeper(
    iii: III,
    bus: Arc<dyn StateBus>,
    state_scope: String,
    interval_ms: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker =
            tokio::time::interval(std::time::Duration::from_millis(interval_ms.max(50)));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Drop the immediate first tick so we don't sweep before any
        // pending row could possibly exist.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let all = bus.list_prefix(&state_scope, "").await;
            for (key, flipped, session_id, call_id) in collect_timed_out_for_sweep(&all, now_ms) {
                if let Err(err) = bus.set(&state_scope, &key, flipped).await {
                    tracing::warn!(
                        "approval-gate sweeper: failed to flip {key} → timed_out: {err}"
                    );
                    continue;
                }
                write_event(&iii, &session_id, &timeout_resolved_event(&call_id)).await;
            }
        }
    })
}

async fn write_hook_reply(iii: &III, stream_name: &str, event_id: &str, reply: &Value) {
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

pub fn register(iii: &III, cfg: &WorkerConfig) -> anyhow::Result<Refs> {
    let rules: Arc<Vec<InterceptorRule>> = Arc::new(cfg.interceptors.clone());
    // Layered policy rules consulted before the per-function interceptor
    // flow. Wrapped in RwLock so a user reply with `always: true` on
    // `approval::resolve` can push a new Allow rule at runtime (see the
    // cascade in `handle_resolve`). See [`crate::rules`].
    let policy_rules: Arc<RwLock<crate::rules::Ruleset>> =
        Arc::new(RwLock::new(cfg.rules.clone()));

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
    let ack_delivered =
        iii.register_function((
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

                // Layered policy rules run first. Allow / Deny short-circuit;
                // Ask (and no-match) falls through to the existing per-function
                // interceptor flow. Pattern is "*" in v1 — see `crate::rules`.
                // Read-lock is acquired and dropped inside a block so the
                // guard never crosses an `.await` (std::sync::RwLock is not
                // async-safe to hold across suspension points).
                let policy_outcome = {
                    let guard = policy_rules
                        .read()
                        .expect("approval-gate policy rules lock poisoned");
                    apply_policy_rules(&guard, &call.function_id)
                };
                match policy_outcome {
                    PolicyOutcome::Allow => {
                        return Ok::<_, IIIError>(json!({ "block": false }));
                    }
                    PolicyOutcome::Deny {
                        rule_permission,
                        rule_pattern,
                    } => {
                        let denial = Denial::Policy {
                            classifier_reason: format!(
                                "rule {rule_permission} {rule_pattern} denies"
                            ),
                            classifier_fn: "approval-gate::rules".to_string(),
                        };
                        return Ok::<_, IIIError>(json!({
                            "block": true,
                            "denial": denial,
                            "status": "denied",
                            "call_id": call.function_call_id,
                            "function_id": call.function_id,
                        }));
                    }
                    PolicyOutcome::FallThrough => {}
                }

                let action = decide_intercept_action(
                    rule_for(intercept_rules.as_slice(), &call.function_id),
                    call.requires_approval(),
                );
                let reply = match action {
                    InterceptAction::Pass => json!({ "block": false }),
                    InterceptAction::Pause => {
                        handle_intercept(bus.as_ref(), &sc, &call, now_ms, timeout_ms, false).await
                    }
                    InterceptAction::Classify {
                        classifier_fn,
                        classifier_timeout_ms,
                    } => match iii
                        .trigger(TriggerRequest {
                            function_id: classifier_fn.clone(),
                            payload: call.args.clone(),
                            action: None,
                            timeout_ms: Some(classifier_timeout_ms),
                        })
                        .await
                    {
                        Ok(v) => match interpret_classifier_reply(&v, &classifier_fn) {
                            Ok(ClassifierDecision::Auto) => json!({ "block": false }),
                            Ok(ClassifierDecision::Deny(denial)) => json!({
                                "block": true,
                                "denial": denial,
                                "status": "denied",
                                "call_id": call.function_call_id,
                                "function_id": call.function_id,
                            }),
                            Ok(ClassifierDecision::Ask) | Err(()) => {
                                handle_intercept(
                                    bus.as_ref(),
                                    &sc,
                                    &call,
                                    now_ms,
                                    timeout_ms,
                                    true,
                                )
                                .await
                            }
                        },
                        Err(_) => {
                            handle_intercept(bus.as_ref(), &sc, &call, now_ms, timeout_ms, true)
                                .await
                        }
                    },
                };

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Empty policy ruleset for tests that exercise [`handle_resolve`]
    /// without cascading. Each call freshly constructs the lock so unit
    /// tests stay independent — there's no shared mutable state.
    fn empty_policy_rules() -> std::sync::RwLock<crate::rules::Ruleset> {
        std::sync::RwLock::new(crate::rules::Ruleset::new())
    }

    #[test]
    fn maybe_flip_timed_out_returns_some_when_pending_and_expired() {
        let rec = build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000);
        let flipped = maybe_flip_timed_out(&rec, 70_000).expect("should flip");
        assert_eq!(flipped["status"], "timed_out");
        // Timeout carries no Denial — the status alone explains the outcome.
        assert!(flipped.get("denial").is_none());
        assert!(flipped.get("decision_reason").is_none());
    }

    #[test]
    fn maybe_flip_timed_out_returns_none_when_pending_and_not_expired() {
        let rec = build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000);
        assert!(maybe_flip_timed_out(&rec, 60_000).is_none());
        assert!(maybe_flip_timed_out(&rec, 1_500).is_none());
    }

    #[test]
    fn maybe_flip_timed_out_returns_none_when_not_pending() {
        let rec = json!({
            "function_call_id": "tc-1",
            "status": "executed",
            "expires_at": 1_000_u64,
        });
        assert!(maybe_flip_timed_out(&rec, 999_999_999).is_none());
    }

    #[test]
    fn transition_record_stamps_resolved_at_for_terminal_status() {
        let base = build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000);
        let rec = transition_record_with_now(
            &base,
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
            12_345,
        );
        assert_eq!(rec["resolved_at"].as_u64(), Some(12_345));
    }

    #[test]
    fn transition_record_preserves_existing_resolved_at_on_relift() {
        let base = build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000);
        let first = transition_record_with_now(
            &base,
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
            12_345,
        );
        let second = transition_record_with_now(
            &first,
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
            99_999,
        );
        assert_eq!(second["resolved_at"].as_u64(), Some(12_345));
    }

    #[test]
    fn transition_record_does_not_stamp_resolved_at_for_intermediate_status() {
        let base = build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000);
        let rec =
            transition_record_with_now(&base, "approved", None, None, None, 12_345);
        assert!(rec.get("resolved_at").is_none());
    }

    #[tokio::test]
    async fn handle_list_undelivered_caps_at_default_limit_and_reports_omitted() {
        let bus = InMemoryStateBus::new();
        for i in 0..75 {
            let cid = format!("c{i}");
            let mut rec = transition_record_with_now(
                &build_pending_record(&cid, "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
                1_000 + i as u64,
            );
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), Value::String("s1".into()));
            bus.set(STATE_SCOPE, &pending_key("s1", &cid), rec)
                .await
                .unwrap();
        }
        let resp =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 100_000).await;
        assert_eq!(resp["entries"].as_array().unwrap().len(), 50);
        assert_eq!(resp["omitted"].as_u64(), Some(25));
    }

    #[tokio::test]
    async fn handle_list_undelivered_honors_explicit_limit() {
        let bus = InMemoryStateBus::new();
        for i in 0..10 {
            let cid = format!("c{i}");
            let mut rec = transition_record_with_now(
                &build_pending_record(&cid, "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
                1_000 + i as u64,
            );
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), Value::String("s1".into()));
            bus.set(STATE_SCOPE, &pending_key("s1", &cid), rec)
                .await
                .unwrap();
        }
        let resp = handle_list_undelivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "limit": 3}),
            100_000,
        )
        .await;
        assert_eq!(resp["entries"].as_array().unwrap().len(), 3);
        assert_eq!(resp["omitted"].as_u64(), Some(7));
    }

    #[tokio::test]
    async fn handle_list_undelivered_returns_oldest_first_by_resolved_at() {
        let bus = InMemoryStateBus::new();
        for (i, ts) in [(0_u32, 5_000_u64), (1, 1_000), (2, 3_000)] {
            let cid = format!("c{i}");
            let mut rec = transition_record_with_now(
                &build_pending_record(&cid, "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
                ts,
            );
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), Value::String("s1".into()));
            bus.set(STATE_SCOPE, &pending_key("s1", &cid), rec)
                .await
                .unwrap();
        }
        let resp = handle_list_undelivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "limit": 10}),
            100_000,
        )
        .await;
        let entries = resp["entries"].as_array().unwrap();
        let ids: Vec<&str> = entries
            .iter()
            .map(|e| e["function_call_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["c1", "c2", "c0"]);
    }

    #[tokio::test]
    async fn handle_list_undelivered_omitted_is_zero_when_under_limit() {
        let bus = InMemoryStateBus::new();
        let mut rec = transition_record_with_now(
            &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
            1_500,
        );
        rec.as_object_mut()
            .unwrap()
            .insert("session_id".into(), Value::String("s1".into()));
        bus.set(STATE_SCOPE, &pending_key("s1", "c1"), rec)
            .await
            .unwrap();
        let resp =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 100_000).await;
        assert_eq!(resp["entries"].as_array().unwrap().len(), 1);
        assert_eq!(resp["omitted"].as_u64(), Some(0));
    }

    #[tokio::test]
    async fn handle_consume_undelivered_stamps_returned_entries() {
        let bus = InMemoryStateBus::new();
        for i in 0..3 {
            let cid = format!("c{i}");
            let mut rec = transition_record_with_now(
                &build_pending_record(&cid, "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
                1_000 + i as u64,
            );
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), Value::String("s1".into()));
            bus.set(STATE_SCOPE, &pending_key("s1", &cid), rec)
                .await
                .unwrap();
        }
        let resp = handle_consume_undelivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "turn_id": "turn-7", "limit": 10}),
            100_000,
        )
        .await;
        assert_eq!(resp["ok"], json!(true));
        assert_eq!(resp["entries"].as_array().unwrap().len(), 3);
        assert_eq!(resp["omitted"].as_u64(), Some(0));
        let next =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 100_000).await;
        assert_eq!(next["entries"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn handle_consume_undelivered_respects_limit_and_leaves_remainder() {
        let bus = InMemoryStateBus::new();
        for i in 0..5 {
            let cid = format!("c{i}");
            let mut rec = transition_record_with_now(
                &build_pending_record(&cid, "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
                1_000 + i as u64,
            );
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), Value::String("s1".into()));
            bus.set(STATE_SCOPE, &pending_key("s1", &cid), rec)
                .await
                .unwrap();
        }
        let resp = handle_consume_undelivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "turn_id": "turn-7", "limit": 2}),
            100_000,
        )
        .await;
        assert_eq!(resp["entries"].as_array().unwrap().len(), 2);
        assert_eq!(resp["omitted"].as_u64(), Some(3));
        let next =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 100_000).await;
        assert_eq!(next["entries"].as_array().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn handle_consume_undelivered_missing_turn_id_returns_error() {
        let bus = InMemoryStateBus::new();
        let resp = handle_consume_undelivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1"}),
            100_000,
        )
        .await;
        assert_eq!(resp["ok"], json!(false));
        assert_eq!(resp["error"], json!("missing_turn_id"));
    }

    #[tokio::test]
    async fn handle_flush_delivered_stamps_all_unacked_terminals() {
        let bus = InMemoryStateBus::new();
        for i in 0..5 {
            let cid = format!("c{i}");
            let mut rec = transition_record_with_now(
                &build_pending_record(&cid, "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
                1_000 + i as u64,
            );
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), Value::String("s1".into()));
            bus.set(STATE_SCOPE, &pending_key("s1", &cid), rec)
                .await
                .unwrap();
        }
        let resp = handle_flush_delivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "turn_id": "manual-flush"}),
        )
        .await;
        assert_eq!(resp["ok"], json!(true));
        assert_eq!(resp["stamped"].as_u64(), Some(5));
        let next =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 100_000).await;
        assert_eq!(next["entries"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn handle_flush_delivered_skips_pending_records() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "c1"),
            build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
        )
        .await
        .unwrap();
        let resp = handle_flush_delivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "turn_id": "manual-flush"}),
        )
        .await;
        assert_eq!(resp["stamped"].as_u64(), Some(0));
        let still = bus
            .get(STATE_SCOPE, &pending_key("s1", "c1"))
            .await
            .unwrap();
        assert_eq!(still["status"].as_str(), Some("pending"));
        assert!(still.get("delivered_in_turn_id").is_none());
    }

    #[tokio::test]
    async fn handle_flush_delivered_idempotent_on_already_stamped() {
        let bus = InMemoryStateBus::new();
        let mut rec = transition_record_with_now(
            &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
            1_500,
        );
        {
            let obj = rec.as_object_mut().unwrap();
            obj.insert(
                "delivered_in_turn_id".into(),
                Value::String("turn-prev".into()),
            );
            obj.insert("session_id".into(), Value::String("s1".into()));
        }
        bus.set(STATE_SCOPE, &pending_key("s1", "c1"), rec)
            .await
            .unwrap();
        let resp = handle_flush_delivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "turn_id": "manual-flush"}),
        )
        .await;
        assert_eq!(resp["stamped"].as_u64(), Some(0));
        let still = bus
            .get(STATE_SCOPE, &pending_key("s1", "c1"))
            .await
            .unwrap();
        assert_eq!(still["delivered_in_turn_id"].as_str(), Some("turn-prev"));
    }

    #[tokio::test]
    async fn handle_list_undelivered_returns_terminal_records_with_no_delivered_stamp() {
        let bus = InMemoryStateBus::new();
        let mut r1 = transition_record(
            &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
        );
        r1.as_object_mut()
            .unwrap()
            .insert("session_id".into(), Value::String("s1".into()));
        bus.set(STATE_SCOPE, &pending_key("s1", "c1"), r1)
            .await
            .unwrap();
        let mut r2 = transition_record(
            &build_pending_record("c2", "shell::fs::write", &json!({}), 1_000, 60_000),
            "denied",
            None,
            None,
            Some(Denial::UserCorrected {
                feedback: "nope".into(),
            }),
        );
        r2.as_object_mut()
            .unwrap()
            .insert("session_id".into(), Value::String("s1".into()));
        bus.set(STATE_SCOPE, &pending_key("s1", "c2"), r2)
            .await
            .unwrap();

        let resp =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 100_000).await;
        let entries = resp["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(resp["omitted"].as_u64(), Some(0));
    }

    #[tokio::test]
    async fn handle_list_undelivered_excludes_pending_records() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "c1"),
            build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
        )
        .await
        .unwrap();

        let resp =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 1_500).await;
        assert_eq!(resp["entries"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn handle_list_undelivered_empty_session_returns_empty() {
        let bus = InMemoryStateBus::new();
        let resp =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 1_500).await;
        assert_eq!(resp["entries"], json!([]));
    }

    #[tokio::test]
    async fn handle_list_undelivered_excludes_records_stamped_with_delivered_turn_id() {
        let bus = InMemoryStateBus::new();
        let mut rec = transition_record(
            &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
        );
        {
            let obj = rec.as_object_mut().unwrap();
            obj.insert(
                "delivered_in_turn_id".into(),
                Value::String("turn-prev".into()),
            );
            obj.insert("session_id".into(), Value::String("s1".into()));
        }
        bus.set(STATE_SCOPE, &pending_key("s1", "c1"), rec)
            .await
            .unwrap();

        let mut r2 = transition_record(
            &build_pending_record("c2", "shell::fs::write", &json!({}), 1_000, 60_000),
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
        );
        r2.as_object_mut()
            .unwrap()
            .insert("session_id".into(), Value::String("s1".into()));
        bus.set(STATE_SCOPE, &pending_key("s1", "c2"), r2)
            .await
            .unwrap();

        let resp =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 100_000).await;
        let entries = resp["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["function_call_id"], "c2");
    }

    #[tokio::test]
    async fn handle_list_undelivered_returns_empty_when_session_id_missing() {
        let bus = InMemoryStateBus::new();
        let resp = handle_list_undelivered(&bus, STATE_SCOPE, json!({}), 1_500).await;
        assert_eq!(resp["entries"], json!([]));
    }

    #[tokio::test]
    async fn handle_ack_delivered_stamps_records_with_turn_id() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "c1"),
            transition_record(
                &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
            ),
        )
        .await
        .unwrap();

        let resp = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({
                "session_id": "s1",
                "call_ids": ["c1"],
                "turn_id": "turn-1",
            }),
        )
        .await;
        assert_eq!(resp["ok"], json!(true));
        assert_eq!(resp["stamped"], json!(1));

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "c1"))
            .await
            .unwrap();
        assert_eq!(rec["delivered_in_turn_id"], "turn-1");
    }

    #[tokio::test]
    async fn handle_ack_delivered_is_idempotent_keeps_first_turn_id() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "c1"),
            transition_record(
                &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
            ),
        )
        .await
        .unwrap();

        let _ = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({
                "session_id": "s1", "call_ids": ["c1"], "turn_id": "turn-first",
            }),
        )
        .await;
        let resp = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({
                "session_id": "s1", "call_ids": ["c1"], "turn_id": "turn-second",
            }),
        )
        .await;
        assert_eq!(resp["stamped"], json!(0), "second ack must not re-stamp");

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "c1"))
            .await
            .unwrap();
        assert_eq!(rec["delivered_in_turn_id"], "turn-first");
    }

    #[tokio::test]
    async fn handle_ack_delivered_skips_unknown_call_ids_silently() {
        let bus = InMemoryStateBus::new();
        let resp = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({
                "session_id": "s1", "call_ids": ["ghost"], "turn_id": "turn-1",
            }),
        )
        .await;
        assert_eq!(resp["ok"], json!(true));
        assert_eq!(resp["stamped"], json!(0));
    }

    #[tokio::test]
    async fn handle_resolve_on_expired_pending_flips_to_timed_out_and_ignores_decision() {
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000),
        )
        .await
        .unwrap();

        let resp = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &empty_policy_rules(),
            json!({"session_id":"s1","function_call_id":"tc-1","decision":"allow"}),
            70_000,
        )
        .await;
        assert_eq!(resp["ok"], json!(false));
        assert_eq!(resp["error"], "timed_out");

        assert!(exec.calls.lock().unwrap().is_empty());

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .unwrap();
        assert_eq!(rec["status"], "timed_out");
    }

    #[test]
    fn fn_constants_match_spec_strings() {
        assert_eq!(FN_RESOLVE, "approval::resolve");
        assert_eq!(FN_LIST_PENDING, "approval::list_pending");
        assert_eq!(FN_LIST_UNDELIVERED, "approval::list_undelivered");
        assert_eq!(FN_ACK_DELIVERED, "approval::ack_delivered");
        assert_eq!(FN_LOOKUP_RECORD, "approval::lookup_record");
    }

    #[test]
    fn interpret_classifier_reply_reads_decision_tags() {
        assert!(matches!(
            interpret_classifier_reply(&json!({"decision": "auto"}), "shell::classify_argv"),
            Ok(ClassifierDecision::Auto)
        ));
        match interpret_classifier_reply(
            &json!({"decision":"deny","reason":"nope"}),
            "shell::classify_argv",
        ) {
            Ok(ClassifierDecision::Deny(Denial::Policy {
                classifier_reason,
                classifier_fn,
            })) => {
                assert_eq!(classifier_reason, "nope");
                assert_eq!(classifier_fn, "shell::classify_argv");
            }
            o => panic!("expected Policy denial {:?}", o),
        }
        assert!(matches!(
            interpret_classifier_reply(
                &json!({"decision":"ask","summary":"x"}),
                "shell::classify_argv"
            ),
            Ok(ClassifierDecision::Ask)
        ));
        assert!(interpret_classifier_reply(&json!({}), "shell::classify_argv").is_err());
    }

    #[test]
    fn merge_from_approval_inserts_marker_when_inject_true() {
        let m = merge_from_approval_marker_if_needed(
            true,
            json!({"command": "git"}),
            "call-1",
            "sess-1",
        );
        let inner = m.get("__from_approval").unwrap();
        assert_eq!(inner["call_id"], "call-1");
        assert_eq!(inner["session_id"], "sess-1");
        assert_eq!(m["command"], "git");
    }

    #[test]
    fn merge_from_approval_noop_when_inject_false() {
        let j = json!({"a": 1});
        let out = merge_from_approval_marker_if_needed(false, j.clone(), "c", "s");
        assert_eq!(out, j);
    }

    #[test]
    fn rule_for_returns_matching_rule() {
        let rules = vec![
            InterceptorRule {
                function_id: "shell::exec".into(),
                classifier: Some("shell::classify_argv".into()),
                classifier_timeout_ms: 2000,
                inject_approval_marker: true,
                marker_target_verified: true,
            },
            InterceptorRule {
                function_id: "other::fn".into(),
                classifier: None,
                classifier_timeout_ms: 2000,
                inject_approval_marker: false,
                marker_target_verified: false,
            },
        ];
        let r = rule_for(&rules, "shell::exec").expect("match");
        assert_eq!(r.classifier.as_deref(), Some("shell::classify_argv"));
        assert!(r.inject_approval_marker);
    }

    #[test]
    fn rule_for_returns_none_when_absent() {
        let rules = vec![InterceptorRule {
            function_id: "x::y".into(),
            classifier: None,
            classifier_timeout_ms: 2000,
            inject_approval_marker: false,
            marker_target_verified: false,
        }];
        assert!(rule_for(&rules, "missing::id").is_none());
    }

    /// An operator-registered rule is authoritative: every call to that
    /// function id runs through the classifier, even when the run's
    /// `approval_required` list is empty. This is the inverted contract
    /// vs. the original "approval_required ANDs the rule" gate.
    #[test]
    fn decide_intercept_action_classifies_when_rule_has_classifier_regardless_of_approval_required() {
        let rule = InterceptorRule {
            function_id: "shell::exec".into(),
            classifier: Some("shell::classify_argv".into()),
            classifier_timeout_ms: 2000,
            inject_approval_marker: true,
            marker_target_verified: true,
        };
        let action = decide_intercept_action(Some(&rule), false);
        assert_eq!(
            action,
            InterceptAction::Classify {
                classifier_fn: "shell::classify_argv".into(),
                classifier_timeout_ms: 2000,
            }
        );
        assert_eq!(action, decide_intercept_action(Some(&rule), true));
    }

    #[test]
    fn decide_intercept_action_pauses_when_rule_has_no_classifier_regardless_of_approval_required() {
        let rule = InterceptorRule {
            function_id: "shell::fs::write".into(),
            classifier: None,
            classifier_timeout_ms: 2000,
            inject_approval_marker: false,
            marker_target_verified: false,
        };
        assert_eq!(
            decide_intercept_action(Some(&rule), false),
            InterceptAction::Pause
        );
        assert_eq!(
            decide_intercept_action(Some(&rule), true),
            InterceptAction::Pause
        );
    }

    #[test]
    fn decide_intercept_action_pauses_when_no_rule_but_run_listed_approval_required() {
        assert_eq!(decide_intercept_action(None, true), InterceptAction::Pause);
    }

    #[test]
    fn decide_intercept_action_passes_when_no_rule_and_not_approval_required() {
        assert_eq!(decide_intercept_action(None, false), InterceptAction::Pass);
    }

    #[test]
    fn apply_policy_rules_empty_ruleset_falls_through() {
        let rs: rules::Ruleset = vec![];
        assert_eq!(
            apply_policy_rules(&rs, "shell::exec"),
            PolicyOutcome::FallThrough
        );
    }

    #[test]
    fn apply_policy_rules_allow_short_circuits() {
        let rs: rules::Ruleset = vec![rules::Rule {
            permission: "shell::exec".into(),
            pattern: "*".into(),
            action: rules::Action::Allow,
        }];
        assert_eq!(
            apply_policy_rules(&rs, "shell::exec"),
            PolicyOutcome::Allow
        );
    }

    #[test]
    fn apply_policy_rules_deny_carries_matched_rule_identity() {
        let rs: rules::Ruleset = vec![rules::Rule {
            permission: "shell::*".into(),
            pattern: "*".into(),
            action: rules::Action::Deny,
        }];
        assert_eq!(
            apply_policy_rules(&rs, "shell::fs::write"),
            PolicyOutcome::Deny {
                rule_permission: "shell::*".into(),
                rule_pattern: "*".into(),
            }
        );
    }

    #[test]
    fn apply_policy_rules_ask_falls_through_to_interceptor_flow() {
        // Ask means "no decision from this layer — let the next handle it".
        let rs: rules::Ruleset = vec![rules::Rule {
            permission: "shell::exec".into(),
            pattern: "*".into(),
            action: rules::Action::Ask,
        }];
        assert_eq!(
            apply_policy_rules(&rs, "shell::exec"),
            PolicyOutcome::FallThrough
        );
    }

    #[test]
    fn apply_policy_rules_last_matching_wins() {
        // Later-listed more-specific rule overrides earlier permissive default.
        let rs: rules::Ruleset = vec![
            rules::Rule {
                permission: "*".into(),
                pattern: "*".into(),
                action: rules::Action::Allow,
            },
            rules::Rule {
                permission: "shell::exec".into(),
                pattern: "*".into(),
                action: rules::Action::Deny,
            },
        ];
        assert!(matches!(
            apply_policy_rules(&rs, "shell::exec"),
            PolicyOutcome::Deny { .. }
        ));
        assert_eq!(
            apply_policy_rules(&rs, "approval::resolve"),
            PolicyOutcome::Allow
        );
    }

    #[test]
    fn decide_intercept_action_classifier_empty_string_treated_as_no_classifier() {
        let rule = InterceptorRule {
            function_id: "shell::exec".into(),
            classifier: Some(String::new()),
            classifier_timeout_ms: 2000,
            inject_approval_marker: false,
            marker_target_verified: false,
        };
        assert_eq!(
            decide_intercept_action(Some(&rule), false),
            InterceptAction::Pause
        );
    }

    #[test]
    fn is_terminal_status_returns_true_for_terminal_states() {
        assert!(is_terminal_status("executed"));
        assert!(is_terminal_status("failed"));
        assert!(is_terminal_status("denied"));
        assert!(is_terminal_status("timed_out"));
    }

    #[test]
    fn is_terminal_status_returns_false_for_in_progress_states() {
        assert!(!is_terminal_status("pending"));
        assert!(!is_terminal_status("approved"));
        assert!(!is_terminal_status("anything_else"));
        assert!(!is_terminal_status(""));
    }

    #[test]
    fn pending_key_includes_session_and_tool_call_id() {
        assert_eq!(pending_key("s1", "tc-1"), "s1/tc-1");
    }

    #[test]
    fn extract_call_reads_session_id_and_function_call_from_envelope() {
        let envelope = json!({
            "event_id": "evt-1",
            "reply_stream": "rs-1",
            "payload": {
                "function_call": { "id": "tc-1", "function_id": "write", "arguments": {"path": "/tmp/x"} },
                "approval_required": ["write"],
                "session_id": "s1",
            }
        });
        let call = extract_call(&envelope).expect("decoded");
        assert_eq!(call.session_id, "s1");
        assert_eq!(call.function_call_id, "tc-1");
        assert_eq!(call.function_id, "write");
        assert_eq!(call.event_id, "evt-1");
        assert_eq!(call.reply_stream, "rs-1");
        assert!(call.approval_required.iter().any(|s| s == "write"));
    }

    #[test]
    fn extract_call_accepts_legacy_tool_call_envelope_with_name() {
        let envelope = json!({
            "event_id": "evt-1",
            "reply_stream": "rs-1",
            "payload": {
                "tool_call": { "id": "tc-1", "name": "write", "arguments": {} },
                "approval_required": ["write"],
                "session_id": "s1",
            }
        });
        let call = extract_call(&envelope).expect("decoded");
        assert_eq!(call.function_call_id, "tc-1");
        assert_eq!(call.function_id, "write");
    }

    #[test]
    fn requires_approval_only_for_listed_functions() {
        let call = IncomingCall {
            session_id: "s1".into(),
            function_call_id: "tc-1".into(),
            function_id: "ls".into(),
            args: json!({}),
            approval_required: vec!["write".into()],
            event_id: "e".into(),
            reply_stream: "r".into(),
        };
        assert!(!call.requires_approval());

        let call2 = IncomingCall {
            function_id: "write".into(),
            ..call
        };
        assert!(call2.requires_approval());
    }

    #[test]
    fn build_pending_record_sets_status_and_expiry() {
        let now = 1_000_000;
        let rec = build_pending_record("tc-1", "write", &json!({"x": 1}), now, 60_000);
        assert_eq!(rec["status"], "pending");
        assert_eq!(rec["function_call_id"], "tc-1");
        assert_eq!(rec["expires_at"], 1_060_000);
    }

    #[test]
    fn block_reply_for_decision_allow_does_not_block() {
        let reply = block_reply_for(&Decision::Allow);
        assert_eq!(reply["block"], false);
    }

    #[test]
    fn block_reply_for_deny_emits_structured_denial() {
        let reply = block_reply_for(&Decision::Deny(Denial::UserRejected));
        assert_eq!(reply["block"], true);
        assert_eq!(reply["denial"]["kind"], "user_rejected");
        assert!(reply.as_object().unwrap().get("reason").is_none());
    }

    #[test]
    fn block_reply_for_policy_deny_carries_classifier_detail() {
        let reply = block_reply_for(&Decision::Deny(Denial::Policy {
            classifier_reason: "command matches denylist".into(),
            classifier_fn: "shell::classify_argv".into(),
        }));
        assert_eq!(reply["block"], true);
        assert_eq!(reply["denial"]["kind"], "policy");
        assert_eq!(
            reply["denial"]["detail"]["classifier_reason"],
            "command matches denylist"
        );
        assert_eq!(
            reply["denial"]["detail"]["classifier_fn"],
            "shell::classify_argv"
        );
    }

    #[test]
    fn block_reply_for_user_corrected_carries_feedback() {
        let reply = block_reply_for(&Decision::Deny(Denial::UserCorrected {
            feedback: "use git diff instead".into(),
        }));
        assert_eq!(reply["denial"]["kind"], "user_corrected");
        assert_eq!(
            reply["denial"]["detail"]["feedback"],
            "use git diff instead"
        );
    }

    #[test]
    fn extract_call_returns_none_when_function_call_absent() {
        let envelope = json!({
            "event_id": "evt-1",
            "reply_stream": "rs-1",
            "payload": { "session_id": "s1", "approval_required": ["write"] }
        });
        assert!(extract_call(&envelope).is_none());
    }

    #[test]
    fn extract_call_returns_none_when_session_id_absent() {
        let envelope = json!({
            "event_id": "evt-1",
            "reply_stream": "rs-1",
            "payload": {
                "tool_call": { "id": "tc-1", "name": "write", "arguments": {} }
            }
        });
        assert!(extract_call(&envelope).is_none());
    }

    #[test]
    fn block_reply_for_allow_omits_denial_and_reason() {
        let reply = block_reply_for(&Decision::Allow);
        assert_eq!(reply["block"], false);
        assert!(
            reply.get("reason").is_none(),
            "Allow must not include reason: {reply}"
        );
        assert!(
            reply.get("denial").is_none(),
            "Allow must not include denial: {reply}"
        );
    }

    use std::sync::Mutex;

    fn sample_call() -> IncomingCall {
        IncomingCall {
            session_id: "s1".into(),
            function_call_id: "tc-1".into(),
            function_id: "shell::fs::write".into(),
            args: json!({"path": "/tmp/a"}),
            approval_required: vec!["shell::fs::write".into()],
            event_id: "evt-1".into(),
            reply_stream: "rs-1".into(),
        }
    }

    #[tokio::test]
    async fn handle_intercept_returns_pending_envelope_when_call_is_gated() {
        let bus = InMemoryStateBus::new();
        let call = sample_call();
        let reply = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, false).await;
        assert_eq!(reply["block"], json!(true));
        assert_eq!(reply["status"], json!("pending"));
        assert_eq!(reply["call_id"], json!("tc-1"));
        assert_eq!(reply["function_id"], json!("shell::fs::write"));
        // Pending status is self-describing — no `reason` or `denial` field
        // is emitted while the call is in-flight.
        assert!(reply.get("reason").is_none());
        assert!(reply.get("denial").is_none());
    }

    #[tokio::test]
    async fn handle_intercept_writes_pending_record_to_state() {
        let bus = InMemoryStateBus::new();
        let call = sample_call();
        let _ = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, false).await;
        let key = pending_key(&call.session_id, &call.function_call_id);
        let rec = bus
            .get(STATE_SCOPE, &key)
            .await
            .expect("pending record written");
        assert_eq!(rec["status"], "pending");
        assert_eq!(rec["function_call_id"], "tc-1");
        assert_eq!(rec["expires_at"], 61_000);
    }

    #[tokio::test]
    async fn handle_intercept_passes_through_when_call_is_not_gated() {
        let bus = InMemoryStateBus::new();
        let mut call = sample_call();
        call.approval_required = vec!["other".into()];
        let reply = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, false).await;
        assert_eq!(reply["block"], json!(false));
        let key = pending_key(&call.session_id, &call.function_call_id);
        assert!(
            bus.get(STATE_SCOPE, &key).await.is_none(),
            "no record written"
        );
    }

    #[tokio::test]
    async fn handle_intercept_force_pending_writes_when_not_on_required_list() {
        let bus = InMemoryStateBus::new();
        let mut call = sample_call();
        call.approval_required = vec!["other".into()];
        let reply = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, true).await;
        assert_eq!(reply["block"], json!(true));
        assert_eq!(reply["status"], json!("pending"));
        let key = pending_key(&call.session_id, &call.function_call_id);
        assert!(bus.get(STATE_SCOPE, &key).await.is_some());
    }

    #[tokio::test]
    async fn handle_lookup_record_returns_null_when_missing() {
        let bus = InMemoryStateBus::new();
        let v = handle_lookup_record(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "function_call_id": "c1"}),
        )
        .await;
        assert!(v.is_null());
    }

    #[tokio::test]
    async fn handle_lookup_record_returns_record_when_present() {
        let bus = InMemoryStateBus::new();
        let call = sample_call();
        let _ = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, false).await;
        let v = handle_lookup_record(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "function_call_id": "tc-1"}),
        )
        .await;
        assert_eq!(v["status"], json!("pending"));
        assert_eq!(v["function_id"], json!("shell::fs::write"));
    }

    #[derive(Default)]
    struct FakeExecutor {
        calls: Mutex<Vec<(String, Value, String, String)>>,
        response: Mutex<Option<Result<Value, String>>>,
    }

    #[async_trait::async_trait]
    impl FunctionExecutor for FakeExecutor {
        async fn invoke(
            &self,
            function_id: &str,
            args: Value,
            function_call_id: &str,
            session_id: &str,
        ) -> Result<Value, String> {
            self.calls.lock().unwrap().push((
                function_id.to_string(),
                args,
                function_call_id.to_string(),
                session_id.to_string(),
            ));
            self.response
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| Ok(json!({"ok": true})))
        }
    }

    #[tokio::test]
    async fn handle_resolve_allow_invokes_function_and_records_executed() {
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record(
                "tc-1",
                "shell::fs::write",
                &json!({"path":"/a"}),
                1_000,
                60_000,
            ),
        )
        .await
        .unwrap();

        let resp = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &empty_policy_rules(),
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "allow",
            }),
            1_500,
        )
        .await;
        assert_eq!(resp["ok"], json!(true));

        let calls = exec.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "shell::fs::write");
        assert_eq!(calls[0].1, json!({"path":"/a"}));
        assert_eq!(calls[0].2, "tc-1");
        assert_eq!(calls[0].3, "s1");

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .unwrap();
        assert_eq!(rec["status"], "executed");
        assert_eq!(rec["result"], json!({"ok": true}));
    }

    #[tokio::test]
    async fn allow_without_always_does_not_cascade() {
        // Two pending shell::exec calls in the same session. Resolving
        // the first with allow (always=false) must NOT touch the second.
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        for cid in ["tc-1", "tc-2"] {
            let mut rec = build_pending_record(cid, "shell::exec", &json!({}), 1_000, 60_000);
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), json!("s1"));
            bus.set(STATE_SCOPE, &pending_key("s1", cid), rec)
                .await
                .unwrap();
        }
        let rules = empty_policy_rules();
        let resp = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &rules,
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "allow",
            }),
            1_500,
        )
        .await;
        assert_eq!(resp["ok"], true);
        assert!(
            resp.get("cascaded").is_none(),
            "cascaded field must be omitted when always was not set: {resp}"
        );
        let other = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-2"))
            .await
            .unwrap();
        assert_eq!(other["status"], "pending");
        assert_eq!(rules.read().unwrap().len(), 0, "rule must not be pushed");
    }

    #[tokio::test]
    async fn allow_with_always_pushes_rule_and_cascades_same_session_pending() {
        // Three pending calls in session s1: two shell::exec, one
        // shell::fs::write. Resolving the first shell::exec with
        // always=true must:
        //   1. Push an Allow rule for shell::exec
        //   2. Auto-resolve the other shell::exec pending in this session
        //   3. Leave the shell::fs::write pending untouched
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        for (cid, fn_id) in [
            ("tc-1", "shell::exec"),
            ("tc-2", "shell::exec"),
            ("tc-3", "shell::fs::write"),
        ] {
            let mut rec = build_pending_record(cid, fn_id, &json!({}), 1_000, 60_000);
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), json!("s1"));
            bus.set(STATE_SCOPE, &pending_key("s1", cid), rec)
                .await
                .unwrap();
        }
        let rules = empty_policy_rules();

        let resp = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &rules,
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "allow",
                "always": true,
            }),
            1_500,
        )
        .await;
        assert_eq!(resp["ok"], true);
        assert_eq!(
            resp["cascaded"], json!(1),
            "tc-2 should cascade; tc-1 originator excluded; tc-3 not matched"
        );

        // The Allow rule for shell::exec is now in the shared ruleset.
        let pushed = rules.read().unwrap();
        assert_eq!(pushed.len(), 1);
        assert_eq!(pushed[0].permission, "shell::exec");
        assert_eq!(pushed[0].action, rules::Action::Allow);
        drop(pushed);

        // Originator and cascaded record both transitioned to executed.
        let r1 = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .unwrap();
        let r2 = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-2"))
            .await
            .unwrap();
        let r3 = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-3"))
            .await
            .unwrap();
        assert_eq!(r1["status"], "executed");
        assert_eq!(r2["status"], "executed");
        assert_eq!(
            r3["status"], "pending",
            "non-matching function_id must stay pending: {r3}"
        );

        // Executor was invoked twice: originator + cascaded.
        assert_eq!(exec.calls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn cascade_does_not_cross_session_boundary() {
        // tc-1 in session s1, tc-2 in session s2 — both shell::exec.
        // Resolving s1/tc-1 with always must not touch s2/tc-2.
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        for (session, cid) in [("s1", "tc-1"), ("s2", "tc-2")] {
            let mut rec = build_pending_record(cid, "shell::exec", &json!({}), 1_000, 60_000);
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), json!(session));
            bus.set(STATE_SCOPE, &pending_key(session, cid), rec)
                .await
                .unwrap();
        }
        let rules = empty_policy_rules();

        let resp = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &rules,
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "allow",
                "always": true,
            }),
            1_500,
        )
        .await;
        assert_eq!(resp["ok"], true);
        assert!(
            resp.get("cascaded").is_none() || resp["cascaded"] == json!(0),
            "no record in s1 to cascade onto; tc-2 in s2 must NOT be touched: {resp}"
        );

        let other_session = bus
            .get(STATE_SCOPE, &pending_key("s2", "tc-2"))
            .await
            .unwrap();
        assert_eq!(other_session["status"], "pending");
        assert_eq!(
            exec.calls.lock().unwrap().len(),
            1,
            "only the originator should have been invoked"
        );
    }

    #[tokio::test]
    async fn cascade_skips_originator_record() {
        // Single pending record. always=true must not double-resolve it.
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        let mut rec = build_pending_record("tc-1", "shell::exec", &json!({}), 1_000, 60_000);
        rec.as_object_mut()
            .unwrap()
            .insert("session_id".into(), json!("s1"));
        bus.set(STATE_SCOPE, &pending_key("s1", "tc-1"), rec)
            .await
            .unwrap();
        let rules = empty_policy_rules();

        let resp = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &rules,
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "allow",
                "always": true,
            }),
            1_500,
        )
        .await;
        assert_eq!(resp["ok"], true);
        // Originator counts under the existing allow path, not the cascade.
        assert!(resp.get("cascaded").is_none() || resp["cascaded"] == json!(0));
        assert_eq!(exec.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn cascade_skips_already_resolved_records_in_session() {
        // Two records in s1: tc-1 pending, tc-2 already terminal. The
        // cascade must skip tc-2.
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        let mut r1 = build_pending_record("tc-1", "shell::exec", &json!({}), 1_000, 60_000);
        r1.as_object_mut()
            .unwrap()
            .insert("session_id".into(), json!("s1"));
        bus.set(STATE_SCOPE, &pending_key("s1", "tc-1"), r1)
            .await
            .unwrap();
        let mut r2 = build_pending_record("tc-2", "shell::exec", &json!({}), 1_000, 60_000);
        r2.as_object_mut()
            .unwrap()
            .insert("session_id".into(), json!("s1"));
        let r2_done = transition_record(&r2, "executed", Some(json!({"ok": true})), None, None);
        bus.set(STATE_SCOPE, &pending_key("s1", "tc-2"), r2_done)
            .await
            .unwrap();

        let rules = empty_policy_rules();
        let resp = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &rules,
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "allow",
                "always": true,
            }),
            1_500,
        )
        .await;
        assert_eq!(resp["ok"], true);
        // tc-2 is terminal — not pending — so cascade skips it.
        assert!(resp.get("cascaded").is_none() || resp["cascaded"] == json!(0));
    }

    #[tokio::test]
    async fn handle_resolve_deny_does_not_invoke_function() {
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000),
        )
        .await
        .unwrap();

        let resp = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &empty_policy_rules(),
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "deny",
                "denial": {
                    "kind": "user_corrected",
                    "detail": { "feedback": "not authorized" }
                },
            }),
            1_500,
        )
        .await;
        assert_eq!(resp["ok"], json!(true));

        assert!(exec.calls.lock().unwrap().is_empty());

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .unwrap();
        assert_eq!(rec["status"], "denied");
        assert_eq!(rec["denial"]["kind"], "user_corrected");
        assert_eq!(rec["denial"]["detail"]["feedback"], "not authorized");
    }

    #[tokio::test]
    async fn handle_resolve_allow_records_failed_when_function_errors() {
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        *exec.response.lock().unwrap() = Some(Err("EACCES".into()));
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000),
        )
        .await
        .unwrap();

        let resp = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &empty_policy_rules(),
            json!({"session_id":"s1","function_call_id":"tc-1","decision":"allow"}),
            1_500,
        )
        .await;
        assert_eq!(resp["ok"], json!(true));

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .unwrap();
        assert_eq!(rec["status"], "failed");
        assert_eq!(rec["error"], "EACCES");
    }

    #[tokio::test]
    async fn fake_executor_records_calls() {
        let exec = FakeExecutor::default();
        let out = exec
            .invoke("shell::fs::write", json!({"x": 1}), "cid", "sid")
            .await
            .unwrap();
        assert_eq!(out, json!({"ok": true}));
        let calls = exec.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "shell::fs::write");
        assert_eq!(calls[0].2, "cid");
        assert_eq!(calls[0].3, "sid");
    }

    struct InMemoryStateBus {
        store: Mutex<std::collections::HashMap<String, Value>>,
    }

    impl InMemoryStateBus {
        fn new() -> Self {
            Self {
                store: Mutex::new(std::collections::HashMap::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl StateBus for InMemoryStateBus {
        async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), iii_sdk::IIIError> {
            self.store
                .lock()
                .unwrap()
                .insert(format!("{scope}/{key}"), value);
            Ok(())
        }
        async fn get(&self, scope: &str, key: &str) -> Option<Value> {
            self.store
                .lock()
                .unwrap()
                .get(&format!("{scope}/{key}"))
                .cloned()
        }
        async fn list_prefix(&self, scope: &str, prefix: &str) -> Vec<Value> {
            let map = self.store.lock().unwrap();
            map.iter()
                .filter(|(k, _)| k.starts_with(&format!("{scope}/{prefix}")))
                .map(|(_, v)| v.clone())
                .collect()
        }
    }

    #[tokio::test]
    async fn resolve_flips_status_when_pending() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record("tc-1", "write", &json!({}), 0, 60_000),
        )
        .await
        .unwrap();

        let exec = FakeExecutor::default();
        let out = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &empty_policy_rules(),
            json!({
                "function_call_id": "tc-1",
                "session_id": "s1",
                "decision": "allow",
            }),
            1_500,
        )
        .await;

        assert_eq!(out["ok"], true);
        let stored = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .unwrap();
        assert_eq!(stored["status"], "executed");
    }

    #[tokio::test]
    async fn resolve_accepts_legacy_tool_call_id_field() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record("tc-1", "write", &json!({}), 0, 60_000),
        )
        .await
        .unwrap();

        let exec = FakeExecutor::default();
        let out = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &empty_policy_rules(),
            json!({
                "tool_call_id": "tc-1",
                "session_id": "s1",
                "decision": "allow",
            }),
            1_500,
        )
        .await;

        assert_eq!(out["ok"], true);
    }

    #[tokio::test]
    async fn resolve_rejects_already_resolved_entry() {
        let bus = InMemoryStateBus::new();
        let mut rec = build_pending_record("tc-1", "write", &json!({}), 0, 60_000);
        rec["status"] = json!("allow");
        bus.set(STATE_SCOPE, &pending_key("s1", "tc-1"), rec)
            .await
            .unwrap();

        let exec = FakeExecutor::default();
        let out = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &empty_policy_rules(),
            json!({"function_call_id": "tc-1", "session_id": "s1", "decision": "deny"}),
            1_500,
        )
        .await;
        assert_eq!(out["ok"], false);
        assert_eq!(out["error"], "already_resolved");
    }

    #[tokio::test]
    async fn list_pending_returns_only_pending_for_session() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record("tc-1", "write", &json!({}), 0, 60_000),
        )
        .await
        .unwrap();
        let mut resolved = build_pending_record("tc-2", "write", &json!({}), 0, 60_000);
        resolved["status"] = json!("allow");
        bus.set(STATE_SCOPE, &pending_key("s1", "tc-2"), resolved)
            .await
            .unwrap();
        bus.set(
            STATE_SCOPE,
            &pending_key("other", "tc-3"),
            build_pending_record("tc-3", "write", &json!({}), 0, 60_000),
        )
        .await
        .unwrap();

        let out = handle_list_pending(&bus, STATE_SCOPE, json!({ "session_id": "s1" })).await;
        let items = out["pending"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["function_call_id"], "tc-1");
    }

    #[tokio::test]
    async fn resolve_deny_without_denial_defaults_to_user_rejected() {
        let bus = InMemoryStateBus::new();
        let _ = bus
            .set(
                STATE_SCOPE,
                &pending_key("s1", "tc-1"),
                build_pending_record("tc-1", "write", &json!({}), 0, 60_000),
            )
            .await;

        let exec = FakeExecutor::default();
        let out = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &empty_policy_rules(),
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "deny",
            }),
            1_500,
        )
        .await;
        assert_eq!(out["ok"], true);

        let stored = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .unwrap();
        assert_eq!(stored["status"], "denied");
        assert_eq!(stored["denial"]["kind"], "user_rejected");
    }

    #[tokio::test]
    async fn resolve_deny_rejects_malformed_denial() {
        let bus = InMemoryStateBus::new();
        let _ = bus
            .set(
                STATE_SCOPE,
                &pending_key("s1", "tc-1"),
                build_pending_record("tc-1", "write", &json!({}), 0, 60_000),
            )
            .await;

        let exec = FakeExecutor::default();
        let out = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &empty_policy_rules(),
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "deny",
                "denial": { "kind": "not_a_real_kind" },
            }),
            1_500,
        )
        .await;
        assert_eq!(out["ok"], false);
        assert_eq!(out["error"], "bad_denial");
    }

    #[test]
    fn transition_record_to_executed_attaches_result() {
        let base = build_pending_record(
            "tc-1",
            "shell::fs::write",
            &json!({"path":"/a"}),
            1_000,
            60_000,
        );
        let rec = transition_record(&base, "executed", Some(json!({"ok": true})), None, None);
        assert_eq!(rec["status"], "executed");
        assert_eq!(rec["result"], json!({"ok": true}));
        assert!(rec.get("error").is_none() || rec["error"].is_null());
        assert_eq!(rec["function_call_id"], "tc-1");
        assert_eq!(rec["function_id"], "shell::fs::write");
    }

    #[test]
    fn transition_record_to_failed_attaches_error() {
        let base = build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000);
        let rec = transition_record(&base, "failed", None, Some("EACCES".into()), None);
        assert_eq!(rec["status"], "failed");
        assert_eq!(rec["error"], "EACCES");
        assert!(rec.get("result").is_none() || rec["result"].is_null());
    }

    #[test]
    fn transition_record_to_denied_attaches_structured_denial() {
        let base = build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000);
        let rec = transition_record(
            &base,
            "denied",
            None,
            None,
            Some(Denial::Policy {
                classifier_reason: "not authorized".into(),
                classifier_fn: "shell::classify_argv".into(),
            }),
        );
        assert_eq!(rec["status"], "denied");
        assert_eq!(rec["denial"]["kind"], "policy");
        assert_eq!(rec["denial"]["detail"]["classifier_reason"], "not authorized");
        assert!(
            rec.get("decision_reason").is_none(),
            "legacy decision_reason must not be written: {rec}"
        );
    }

    #[test]
    fn transition_record_to_timed_out_carries_no_denial() {
        // Timeout status is self-describing — no Denial attached.
        let base = build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000);
        let rec = transition_record(&base, "timed_out", None, None, None);
        assert_eq!(rec["status"], "timed_out");
        assert!(rec.get("denial").is_none());
        assert!(rec.get("decision_reason").is_none());
    }

    #[test]
    fn transition_record_preserves_delivered_in_turn_id_when_set() {
        let mut base = build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000);
        base.as_object_mut().unwrap().insert(
            "delivered_in_turn_id".into(),
            Value::String("turn-X".into()),
        );
        let rec = transition_record(&base, "executed", Some(json!({"ok": true})), None, None);
        assert_eq!(rec["delivered_in_turn_id"], "turn-X");
    }

    #[tokio::test]
    async fn handle_sweep_session_flips_pending_records_to_timed_out() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "c1"),
            build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
        )
        .await
        .unwrap();

        let resp = handle_sweep_session(&bus, STATE_SCOPE, json!({"session_id": "s1"})).await;
        assert_eq!(resp["swept"], json!(1));

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "c1"))
            .await
            .unwrap();
        assert_eq!(rec["status"], "timed_out");
        // sweep_session no longer stamps a reason string — timed_out is
        // self-describing per the Denial refactor.
        assert!(rec.get("denial").is_none());
        assert!(rec.get("decision_reason").is_none());
    }

    #[tokio::test]
    async fn handle_sweep_session_ignores_legacy_reason_payload_field() {
        // Old callers may still pass `reason` — approval-gate accepts the
        // payload but does not persist it. Behavior is identical to a
        // bare {session_id} payload.
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "c1"),
            build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
        )
        .await
        .unwrap();
        let resp = handle_sweep_session(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "reason": "run_stopped"}),
        )
        .await;
        assert_eq!(resp["swept"], json!(1));
        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "c1"))
            .await
            .unwrap();
        assert_eq!(rec["status"], "timed_out");
        assert!(rec.get("denial").is_none());
    }

    #[tokio::test]
    async fn handle_sweep_session_skips_non_pending_records() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "c1"),
            transition_record(
                &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
            ),
        )
        .await
        .unwrap();

        let resp = handle_sweep_session(&bus, STATE_SCOPE, json!({"session_id": "s1"})).await;
        assert_eq!(resp["swept"], json!(0));

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "c1"))
            .await
            .unwrap();
        assert_eq!(rec["status"], "executed");
    }

    #[tokio::test]
    async fn handle_sweep_session_returns_error_when_session_id_missing() {
        let bus = InMemoryStateBus::new();
        let resp = handle_sweep_session(&bus, STATE_SCOPE, json!({})).await;
        assert_eq!(resp["ok"], json!(false));
        assert_eq!(resp["error"], "missing_session_id");
        assert_eq!(resp["swept"], json!(0));
    }

    // ── New reliability fixes ─────────────────────────────────────────────

    /// A bus that always refuses writes, to exercise fail-closed semantics.
    struct FailingStateBus;

    #[async_trait::async_trait]
    impl StateBus for FailingStateBus {
        async fn set(
            &self,
            _scope: &str,
            _key: &str,
            _value: Value,
        ) -> Result<(), iii_sdk::IIIError> {
            Err(iii_sdk::IIIError::Runtime("kv unreachable".into()))
        }
        async fn get(&self, _scope: &str, _key: &str) -> Option<Value> {
            None
        }
        async fn list_prefix(&self, _scope: &str, _prefix: &str) -> Vec<Value> {
            Vec::new()
        }
    }

    #[tokio::test]
    async fn handle_intercept_fails_closed_on_state_write_error() {
        let bus = FailingStateBus;
        let call = sample_call();
        let reply = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, false).await;
        assert_eq!(
            reply["block"],
            json!(true),
            "state write failure must NOT fail-open"
        );
        assert_eq!(reply["status"], json!("denied"));
        assert_eq!(reply["denial"]["kind"], json!("state_error"));
        assert_eq!(
            reply["denial"]["detail"]["phase"],
            json!("intercept_write_pending")
        );
        // The underlying error message is present but its exact text is
        // bus-implementation-specific; just check it's non-empty.
        assert!(
            reply["denial"]["detail"]["error"]
                .as_str()
                .map(|s| !s.is_empty())
                .unwrap_or(false),
            "state_error detail must include error message: {reply}"
        );
        assert_eq!(reply["function_id"], json!("shell::fs::write"));
    }

    #[tokio::test]
    async fn handle_intercept_stamps_session_id_into_pending_record() {
        let bus = InMemoryStateBus::new();
        let call = sample_call();
        let _ = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, false).await;
        let rec = bus
            .get(
                STATE_SCOPE,
                &pending_key(&call.session_id, &call.function_call_id),
            )
            .await
            .expect("pending record");
        assert_eq!(rec["session_id"], json!(call.session_id));
    }

    #[test]
    fn collect_timed_out_for_sweep_returns_expired_records_with_session_id() {
        let mut rec = build_pending_record("tc-1", "shell::fs::write", &json!({}), 0, 60_000);
        rec.as_object_mut()
            .unwrap()
            .insert("session_id".into(), json!("s-42"));
        let pile = vec![
            rec.clone(),
            build_pending_record("tc-2", "shell::fs::write", &json!({}), 0, 999_999_999),
        ];
        let out = collect_timed_out_for_sweep(&pile, 70_000);
        assert_eq!(out.len(), 1);
        let (key, flipped, session_id, call_id) = &out[0];
        assert_eq!(key, "s-42/tc-1");
        assert_eq!(session_id, "s-42");
        assert_eq!(call_id, "tc-1");
        assert_eq!(flipped["status"], json!("timed_out"));
        // Timeout carries no Denial — status is self-describing.
        assert!(flipped.get("denial").is_none());
        assert!(flipped.get("decision_reason").is_none());
    }

    #[test]
    fn collect_timed_out_for_sweep_skips_records_without_session_id() {
        // Legacy row (pre-session_id-stamping fix). The sweeper can't
        // address the right session stream, so it must skip silently —
        // lazy-flip on read will still pick it up.
        let pile = vec![build_pending_record(
            "tc-legacy",
            "shell::fs::write",
            &json!({}),
            0,
            60_000,
        )];
        let out = collect_timed_out_for_sweep(&pile, 70_000);
        assert!(
            out.is_empty(),
            "legacy record without session_id must not be swept"
        );
    }

    #[test]
    fn timeout_resolved_event_shape() {
        let evt = timeout_resolved_event("tc-1");
        assert_eq!(evt["type"], "approval_resolved");
        assert_eq!(evt["function_call_id"], "tc-1");
        assert_eq!(evt["tool_call_id"], "tc-1");
        assert_eq!(evt["decision"], "deny");
        assert_eq!(evt["status"], "timed_out");
        // timed_out is self-describing — no Denial / no legacy reason.
        assert!(evt.get("decision_reason").is_none());
        assert!(evt.get("denial").is_none());
    }

    #[test]
    fn unverified_marker_targets_lists_unasserted_rules() {
        let rules = vec![
            InterceptorRule {
                function_id: "shell::exec".into(),
                classifier: None,
                classifier_timeout_ms: 2000,
                inject_approval_marker: true,
                marker_target_verified: false,
            },
            InterceptorRule {
                function_id: "shell::exec_bg".into(),
                classifier: None,
                classifier_timeout_ms: 2000,
                inject_approval_marker: true,
                marker_target_verified: true,
            },
            InterceptorRule {
                function_id: "no_marker::fn".into(),
                classifier: None,
                classifier_timeout_ms: 2000,
                inject_approval_marker: false,
                marker_target_verified: false,
            },
        ];
        assert_eq!(unverified_marker_targets(&rules), vec!["shell::exec"]);
    }

    #[test]
    fn unverified_marker_targets_empty_when_all_verified_or_marker_off() {
        let rules = vec![
            InterceptorRule {
                function_id: "shell::exec".into(),
                classifier: None,
                classifier_timeout_ms: 2000,
                inject_approval_marker: true,
                marker_target_verified: true,
            },
            InterceptorRule {
                function_id: "other".into(),
                classifier: None,
                classifier_timeout_ms: 2000,
                inject_approval_marker: false,
                marker_target_verified: false,
            },
        ];
        assert!(unverified_marker_targets(&rules).is_empty());
    }

    // ── Boundary + edge-case tests prompted by cargo-mutants survivors ────
    //
    // Each test corresponds to a mutant the test suite previously didn't
    // catch. Test name → mutated line in src/lib.rs.

    #[test]
    fn merge_from_approval_wraps_null_args_in_marker_only() {
        // mutant L48: replace `other.is_null()` match guard
        let out = merge_from_approval_marker_if_needed(true, Value::Null, "c1", "s1");
        assert!(out.get("__from_approval").is_some());
        assert!(
            out.get("payload").is_none(),
            "null-arg branch must NOT wrap as payload"
        );
    }

    #[test]
    fn merge_from_approval_wraps_scalar_args_in_payload() {
        // mutant L48: same guard, the other branch
        let out = merge_from_approval_marker_if_needed(true, json!("scalar"), "c1", "s1");
        assert!(out.get("__from_approval").is_some());
        assert_eq!(
            out.get("payload"),
            Some(&json!("scalar")),
            "scalar-arg branch must wrap original under `payload`"
        );
    }

    #[tokio::test]
    async fn handle_intercept_replay_of_terminal_record_returns_already_resolved() {
        // mutant L331: replace `==` with `!=` in the replay defense — if
        // flipped, terminal records would be overwritten with fresh pending.
        let bus = InMemoryStateBus::new();
        let call = sample_call();
        let key = pending_key(&call.session_id, &call.function_call_id);
        let terminal = transition_record(
            &build_pending_record(
                &call.function_call_id,
                &call.function_id,
                &call.args,
                0,
                60_000,
            ),
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
        );
        bus.set(STATE_SCOPE, &key, terminal).await.unwrap();

        let reply = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, false).await;
        assert_eq!(reply["block"], json!(true));
        assert_eq!(reply["status"], json!("executed"));
        // Replay reply: status carries the prior outcome, `replay` discriminator
        // says we're echoing rather than denying afresh, and no `denial` is
        // synthesized (the historical record is the source of truth).
        assert_eq!(reply["replay"], json!("already_resolved"));
        assert!(reply.get("denial").is_none());
        assert!(reply.get("reason").is_none());

        // Crucial: the stored row is still `executed`, not overwritten.
        let stored = bus.get(STATE_SCOPE, &key).await.unwrap();
        assert_eq!(stored["status"], json!("executed"));
        assert_eq!(stored["result"], json!({"ok": true}));
    }

    #[tokio::test]
    async fn handle_intercept_replay_of_pending_record_preserves_expires_at() {
        // mutant L331: same branch, pending side. New pending must not bump
        // the expires_at on the existing row.
        let bus = InMemoryStateBus::new();
        let call = sample_call();
        let key = pending_key(&call.session_id, &call.function_call_id);
        let pending = build_pending_record(
            &call.function_call_id,
            &call.function_id,
            &call.args,
            0,
            60_000,
        );
        bus.set(STATE_SCOPE, &key, pending.clone()).await.unwrap();

        let _ = handle_intercept(&bus, STATE_SCOPE, &call, 999_000, 60_000, false).await;
        let stored = bus.get(STATE_SCOPE, &key).await.unwrap();
        assert_eq!(
            stored["expires_at"], pending["expires_at"],
            "replay must not bump expires_at on the live row"
        );
    }

    #[tokio::test]
    async fn handle_lookup_record_rejects_when_only_one_id_is_empty() {
        // mutant L395: `||` → `&&` would let one-empty slip through.
        let bus = InMemoryStateBus::new();
        let v1 = handle_lookup_record(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "", "function_call_id": "c"}),
        )
        .await;
        assert!(v1.is_null());
        let v2 = handle_lookup_record(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s", "function_call_id": ""}),
        )
        .await;
        assert!(v2.is_null());
    }

    #[tokio::test]
    async fn handle_resolve_rejects_when_only_one_id_is_empty() {
        // mutant L489: same `||` pattern in handle_resolve guard.
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        let r1 = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &empty_policy_rules(),
            json!({"session_id": "", "function_call_id": "c", "decision": "allow"}),
            0,
        )
        .await;
        assert_eq!(r1["error"], json!("missing_id"));
        let r2 = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &empty_policy_rules(),
            json!({"session_id": "s", "function_call_id": "", "decision": "allow"}),
            0,
        )
        .await;
        assert_eq!(r2["error"], json!("missing_id"));
    }

    #[tokio::test]
    async fn handle_ack_delivered_returns_zero_when_only_one_field_is_empty() {
        // mutant L677: two `||` operators in the empty-field guard.
        let bus = InMemoryStateBus::new();
        // empty turn_id
        let r1 = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s", "turn_id": "", "call_ids": ["c"]}),
        )
        .await;
        assert_eq!(r1["stamped"], json!(0));
        // empty call_ids
        let r2 = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s", "turn_id": "t", "call_ids": []}),
        )
        .await;
        assert_eq!(r2["stamped"], json!(0));
        // empty session_id
        let r3 = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "", "turn_id": "t", "call_ids": ["c"]}),
        )
        .await;
        assert_eq!(r3["stamped"], json!(0));
    }

    #[test]
    fn collect_timed_out_for_sweep_rejects_record_missing_only_call_id() {
        // mutant L423: `||` → `&&` would let one-empty records sweep.
        let mut rec = build_pending_record("c1", "shell::fs::write", &json!({}), 0, 60_000);
        rec.as_object_mut()
            .unwrap()
            .insert("session_id".into(), json!("s1"));
        rec.as_object_mut()
            .unwrap()
            .insert("function_call_id".into(), json!(""));
        let out = collect_timed_out_for_sweep(&[rec], 70_000);
        assert!(out.is_empty(), "empty function_call_id must skip sweep");
    }

    #[tokio::test]
    async fn handle_intercept_replay_of_approved_record_preserves_state() {
        // mutant L331:42 — replace `==` with `!=` on the "approved" side.
        // The L331:19 mutation is killed by the *_pending_* test above;
        // this one requires an approved record specifically.
        let bus = InMemoryStateBus::new();
        let call = sample_call();
        let key = pending_key(&call.session_id, &call.function_call_id);
        let approved = transition_record(
            &build_pending_record(
                &call.function_call_id,
                &call.function_id,
                &call.args,
                0,
                60_000,
            ),
            "approved",
            None,
            None,
            None,
        );
        bus.set(STATE_SCOPE, &key, approved.clone()).await.unwrap();

        let _ = handle_intercept(&bus, STATE_SCOPE, &call, 999_000, 60_000, false).await;
        let stored = bus.get(STATE_SCOPE, &key).await.unwrap();
        assert_eq!(
            stored["status"],
            json!("approved"),
            "replay of approved row must keep status; mutant would overwrite with pending"
        );
    }

    #[tokio::test]
    async fn handle_lookup_record_short_circuits_before_bus_get_on_one_empty_id() {
        // mutant L395 — `||` → `&&` would let one-empty slip into bus.get.
        // Seed a record at the address the mutant would compute (pending_key("", "c") = "/c"),
        // so the mutant returns the seeded row while original code stays at Null.
        let bus = InMemoryStateBus::new();
        bus.set(STATE_SCOPE, "/c", json!({"sentinel": "should_not_leak"}))
            .await
            .unwrap();
        let v = handle_lookup_record(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "", "function_call_id": "c"}),
        )
        .await;
        assert!(
            v.is_null(),
            "must short-circuit; the seeded sentinel must not leak through"
        );
    }

    #[tokio::test]
    async fn handle_ack_delivered_short_circuits_before_stamping_on_one_empty_field() {
        // mutant L677 — two `||` operators. If either flips to `&&`, the
        // function falls through and stamps a record even when a required
        // field is empty. Seed a record so the stamping path can be
        // observed.
        let bus = InMemoryStateBus::new();
        let terminal = transition_record(
            &build_pending_record("c", "shell::fs::write", &json!({}), 0, 60_000),
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
        );
        bus.set(STATE_SCOPE, &pending_key("s", "c"), terminal)
            .await
            .unwrap();

        // empty turn_id — must NOT stamp the seeded record.
        let r = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s", "turn_id": "", "call_ids": ["c"]}),
        )
        .await;
        assert_eq!(r["stamped"], json!(0));
        let stored = bus.get(STATE_SCOPE, &pending_key("s", "c")).await.unwrap();
        assert!(
            stored.get("delivered_in_turn_id").is_none(),
            "must not stamp when turn_id is empty; mutant would stamp"
        );

        // empty call_ids — same property.
        let r = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s", "turn_id": "t", "call_ids": []}),
        )
        .await;
        assert_eq!(r["stamped"], json!(0));
        let stored = bus.get(STATE_SCOPE, &pending_key("s", "c")).await.unwrap();
        assert!(
            stored.get("delivered_in_turn_id").is_none(),
            "must not stamp when call_ids is empty"
        );
    }

    #[test]
    fn maybe_flip_timed_out_flips_at_exact_expires_at() {
        // mutant L439: `<` → `<=` would not flip at the exact boundary.
        let rec = build_pending_record("c1", "f", &json!({}), 0, 60_000);
        // expires_at = 0 + 60_000 = 60_000. At now=60_000 the gate
        // considers the record expired (strictly past or AT expiry).
        assert!(
            maybe_flip_timed_out(&rec, 60_000).is_some(),
            "must flip at exactly expires_at"
        );
        assert!(
            maybe_flip_timed_out(&rec, 59_999).is_none(),
            "must not flip one ms before expires_at"
        );
    }

    // ── proptest: state-machine invariants ────────────────────────────────
    //
    // Random sequences of intercept/resolve/sweep/ack/lazy-flip operations
    // on a single (session, call) record. After every step we assert four
    // invariants that the lifecycle is supposed to guarantee:
    //
    //   I1. status ∈ {pending, approved, executed, failed, denied, timed_out}.
    //       Any other string is a corrupt record.
    //   I2. Once a terminal status is observed, the record never returns to
    //       `pending`. Terminal = executed | failed | denied | timed_out.
    //   I3. Every `pending` record carries an `expires_at: u64`. Without it
    //       the sweeper and lazy-flip paths can't classify the record.
    //   I4. `delivered_in_turn_id` is monotonic: once a non-null value is
    //       written it is never unset, never replaced with a different turn.
    //
    // If any future change can produce a sequence that violates one of
    // these, proptest will shrink to the minimal failing sequence and
    // surface it as a counterexample.

    use proptest::prelude::*;

    #[derive(Debug, Clone)]
    enum Op {
        InterceptRequired,
        InterceptNotRequired,
        ResolveAllow,
        ResolveDeny,
        AdvanceClockAndLazyFlip, // bumps clock past expires_at, hits list_undelivered
        SweepSession,
        AckDelivered,
    }

    fn arb_op() -> impl Strategy<Value = Op> {
        prop_oneof![
            Just(Op::InterceptRequired),
            Just(Op::InterceptNotRequired),
            Just(Op::ResolveAllow),
            Just(Op::ResolveDeny),
            Just(Op::AdvanceClockAndLazyFlip),
            Just(Op::SweepSession),
            Just(Op::AckDelivered),
        ]
    }

    fn make_call(approval_required_self: bool) -> IncomingCall {
        IncomingCall {
            session_id: "s".into(),
            function_call_id: "c".into(),
            function_id: "test::write".into(),
            args: json!({}),
            approval_required: if approval_required_self {
                vec!["test::write".into()]
            } else {
                vec!["other::fn".into()]
            },
            event_id: "e".into(),
            reply_stream: "r".into(),
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 256,
            .. ProptestConfig::default()
        })]

        #[test]
        fn state_machine_invariants(ops in prop::collection::vec(arb_op(), 1..30)) {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");

            rt.block_on(async {
                let bus = InMemoryStateBus::new();
                let exec = FakeExecutor::default();
                let session_id = "s";
                let call_id = "c";
                let timeout_ms: u64 = 60_000;
                let mut now_ms: u64 = 1_000;

                let mut ever_terminal = false;
                let mut last_delivered: Option<String> = None;

                for op in &ops {
                    match op {
                        Op::InterceptRequired => {
                            let call = make_call(true);
                            let _ = handle_intercept(&bus, STATE_SCOPE, &call, now_ms, timeout_ms, false).await;
                        }
                        Op::InterceptNotRequired => {
                            let call = make_call(false);
                            let _ = handle_intercept(&bus, STATE_SCOPE, &call, now_ms, timeout_ms, false).await;
                        }
                        Op::ResolveAllow => {
                            let _ = handle_resolve(
                                &bus,
                                &exec,
                                STATE_SCOPE,
                                &empty_policy_rules(),
                                json!({
                                    "session_id": session_id,
                                    "function_call_id": call_id,
                                    "decision": "allow",
                                }),
                                now_ms,
                            )
                            .await;
                        }
                        Op::ResolveDeny => {
                            let _ = handle_resolve(
                                &bus,
                                &exec,
                                STATE_SCOPE,
                                &empty_policy_rules(),
                                json!({
                                    "session_id": session_id,
                                    "function_call_id": call_id,
                                    "decision": "deny",
                                }),
                                now_ms,
                            )
                            .await;
                        }
                        Op::AdvanceClockAndLazyFlip => {
                            now_ms = now_ms.saturating_add(timeout_ms + 1);
                            let _ = handle_list_undelivered(
                                &bus, STATE_SCOPE,
                                json!({ "session_id": session_id }),
                                now_ms,
                            ).await;
                        }
                        Op::SweepSession => {
                            let _ = handle_sweep_session(
                                &bus, STATE_SCOPE,
                                json!({ "session_id": session_id }),
                            ).await;
                        }
                        Op::AckDelivered => {
                            let _ = handle_ack_delivered(
                                &bus, STATE_SCOPE,
                                json!({
                                    "session_id": session_id,
                                    "turn_id": format!("turn-{now_ms}"),
                                    "call_ids": [call_id],
                                }),
                            ).await;
                        }
                    }

                    // Assert invariants on whatever the record currently is.
                    let key = pending_key(session_id, call_id);
                    let Some(rec) = bus.get(STATE_SCOPE, &key).await else {
                        // No record yet (e.g. only InterceptNotRequired so far). Skip.
                        continue;
                    };

                    // I1: legal status
                    let status = rec.get("status").and_then(Value::as_str).unwrap_or("");
                    assert!(
                        matches!(
                            status,
                            "pending" | "approved" | "executed" | "failed" | "denied" | "timed_out"
                        ),
                        "I1 violated: illegal status {status:?} after ops {ops:?}; record={rec:?}"
                    );

                    // I2: no reverting terminal → pending
                    if matches!(status, "executed" | "failed" | "denied" | "timed_out") {
                        ever_terminal = true;
                    }
                    if ever_terminal {
                        assert!(
                            status != "pending",
                            "I2 violated: reverted to pending after terminal; ops={ops:?}; record={rec:?}"
                        );
                    }

                    // I3: pending records always have expires_at: u64
                    if status == "pending" {
                        let exp = rec.get("expires_at").and_then(Value::as_u64);
                        assert!(
                            exp.is_some(),
                            "I3 violated: pending record missing expires_at; ops={ops:?}; record={rec:?}"
                        );
                    }

                    // I4: delivered_in_turn_id is monotonic — once set non-null, never unset / never replaced
                    let cur_delivered = rec
                        .get("delivered_in_turn_id")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    if let Some(prev) = &last_delivered {
                        match &cur_delivered {
                            Some(cur) => {
                                assert_eq!(
                                    cur, prev,
                                    "I4 violated: delivered_in_turn_id replaced {prev:?} → {cur:?}; ops={ops:?}"
                                );
                            }
                            None => {
                                panic!(
                                    "I4 violated: delivered_in_turn_id unset after being {prev:?}; ops={ops:?}; record={rec:?}"
                                );
                            }
                        }
                    }
                    if cur_delivered.is_some() {
                        last_delivered = cur_delivered;
                    }
                }
            });
        }
    }
}
