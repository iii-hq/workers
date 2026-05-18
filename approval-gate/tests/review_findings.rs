//! Tests that pin the **expected, correct** behavior for each finding
//! raised in the approval-gate review.
//!
//! These tests are regression coverage for fixes that have landed. Each
//! test header references the finding number and states the invariant in
//! positive form so future changes break loudly if they regress it.
//!
//! Findings covered here (Rust / backend):
//!   - #2 `allow + always` must be session-scoped (UI contract)
//!   - #3 Pending → InFlight transition must be atomic (one invoke per call)
//!   - #4 Pending-row timeouts must wake the orchestrator
//!   - #5 A Done-write failure must not orphan a row in `InFlight` forever
//!   - #6 Concurrent `approval::consume` must not return the same row twice
//!   - #7 Approved execution must augment args the same way the normal
//!         `function_execute` path does
//!   - approval_resolved decision must describe the operator decision,
//!         not executor success/failure
//!
//! Findings #1 (hook fail-open) and #8 (UI ignores `ok:false`) live
//! outside this crate (turn-orchestrator and harness/web respectively)
//! and are not pinned here.

mod common;

use std::sync::{Arc, Mutex, RwLock};

use approval_gate::record::{Outcome, Record, Status};
use approval_gate::{
    handle_consume, handle_intercept, handle_resolve, IncomingCall, LayeredRules, StateBus,
    STATE_SCOPE,
};
use common::{FakeExecutor, InMemoryStateBus};
use serde_json::{json, Value};

fn ask_all_rules() -> LayeredRules {
    LayeredRules::from_global(Some(Vec::new()))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn pending_row(session: &str, cid: &str, function_id: &str, args: Value) -> Record {
    Record::pending(
        cid.to_string(),
        function_id.to_string(),
        args,
        session.to_string(),
        now_ms(),
        60_000,
    )
}

async fn seed_pending(bus: &InMemoryStateBus, r: &Record) {
    bus.set(
        STATE_SCOPE,
        &format!("{}/{}", r.session_id, r.function_call_id),
        r.to_value(),
    )
    .await
    .unwrap();
}

// ────────────────────────────────────────────────────────────────────────
// FINDING #2 — `allow + always` must NOT leak across sessions.
//
// The UI tooltip in `harness/web/src/components/ApprovalRow.tsx`
// promises that `allow + always` only affects "other pending calls in
// THIS session". The runtime rule push must therefore be scoped to the
// originating session_id (or persisted per-session) — not pushed into
// the worker-global ruleset.
// ────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn allow_always_must_not_leak_across_sessions() {
    let bus = InMemoryStateBus::new();
    let exec = FakeExecutor::default();
    let policy = RwLock::new(ask_all_rules());

    let sess_a_pending = pending_row(
        "sess_A",
        "tc-a-1",
        "shell::exec",
        json!({ "command": "git", "args": ["status"] }),
    );
    seed_pending(&bus, &sess_a_pending).await;

    let reply = handle_resolve(
        &bus,
        &exec,
        STATE_SCOPE,
        &policy,
        json!({
            "session_id": "sess_A",
            "function_call_id": "tc-a-1",
            "decision": "allow",
            "always": true,
        }),
        now_ms(),
    )
    .await;
    assert_eq!(reply["ok"], true, "operator's own allow must succeed");

    // A DIFFERENT session asks for the same `git status` shape.
    let call_in_sess_b = IncomingCall {
        session_id: "sess_B".into(),
        function_call_id: "tc-b-1".into(),
        function_id: "shell::exec".into(),
        args: json!({ "command": "git", "args": ["status"] }),
        approval_required: vec!["shell::exec".into()],
        event_id: "evt-b".into(),
        reply_stream: "rs-b".into(),
    };
    let rules_snapshot = policy.read().unwrap().snapshot_for("sess_B");
    let intercept_reply = handle_intercept(
        &bus,
        STATE_SCOPE,
        &call_in_sess_b,
        &rules_snapshot,
        now_ms(),
        60_000,
    )
    .await;

    // Session B must still be paused for operator approval. Session A's
    // `always` rule is not allowed to short-circuit it.
    assert_eq!(
        intercept_reply["block"], true,
        "session B must be paused even though session A pushed `always`",
    );
    assert_eq!(
        intercept_reply["status"], "pending",
        "session B must still ask the operator for approval",
    );
}

// ────────────────────────────────────────────────────────────────────────
// FINDING #3 — Pending → InFlight transition must be atomic.
//
// Two concurrent `approval::resolve` calls for the same
// `(session_id, function_call_id)` must result in EXACTLY one executor
// invocation. The losing call must return a structured `{ok:false,
// error:"in_flight"|"already_resolved"}` reply so the caller can
// react.
//
// Determinism note: the earlier version of this test relied on
// `tokio::Barrier` to align task entry, then trusted scheduler luck for
// the read→check→write race to actually surface. That made the test
// pass on a quiet CPU even with the per-key mutex removed. The version
// below uses [`MaxConcurrentSetBus`] which inserts yield points inside
// `set()` to deterministically widen the race window, and asserts on
// `peak_concurrent_sets <= 1` — that property fails every run when the
// guard is removed.
//
// Manual adversarial check: comment out
//   `let _key_guard = key_lock.lock().await;`
// in `approval-gate/src/resolve.rs` and re-run this test; it must fail
// every time. Restore the line afterwards.
// ────────────────────────────────────────────────────────────────────────
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_resolves_must_invoke_function_at_most_once() {
    use tokio::sync::Barrier;

    let bus = Arc::new(common::MaxConcurrentSetBus::new(8));
    let exec = Arc::new(FakeExecutor::default());
    let policy = Arc::new(RwLock::new(ask_all_rules()));

    let row = pending_row(
        "sess_X",
        "tc-x",
        "shell::exec",
        json!({ "command": "echo", "args": ["hi"] }),
    );
    bus.set(STATE_SCOPE, "sess_X/tc-x", row.to_value())
        .await
        .unwrap();
    // Seeding the row counts as a `set` call. Reset peak so the test
    // only measures concurrency between the two resolves below.
    bus.peak_concurrent_sets
        .store(0, std::sync::atomic::Ordering::SeqCst);

    let payload = json!({
        "session_id": "sess_X",
        "function_call_id": "tc-x",
        "decision": "allow",
    });

    // Start barrier so both tasks reach handle_resolve at the same
    // instant. The bus's set-side yields then guarantee the race
    // window is wide enough to be deterministically observed.
    let start = Arc::new(Barrier::new(2));
    let (bus1, bus2) = (bus.clone(), bus.clone());
    let (exec1, exec2) = (exec.clone(), exec.clone());
    let (policy1, policy2) = (policy.clone(), policy.clone());
    let (p1, p2) = (payload.clone(), payload.clone());
    let (s1, s2) = (start.clone(), start.clone());

    let t1 = tokio::spawn(async move {
        s1.wait().await;
        handle_resolve(
            bus1.as_ref(),
            exec1.as_ref(),
            STATE_SCOPE,
            &policy1,
            p1,
            now_ms(),
        )
        .await
    });
    let t2 = tokio::spawn(async move {
        s2.wait().await;
        handle_resolve(
            bus2.as_ref(),
            exec2.as_ref(),
            STATE_SCOPE,
            &policy2,
            p2,
            now_ms(),
        )
        .await
    });
    let (r1, r2) = tokio::join!(t1, t2);
    let r1 = r1.unwrap();
    let r2 = r2.unwrap();

    // Deterministic property: the bus never saw two `set()` calls in
    // flight at the same time. Without the per-key mutex this would
    // hit 2 every run (the yields keep both writes overlapping).
    assert!(
        bus.peak() <= 1,
        "per-key mutex must serialize the read→check→write window; \
         saw {} concurrent set() calls in flight",
        bus.peak(),
    );

    // Exactly one invoke.
    let invocations = exec.calls.lock().unwrap().len();
    assert_eq!(
        invocations, 1,
        "atomic guard must permit exactly one invoke per call (saw {invocations})",
    );

    // Exactly one ok-true reply; the other must be a typed loss.
    let ok_count = [&r1, &r2].iter().filter(|r| r["ok"] == true).count();
    assert_eq!(
        ok_count, 1,
        "exactly one resolve wins; the other must return ok:false with a typed error",
    );
    let loser = if r1["ok"] == true { &r2 } else { &r1 };
    let err = loser["error"].as_str().unwrap_or("");
    assert!(
        matches!(err, "in_flight" | "already_resolved"),
        "losing resolve must return a typed lose-the-race error, got {err:?}",
    );
}

// ────────────────────────────────────────────────────────────────────────
// FINDING #4 — Pending-row timeouts must wake the orchestrator.
//
// When a Pending row's `expires_at` passes, the orchestrator must be
// woken so it drains the timed-out entry into the assistant turn
// (otherwise the LLM's `pending_approval` placeholder is permanent).
//
// The exact shape is design-flexible — could be a registered
// `approval::wake_timeouts` / `approval::tick` function, a sweeper task
// kicked from `register()`, or a side-effect emitted from
// `handle_list_pending` / `handle_consume`. This test pins the contract
// that SOME timeout-driven wake surface exists in the gate's public API.
// ────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn timed_out_rows_must_have_a_wake_surface() {
    // Seed an already-expired Pending row so a fix that adds an
    // automatic wake on a timer has something to observe.
    let bus = InMemoryStateBus::new();
    let mut r = pending_row("sess_T", "tc-t", "shell::exec", json!({}));
    r.expires_at = 1;
    seed_pending(&bus, &r).await;

    // Confirm the row is reachable via the existing consume path (the
    // post-fix wake mechanism is allowed to lean on this).
    let resp = handle_consume(
        &bus,
        STATE_SCOPE,
        json!({ "session_id": "sess_T" }),
        now_ms(),
    )
    .await;
    assert_eq!(
        resp["entries"].as_array().unwrap()[0]["outcome"]["kind"],
        "timed_out",
        "precondition: expired row surfaces as timed_out on consume",
    );

    // Re-seed (consume deleted it) so the wake surface has something to
    // see after the precondition check.
    seed_pending(&bus, &r).await;

    // Drive the watchdog directly: it must reclaim the expired row and
    // report the affected session so the registered function can call
    // `run::resume` for it (see register.rs).
    let tick = approval_gate::handle_tick_timeouts(&bus, STATE_SCOPE, json!({}), now_ms()).await;
    assert_eq!(tick["ok"], true);
    assert_eq!(
        tick["reclaimed"], 1,
        "watchdog must reclaim the expired Pending row",
    );
    let sessions = tick["sessions_woken"].as_array().unwrap();
    assert_eq!(
        sessions
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>(),
        vec!["sess_T"],
        "watchdog must report sess_T so the orchestrator gets woken",
    );

    // Contract: the gate exposes a public timeout-driven wake surface.
    // `approval::tick_timeouts` is the chosen shape: it performs the
    // flip-and-wake fan-out directly and is also called by the auto-tick
    // loop registered in approval-gate/src/register.rs.
    let surfaces = [
        approval_gate::FN_RESOLVE,
        approval_gate::FN_LIST_PENDING,
        approval_gate::FN_CONSUME,
        approval_gate::FN_SWEEP_SESSION,
        approval_gate::FN_LOOKUP_RECORD,
        approval_gate::FN_TICK_TIMEOUTS,
    ];
    let has_wake_surface = surfaces
        .iter()
        .any(|s| s.contains("tick") || s.contains("wake") || s.contains("watchdog"));
    assert!(
        has_wake_surface,
        "expected the gate to expose a timeout-driven wake surface so \
         expired Pending rows can re-kick the orchestrator; today's \
         surfaces ({surfaces:?}) only fire on operator action",
    );
}

// ────────────────────────────────────────────────────────────────────────
// FINDING #4 follow-up — `run_tick` must call `run::resume` for each
// reclaimed session.
//
// `timed_out_rows_must_have_a_wake_surface` above pins the *contract*
// (handle_tick_timeouts reports sessions_woken; a function-id constant
// exists). The tests below pin the *behavior*: when `run_tick` runs it
// actually invokes the orchestrator's `run::resume` for every session
// in the list, and when the trigger fails the wake-failure event is
// surfaced. They use a [`FakeWaker`] to observe the iii-side calls
// without booting a real engine.
// ────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn run_tick_calls_resume_for_each_woken_session() {
    let bus = InMemoryStateBus::new();
    let mut a = pending_row("sess_A", "tc-a", "shell::exec", json!({}));
    a.expires_at = 1;
    seed_pending(&bus, &a).await;
    let mut b = pending_row("sess_B", "tc-b", "shell::exec", json!({}));
    b.expires_at = 1;
    seed_pending(&bus, &b).await;

    let waker = common::FakeWaker::default();
    let tick = approval_gate::run_tick(&waker, &bus, STATE_SCOPE, json!({})).await;
    assert_eq!(tick["reclaimed"], 2);

    let mut got = waker.resume_calls.lock().unwrap().clone();
    got.sort();
    assert_eq!(
        got,
        vec!["sess_A".to_string(), "sess_B".to_string()],
        "run_tick must call run::resume once per reclaimed session",
    );
    assert!(
        waker.wake_failed_calls.lock().unwrap().is_empty(),
        "no wake-failed events on the happy path",
    );
}

#[tokio::test]
async fn run_tick_writes_wake_failed_when_resume_errors() {
    let bus = InMemoryStateBus::new();
    let mut r = pending_row("sess_T", "tc-t", "shell::exec", json!({}));
    r.expires_at = 1;
    seed_pending(&bus, &r).await;

    let waker = common::FakeWaker::default();
    *waker.resume_response.lock().unwrap() = Some(Err("engine down".to_string()));

    approval_gate::run_tick(&waker, &bus, STATE_SCOPE, json!({})).await;

    assert_eq!(
        *waker.resume_calls.lock().unwrap(),
        vec!["sess_T".to_string()],
        "run_tick must still attempt the wake even though it'll fail",
    );
    assert_eq!(
        *waker.wake_failed_calls.lock().unwrap(),
        vec![("sess_T".to_string(), "engine down".to_string())],
        "wake failures must surface as approval_wake_failed events so the \
         operator doesn't watch a silent stall",
    );
}

#[tokio::test]
async fn run_tick_skips_resume_when_no_rows_expired() {
    let bus = InMemoryStateBus::new();
    // Pending with a deadline well in the future.
    let mut r = pending_row("sess_F", "tc-f", "shell::exec", json!({}));
    r.expires_at = now_ms() + 60_000;
    seed_pending(&bus, &r).await;

    let waker = common::FakeWaker::default();
    let tick = approval_gate::run_tick(&waker, &bus, STATE_SCOPE, json!({})).await;
    assert_eq!(tick["reclaimed"], 0);
    assert!(
        waker.resume_calls.lock().unwrap().is_empty(),
        "no expired rows ⇒ no wake calls",
    );
}

// ────────────────────────────────────────────────────────────────────────
// FINDING #5 — A Done-write failure must NOT orphan a row in `InFlight`
// forever after the side effect already ran.
//
// Expected behaviour (any of these is acceptable; the test pins the
// observable invariant):
//   - the handler retries the Done write enough times that the row
//     reaches Done before returning, OR
//   - the lazy-timeout flip is extended to reclaim sufficiently-stale
//     InFlight rows so a subsequent consume surfaces them as Failed /
//     TimedOut, OR
//   - some other reclamation surface clears the orphan.
//
// In all cases: after a bounded grace period, the row must not be
// observable as `InFlight` with no outcome.
// ────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn done_write_failure_must_not_orphan_in_flight_row() {
    struct FailOnSecondSetBus {
        inner: InMemoryStateBus,
        sets: Mutex<u32>,
    }
    #[async_trait::async_trait]
    impl StateBus for FailOnSecondSetBus {
        async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), iii_sdk::IIIError> {
            let count = {
                let mut n = self.sets.lock().unwrap();
                *n += 1;
                *n
            };
            if count >= 2 {
                return Err(iii_sdk::IIIError::Runtime("kv down on done-write".into()));
            }
            self.inner.set(scope, key, value).await
        }
        async fn get(&self, scope: &str, key: &str) -> Option<Value> {
            self.inner.get(scope, key).await
        }
        async fn list_prefix(&self, scope: &str, prefix: &str) -> Vec<Value> {
            self.inner.list_prefix(scope, prefix).await
        }
        async fn delete(&self, scope: &str, key: &str) -> Result<bool, iii_sdk::IIIError> {
            self.inner.delete(scope, key).await
        }
    }

    let bus = FailOnSecondSetBus {
        inner: InMemoryStateBus::new(),
        sets: Mutex::new(0),
    };
    let exec = FakeExecutor::default();
    let policy = RwLock::new(ask_all_rules());

    let row = pending_row("sess_F", "tc-f", "shell::exec", json!({}));
    bus.inner
        .set(STATE_SCOPE, "sess_F/tc-f", row.to_value())
        .await
        .unwrap();

    let _ = handle_resolve(
        &bus,
        &exec,
        STATE_SCOPE,
        &policy,
        json!({
            "session_id": "sess_F",
            "function_call_id": "tc-f",
            "decision": "allow",
        }),
        now_ms(),
    )
    .await;

    // Side effect did happen (the executor was called).
    assert_eq!(exec.calls.lock().unwrap().len(), 1);

    // The row may briefly remain in InFlight (the side effect already
    // ran and the Done write failed), but it MUST be reclaimable by an
    // external watchdog so the orphan window is bounded. The lazy flip
    // — extended to handle stale InFlight rows after a grace period —
    // is the reclaim mechanism.
    let raw = bus.inner.get(STATE_SCOPE, "sess_F/tc-f").await.unwrap();
    let after = Record::from_value(raw).unwrap();

    let reclaimed = after.flipped_to_timed_out_if_expired(u64::MAX);
    assert!(
        reclaimed.is_some(),
        "lazy-flip must reclaim stale InFlight rows once enough time has \
         passed so executor-side failures cannot leave the row orphaned",
    );
    let reclaimed = reclaimed.unwrap();
    assert_eq!(reclaimed.status, Status::Done);
    assert!(
        matches!(reclaimed.outcome, Some(Outcome::TimedOut)),
        "reclaimed row should carry a terminal outcome the orchestrator \
         can stitch into the LLM history",
    );
}

// ────────────────────────────────────────────────────────────────────────
// FINDING #6 — Concurrent `approval::consume` must return each Done row
// at most once across all callers.
// ────────────────────────────────────────────────────────────────────────
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_consume_must_not_return_duplicate_rows() {
    use tokio::sync::Barrier;

    // Simulate a non-atomic delete surface: even after the row is gone,
    // a state backend can report `existed=true` to two concurrent
    // callers if delete is implemented as get-then-delete. The
    // per-session mutex in `handle_consume` MUST close this gap on its
    // own, without leaning on `delete`'s return value.
    struct RacyDeleteBus {
        inner: InMemoryStateBus,
    }
    #[async_trait::async_trait]
    impl StateBus for RacyDeleteBus {
        async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), iii_sdk::IIIError> {
            self.inner.set(scope, key, value).await
        }
        async fn get(&self, scope: &str, key: &str) -> Option<Value> {
            self.inner.get(scope, key).await
        }
        async fn list_prefix(&self, scope: &str, prefix: &str) -> Vec<Value> {
            self.inner.list_prefix(scope, prefix).await
        }
        async fn delete(&self, scope: &str, key: &str) -> Result<bool, iii_sdk::IIIError> {
            // Always claim the row existed, even if it didn't. Models the
            // iii backend's lack of an atomic delete-if-present primitive.
            let _ = self.inner.delete(scope, key).await;
            Ok(true)
        }
    }

    let bus = Arc::new(RacyDeleteBus {
        inner: InMemoryStateBus::new(),
    });

    let done = pending_row("sess_D", "tc-d", "shell::exec", json!({}))
        .in_flight(now_ms())
        .done(Outcome::Executed {
            result: json!({"stdout": "ok"}),
        });
    bus.inner
        .set(STATE_SCOPE, "sess_D/tc-d", done.to_value())
        .await
        .unwrap();

    // Start barrier so both consumes hit handle_consume at the same
    // instant — exactly the race the per-session mutex must serialize.
    let start = Arc::new(Barrier::new(2));
    let (bus1, bus2) = (bus.clone(), bus.clone());
    let (s1, s2) = (start.clone(), start.clone());
    let t1 = tokio::spawn(async move {
        s1.wait().await;
        handle_consume(
            bus1.as_ref(),
            STATE_SCOPE,
            json!({ "session_id": "sess_D" }),
            now_ms(),
        )
        .await
    });
    let t2 = tokio::spawn(async move {
        s2.wait().await;
        handle_consume(
            bus2.as_ref(),
            STATE_SCOPE,
            json!({ "session_id": "sess_D" }),
            now_ms(),
        )
        .await
    });
    let (r1, r2) = tokio::join!(t1, t2);
    let r1 = r1.unwrap();
    let r2 = r2.unwrap();

    let n1 = r1["entries"].as_array().unwrap().len();
    let n2 = r2["entries"].as_array().unwrap().len();
    assert_eq!(
        n1 + n2,
        1,
        "the same Done row must only be drained once across concurrent \
         consumes (got {n1} + {n2})",
    );
}

// ────────────────────────────────────────────────────────────────────────
// FINDING #7 — The approved execution path must augment args the same
// way the normal `function_execute` path does.
//
// `turn-orchestrator::states::functions::handle_execute` enriches the
// invocation payload with `session_id`, `function_call_id`,
// `function_id`, and a nested `function_call` object. Approved calls
// must reach the target with the same shape so target handlers behave
// identically on both paths.
// ────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn approved_invoke_must_augment_args_like_normal_path() {
    let bus = InMemoryStateBus::new();
    let exec = FakeExecutor::default();
    let policy = RwLock::new(ask_all_rules());

    let original_args = json!({ "command": "git", "args": ["status"] });
    let row = pending_row("sess_A", "tc-1", "shell::exec", original_args.clone());
    seed_pending(&bus, &row).await;

    let _ = handle_resolve(
        &bus,
        &exec,
        STATE_SCOPE,
        &policy,
        json!({
            "session_id": "sess_A",
            "function_call_id": "tc-1",
            "decision": "allow",
        }),
        now_ms(),
    )
    .await;

    let calls = exec.calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "approved call invoked once");
    let (fn_id, args, _call_id_oob, _sess_oob) = &calls[0];
    assert_eq!(fn_id, "shell::exec");

    // The args payload itself must carry the augmentation keys, matching
    // the normal path in turn-orchestrator/src/states/functions.rs.
    assert_eq!(
        args.get("session_id").and_then(Value::as_str),
        Some("sess_A"),
        "approved invoke must augment args.session_id",
    );
    assert_eq!(
        args.get("function_call_id").and_then(Value::as_str),
        Some("tc-1"),
        "approved invoke must augment args.function_call_id",
    );
    assert_eq!(
        args.get("function_id").and_then(Value::as_str),
        Some("shell::exec"),
        "approved invoke must augment args.function_id",
    );
    let nested = args
        .get("function_call")
        .expect("approved invoke must augment args.function_call");
    assert_eq!(nested.get("id").and_then(Value::as_str), Some("tc-1"));
    assert_eq!(
        nested.get("function_id").and_then(Value::as_str),
        Some("shell::exec"),
    );
    assert_eq!(nested.get("arguments"), Some(&original_args));
}

// ────────────────────────────────────────────────────────────────────────
// FINDING — `approval_resolved` event MUST derive `decision` from
// `outcome.kind`, not `Record.status`.
//
// Reproducer for the validator's P2 finding: the previous
// `emit_approval_resolved` read `final_rec.status` (always `"done"` for
// terminal records) and matched it against `"executed"|"approved"`, so
// every resolved approval emitted `decision: "deny"` — the cascade fix
// just made the wrong payload happen N+1 times instead of once.
//
// Pin the corrected mapping via the pure helper so the bug can't return
// without a test breaking:
//   - outcome.kind == "executed"|"failed" → decision == "allow"
//   - denied/timed_out                    → decision == "deny"
// Also pin the wire shape: drop the redundant flat status/result/
// error/denial fields in favor of a single nested `outcome`.
// ────────────────────────────────────────────────────────────────────────

#[test]
fn approval_resolved_event_decision_is_allow_only_for_executed_outcome() {
    let rec = pending_row("sess_A", "tc-1", "shell::exec", json!({}))
        .in_flight(now_ms())
        .done(Outcome::Executed {
            result: json!({"stdout": "ok"}),
        });
    let evt = approval_gate::build_approval_resolved_event("tc-1", &rec.to_value());
    assert_eq!(
        evt["decision"], "allow",
        "outcome.kind=executed must map to decision=allow",
    );
    assert_eq!(evt["function_call_id"], "tc-1");
    assert_eq!(evt["tool_call_id"], "tc-1");
    assert_eq!(
        evt["outcome"]["kind"], "executed",
        "full outcome must be carried for downstream consumers",
    );
    assert_eq!(
        evt["outcome"]["detail"]["result"]["stdout"], "ok",
        "outcome.detail must carry the full executor result",
    );
    // Wire shape pin: redundant flat fields are gone.
    assert!(
        evt.get("status").is_none(),
        "flat `status` field must be removed (use outcome.kind)",
    );
    assert!(
        evt.get("result").is_none(),
        "flat `result` field must be removed (use outcome.detail.result)",
    );
}

#[test]
fn approval_resolved_event_decision_is_allow_for_failed_outcome() {
    let rec = pending_row("sess_A", "tc-2", "shell::exec", json!({}))
        .in_flight(now_ms())
        .done(Outcome::Failed {
            error: "spawn failed".into(),
        });
    let evt = approval_gate::build_approval_resolved_event("tc-2", &rec.to_value());
    assert_eq!(
        evt["decision"], "allow",
        "outcome.kind=failed means the operator allowed execution; \
         outcome.kind carries the target failure",
    );
    assert_eq!(evt["outcome"]["kind"], "failed");
    assert_eq!(evt["outcome"]["detail"]["error"], "spawn failed");
}

#[test]
fn approval_resolved_event_decision_is_deny_for_denied_outcome() {
    let rec = pending_row("sess_A", "tc-3", "shell::exec", json!({})).done_at(
        now_ms(),
        Outcome::Denied {
            denial: approval_gate::Denial::UserRejected,
        },
    );
    let evt = approval_gate::build_approval_resolved_event("tc-3", &rec.to_value());
    assert_eq!(evt["decision"], "deny");
    assert_eq!(evt["outcome"]["kind"], "denied");
    assert_eq!(evt["outcome"]["detail"]["denial"]["kind"], "user_rejected");
}

#[test]
fn approval_resolved_event_decision_is_deny_for_timed_out_outcome() {
    let rec = pending_row("sess_A", "tc-4", "shell::exec", json!({}))
        .done_at(now_ms(), Outcome::TimedOut);
    let evt = approval_gate::build_approval_resolved_event("tc-4", &rec.to_value());
    assert_eq!(evt["decision"], "deny");
    assert_eq!(evt["outcome"]["kind"], "timed_out");
}

/// Regression for the previous bug: if `outcome` is missing entirely
/// (shouldn't happen for terminal records, but defensive) decision must
/// be `"deny"` — never silently emit `"allow"`.
#[test]
fn approval_resolved_event_defaults_decision_to_deny_when_outcome_missing() {
    let evt = approval_gate::build_approval_resolved_event("tc-5", &json!({ "status": "done" }));
    assert_eq!(
        evt["decision"], "deny",
        "no outcome ⇒ deny (safety default); allow must require outcome.kind=executed",
    );
    assert!(evt.get("outcome").is_none());
}
