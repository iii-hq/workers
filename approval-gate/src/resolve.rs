//! Resolve flow — handles `approval::resolve` and the cascading-allow
//! behavior that fires when a reply carries `always: true`.
//!
//! [`handle_resolve`] is the main entry point. On allow it routes
//! through [`approve_and_execute`], which is also reused by the cascade
//! sweep ([`cascade_allow_for_session`]) so the approved → invoke →
//! executed/failed transitions stay in one place. [`handle_lookup_record`]
//! is the small read-only helper called by shell bypass validation.

use std::sync::RwLock;

use serde_json::{json, Value};

// apply_policy_rules / PolicyOutcome were deleted in T5. The cascade loop
// below uses crate::verdict_for instead. T7 rewrites this entirely.
use crate::lifecycle::{maybe_flip_timed_out, transition_record};
use crate::rules;
use crate::state::{FunctionExecutor, StateBus};
use crate::wire::{pending_key, Denial, WireDecision};

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

/// Resolve a pending approval. Wire-format errors return `{ok: false,
/// error: "<reason>"}`. Success returns `{ok: true}` plus an optional
/// `cascaded: N` count when an `always: true` reply triggered the
/// session sweep.
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

    // Lazy timeout flip: if the record is past expires_at, write the
    // timed_out transition and refuse the resolve so the caller can't
    // race the sweeper.
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
                tracing::error!("approval-gate: failed to execute approved call: {err}");
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
        let args = rec.get("args").cloned().unwrap_or(json!({}));
        let verdict = {
            let guard = policy_rules
                .read()
                .expect("approval-gate policy rules lock poisoned");
            crate::verdict_for(&fn_id, &args, &guard)
        };
        if !matches!(verdict, crate::Verdict::Allow) {
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
