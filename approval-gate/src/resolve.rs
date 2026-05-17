//! Resolve flow — handles `approval::resolve` and the cascading-allow
//! behavior that fires when a reply carries `always: true`.
//!
//! ## Three-phase allow path
//!
//! [`handle_resolve`] is the entry point. On allow it routes through
//! [`approve_and_execute`]:
//!   1. write `InFlight` (closes the dup-exec race — a second resolve
//!      arriving during the invoke await sees a non-Pending row and bails);
//!   2. `iii.trigger(function_id, args)` and await;
//!   3. write `Done(Executed{result})` or `Done(Failed{error})`.
//!
//! Deny is a single Pending → Done(Denied) write — no invoke, no InFlight.
//!
//! ## Cascade
//!
//! On `allow + always:true`, [`cascade_allow_for_session`] pushes a runtime
//! `Allow` rule with the originator's **exact pattern** (via
//! [`crate::rules::pattern_for`]) — not a blanket `pattern: "*"`. "Always
//! allow git status" does NOT auto-allow `rm -rf /` via the same
//! `shell::exec` function id. Same-session pending rows whose
//! `verdict_for` returns `Allow` under the new rule are driven through
//! `approve_and_execute`.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use serde_json::{json, Value};
use tokio::sync::Mutex as AsyncMutex;

use crate::record::{Outcome, Record, Status};
use crate::rules::{self, Action, LayeredRules, Rule};
use crate::state::{FunctionExecutor, StateBus};
use crate::wire::{pending_key, Denial, WireDecision};

/// Process-local per-key serialization for `approval::resolve`.
///
/// Closes finding #3: two concurrent `approval::resolve` calls for the
/// same `(session_id, function_call_id)` could both observe `Pending`
/// before either's `InFlight` write landed and both call the executor.
/// `StateBus` has no compare-and-set primitive (the iii state backend
/// supports only unconditional set), so we serialize the read-then-write
/// inside the worker process using a per-key async mutex. Cross-process
/// races (two workers consuming the same scope) remain — out of scope
/// for the current single-worker deployment.
fn resolve_key_lock(key: &str) -> Arc<AsyncMutex<()>> {
    static GUARDS: OnceLock<std::sync::Mutex<HashMap<String, Arc<AsyncMutex<()>>>>> =
        OnceLock::new();
    let map = GUARDS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = map.lock().expect("resolve key-lock map poisoned");
    guard
        .entry(key.to_string())
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
}

/// Augment the original args with the orchestrator-style context fields.
///
/// Closes finding #7: the normal `function_execute` path in
/// `turn-orchestrator/src/states/functions.rs` adds these keys before
/// dispatching; the approval path used to forward bare args, so target
/// handlers behaved differently after an operator approval than they did
/// on the auto-allowed path. This helper keeps the two shapes identical.
fn augment_args(args: &Value, session_id: &str, function_call_id: &str, function_id: &str) -> Value {
    let mut augmented = match args.clone() {
        Value::Object(o) => Value::Object(o),
        other => json!({ "arguments": other }),
    };
    if let Some(obj) = augmented.as_object_mut() {
        obj.insert("session_id".into(), json!(session_id));
        obj.insert("function_call_id".into(), json!(function_call_id));
        obj.insert("function_id".into(), json!(function_id));
        obj.insert(
            "function_call".into(),
            json!({
                "id": function_call_id,
                "function_id": function_id,
                "arguments": args,
            }),
        );
    }
    augmented
}

/// Lookup a single approval record by session + call id (for shell bypass
/// validation). Stays on the old free-form Value shape so shell-side
/// readers don't break — shell strip in T13 deletes the callsite there.
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

/// Resolve a pending approval. Wire-format errors return
/// `{ok:false, error:"<reason>"}`. Success returns `{ok:true}` plus an
/// optional `cascaded: N` count when an `always:true` reply triggered the
/// session sweep.
pub async fn handle_resolve(
    bus: &dyn StateBus,
    exec: &dyn FunctionExecutor,
    state_scope: &str,
    policy_rules: &RwLock<LayeredRules>,
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

    // Atomic guard for finding #3: serialize concurrent resolves for the
    // same key. The lock is released when this function returns; a
    // racer that arrives during the invoke sees the InFlight/Done row
    // and returns a typed error.
    let key_lock = resolve_key_lock(&key);
    let _key_guard = key_lock.lock().await;

    let Some(raw) = bus.get(state_scope, &key).await else {
        return json!({ "ok": false, "error": "not_found" });
    };
    let Some(record) = Record::from_value(raw) else {
        return json!({ "ok": false, "error": "corrupt_record" });
    };

    // Lazy timeout flip — Pending rows past expires_at flip to
    // Done(TimedOut) on read.
    if let Some(flipped) = record.flipped_to_timed_out_if_expired(now_ms) {
        let _ = bus.set(state_scope, &key, flipped.to_value()).await;
        return json!({ "ok": false, "error": "timed_out" });
    }

    // Dup-exec guard: only Pending rows are resolvable. InFlight means a
    // concurrent resolve is still mid-invoke; Done means terminal.
    match record.status {
        Status::Pending => { /* fall through */ }
        Status::InFlight => return json!({ "ok": false, "error": "in_flight" }),
        Status::Done => return json!({ "ok": false, "error": "already_resolved" }),
    }

    match decision {
        WireDecision::Deny => {
            // Optional structured denial from caller; missing → UserRejected.
            let denial = match payload.get("denial").cloned() {
                Some(v) => match serde_json::from_value::<Denial>(v) {
                    Ok(d) => d,
                    Err(_) => return json!({ "ok": false, "error": "bad_denial" }),
                },
                None => Denial::UserRejected,
            };
            let denied = record.done_at(now_ms, Outcome::Denied { denial });
            if let Err(e) = bus.set(state_scope, &key, denied.to_value()).await {
                tracing::error!("approval-gate: failed to write denied record: {e}");
                return json!({ "ok": false, "error": "state_write_failed" });
            }
            json!({ "ok": true })
        }
        WireDecision::Allow => {
            // Snapshot args + function_id before consuming `record` in
            // approve_and_execute — cascade needs them for the rule push.
            let function_id = record.function_id.clone();
            let args = record.args.clone();

            if let Err(err) = approve_and_execute(
                bus,
                exec,
                state_scope,
                record,
                session_id,
                function_call_id,
                now_ms,
            )
            .await
            {
                tracing::error!("approval-gate: failed to execute approved call: {err}");
                return json!({ "ok": false, "error": "state_write_failed" });
            }

            // Cascade on `always:true`. Push a runtime Allow rule with the
            // ORIGINATOR'S EXACT PATTERN (via pattern_for), then sweep the
            // session's other Pending rows.
            let cascaded = if payload
                .get("always")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                cascade_allow_for_session(
                    bus,
                    exec,
                    state_scope,
                    policy_rules,
                    session_id,
                    function_call_id,
                    &function_id,
                    &args,
                    now_ms,
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

/// Push an exact-pattern Allow rule into the shared ruleset, then sweep
/// the session's other Pending rows. Returns the number of rows
/// auto-resolved (originator excluded).
///
/// **Lock-ordering invariant**: the write/read guards on `policy_rules`
/// are released before any `.await`. `std::sync::RwLock` is not async-safe
/// to hold across suspension; a held guard would block every concurrent
/// intercept.
async fn cascade_allow_for_session(
    bus: &dyn StateBus,
    exec: &dyn FunctionExecutor,
    state_scope: &str,
    policy_rules: &RwLock<LayeredRules>,
    session_id: &str,
    originator_call_id: &str,
    originator_function_id: &str,
    originator_args: &Value,
    now_ms: u64,
) -> u64 {
    // 1. Push the exact-pattern Allow rule into THIS SESSION's overlay
    //    (finding #2). The rule must not leak into other sessions — the
    //    UI tooltip on `allow + always` promises session-local scope.
    //    pattern_for is the same extractor used at intercept time, so
    //    "always allow git status" means literally that argv shape — NOT
    //    a blanket "*" pattern that would auto-allow rm -rf /.
    let pushed_pattern = rules::pattern_for(originator_function_id, originator_args);
    {
        let mut guard = policy_rules
            .write()
            .expect("approval-gate policy rules lock poisoned");
        guard.push_session_rule(
            session_id,
            Rule {
                permission: originator_function_id.to_string(),
                pattern: pushed_pattern,
                action: Action::Allow,
            },
        );
    }

    // 2. Snapshot the session's pending rows.
    let prefix = format!("{session_id}/");
    let session_rows = bus.list_prefix(state_scope, &prefix).await;

    let mut cascaded = 0u64;
    for raw in session_rows {
        let Some(record) = Record::from_value(raw) else {
            continue;
        };
        if record.session_id != session_id {
            continue;
        } // defensive
        if record.function_call_id == originator_call_id {
            continue;
        } // skip originator
        if record.status != Status::Pending {
            continue;
        } // skip non-pending

        // 3. Re-evaluate against the per-session snapshot (includes the
        //    just-pushed Allow rule for THIS session only).
        let verdict = {
            let guard = policy_rules
                .read()
                .expect("approval-gate policy rules lock poisoned");
            let snapshot = guard.snapshot_for(session_id);
            crate::verdict_for(&record.function_id, &record.args, &snapshot)
        };
        if !matches!(verdict, crate::Verdict::Allow) {
            continue;
        }

        // 4. Drive through the same approve_and_execute path as the
        //    user-driven allow (InFlight → invoke → Done).
        let cid = record.function_call_id.clone();
        if let Err(err) =
            approve_and_execute(bus, exec, state_scope, record, session_id, &cid, now_ms).await
        {
            tracing::warn!(
                session_id, call_id = %cid,
                "approval-gate: cascade auto-resolve failed: {err}",
            );
            continue;
        }
        cascaded += 1;
    }
    cascaded
}

/// Drive a Pending row through InFlight → invoke → Done. Used by both
/// the user-driven allow path and the cascade sweep so the lifecycle
/// transitions stay in one place.
///
/// Phase 1 (InFlight) is the dup-exec guard: a concurrent resolve seeing
/// a non-Pending row in `handle_resolve` returns `in_flight` and skips
/// the second invoke.
pub(crate) async fn approve_and_execute(
    bus: &dyn StateBus,
    exec: &dyn FunctionExecutor,
    state_scope: &str,
    pending: Record,
    session_id: &str,
    function_call_id: &str,
    now_ms: u64,
) -> Result<(), String> {
    let key = pending_key(session_id, function_call_id);
    let function_id = pending.function_id.clone();
    let args = pending.args.clone();

    // Phase 1: InFlight write. Closes the dup-exec race.
    let in_flight = pending.in_flight(now_ms);
    bus.set(state_scope, &key, in_flight.to_value())
        .await
        .map_err(|e| e.to_string())?;

    // Phase 2: invoke. Augment args so the target sees the same shape
    // it would on the normal `function_execute` path (finding #7).
    let augmented = augment_args(&args, session_id, function_call_id, &function_id);
    let outcome = match exec
        .invoke(&function_id, augmented, function_call_id, session_id)
        .await
    {
        Ok(result) => Outcome::Executed { result },
        Err(error) => Outcome::Failed { error },
    };

    // Phase 3: Done write. resolved_at preserved from the InFlight write.
    let done = in_flight.done(outcome);
    bus.set(state_scope, &key, done.to_value())
        .await
        .map_err(|e| e.to_string())
}
