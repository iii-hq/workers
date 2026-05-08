//! Per-browser subscription registry for the harness UI.
//!
//! Browsers register interest in particular sessions (or all sessions) via
//! `ui::subscribe` / `ui::unsubscribe`. The fanout keeps an in-memory map
//! of `BrowserId -> HashSet<SessionId>` (None = "all sessions, non-session
//! topics like cost/workers/approvals").
//!
//! Two upstream pumps live here:
//!
//! 1. **agent::events stream subscriber** — registers a `stream` trigger
//!    against `agent::events`. On every frame, the engine invokes our handler
//!    with `{groupId, event: {data}, ...}`; we extract the session_id, look
//!    up subscribed browsers, and call
//!    `ui::session::event::<browser_id>` for each (fire-and-forget).
//!
//! 2. **sessions changed poll** — every second, queries `state::list` for
//!    `scope=agent prefix=session/`. Diffs against the prior snapshot and
//!    pushes `ui::sessions::changed::<browser_id>` to every all-sessions
//!    subscriber when the membership changes.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use iii_sdk::{
    FunctionRef, IIIError, RegisterFunctionMessage, RegisterTriggerInput, Trigger, TriggerRequest,
    III,
};
use serde_json::{json, Value};

/// Identity of a connected browser worker. Caller-supplied; we don't mint it.
pub type BrowserId = String;

/// True if `e` is the engine's "no worker has registered this function" error.
/// We match both the structured `Remote { code: "function_not_found", .. }`
/// shape and the `Display` form, since some SDK paths surface it as a flat
/// runtime/handler error string.
pub(crate) fn is_function_not_found(e: &IIIError) -> bool {
    match e {
        IIIError::Remote { code, .. } => code == "function_not_found",
        IIIError::Runtime(s) | IIIError::Handler(s) => s.contains("function_not_found"),
        _ => false,
    }
}

/// `None` means "subscribe to all sessions / non-session topics".
pub type Subscription = Option<String>;

/// Per-browser outbound budget. Tracks in-flight `ui::*` pushes and the
/// last-emitted cost-tick instant. We use atomics so push paths can decrement
/// without acquiring the fanout write lock.
#[derive(Debug)]
pub struct BrowserOutbound {
    in_flight: AtomicU64,
    /// `None` until the first cost tick. Stored as `Mutex<Option<Instant>>`-
    /// equivalent — we only mutate from one place (`maybe_emit_cost_tick`)
    /// under the fanout write lock.
    last_cost_tick: std::sync::Mutex<Option<Instant>>,
    /// True if a `ui::session::resync` is already pending for this browser
    /// after an overflow. Prevents a flood of resyncs while the browser is
    /// still consuming the old queue.
    resync_pending: std::sync::atomic::AtomicBool,
}

impl Default for BrowserOutbound {
    fn default() -> Self {
        Self {
            in_flight: AtomicU64::new(0),
            last_cost_tick: std::sync::Mutex::new(None),
            resync_pending: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

#[derive(Debug, Default)]
pub struct FanoutState {
    /// browser_id -> set of subscribed session ids ("__all__" sentinel = all-sessions)
    pub subs: HashMap<BrowserId, HashSet<String>>,
    /// browser_id -> backpressure / coalescing bookkeeping. Keyed identically
    /// to `subs`; entries are inserted lazily on first push.
    pub outbound: HashMap<BrowserId, Arc<BrowserOutbound>>,
}

const ALL_SESSIONS_SENTINEL: &str = "__all__";

/// How long to wait for a `ui::*` push to a browser before giving up. Browsers
/// shouldn't take long to ack a fire-and-forget trigger; if they do, we'd
/// rather drop the frame than back the pump up.
const PUSH_TIMEOUT_MS: u64 = 2_000;

/// How long to wait for a `state::list` snapshot. If the call is slower than
/// this we skip the tick — the next one will catch up.
const STATE_LIST_TIMEOUT_MS: u64 = 5_000;

/// Sessions-changed poll cadence. Cheap because `state::list` is in-memory in
/// the engine's default state worker; the wire round-trip dominates.
const SESSIONS_POLL_INTERVAL_MS: u64 = 1_000;

/// Approval poll cadence. Hook-driven push (via the agent::events stream
/// pump) covers low-latency notification; this poll catches missed states
/// and clears resolved approvals.
const APPROVAL_POLL_INTERVAL_MS: u64 = 1_000;

/// Cost summary poll cadence. Each tick performs a `budget::list` and (for
/// changed budgets) a `budget::usage` round-trip. 2s is cheap and matches
/// the design coalescing target.
const COST_POLL_INTERVAL_MS: u64 = 2_000;

/// Workers/status poll cadence. Diff-only pushes mean a no-op worker pool
/// generates zero UI traffic.
const WORKERS_POLL_INTERVAL_MS: u64 = 5_000;

/// Per-browser cap on `ui::*` pushes. When the in-flight outbound count
/// exceeds this, we drop the oldest queued push, send a single
/// `ui::session::resync` so the browser re-fetches baseline, and resume.
pub const PER_BROWSER_QUEUE_CAP: usize = 1024;

/// Hard ceiling on `ui::cost::tick` pushes per browser per second. The poll
/// runs every 2s upstream, but a hook-driven stream of cost updates could
/// otherwise burst — we keep the steady-state ≤10/s per design.
const COST_TICK_MIN_INTERVAL_MS: u128 = 100;

impl FanoutState {
    pub fn subscribe(&mut self, browser: BrowserId, session: Subscription) {
        let key = session.unwrap_or_else(|| ALL_SESSIONS_SENTINEL.into());
        self.subs.entry(browser).or_default().insert(key);
    }

    pub fn unsubscribe(&mut self, browser: &str, session: Subscription) {
        let key = session.unwrap_or_else(|| ALL_SESSIONS_SENTINEL.into());
        if let Some(set) = self.subs.get_mut(browser) {
            set.remove(&key);
            if set.is_empty() {
                self.subs.remove(browser);
                self.outbound.remove(browser);
            }
        }
    }

    /// Drop a browser entirely (all sessions, all outbound budget). Used when
    /// the browser's per-browser handler `ui::session::event::<id>` no longer
    /// exists on the engine — the browser closed without calling
    /// `ui::unsubscribe`. Returns `true` if anything was evicted.
    pub fn evict_browser(&mut self, browser: &str) -> bool {
        let removed = self.subs.remove(browser).is_some();
        self.outbound.remove(browser);
        removed
    }

    /// Get-or-insert the per-browser outbound budget. Used by every push
    /// path to gate inflight + coalesce cost ticks.
    pub fn outbound_for(&mut self, browser: &str) -> Arc<BrowserOutbound> {
        if let Some(b) = self.outbound.get(browser) {
            return Arc::clone(b);
        }
        let b = Arc::new(BrowserOutbound::default());
        self.outbound.insert(browser.to_string(), Arc::clone(&b));
        b
    }

    /// Browsers interested in a specific session (or in all sessions).
    pub fn subscribers_for(&self, session_id: &str) -> Vec<BrowserId> {
        self.subs
            .iter()
            .filter(|(_, set)| set.contains(session_id) || set.contains(ALL_SESSIONS_SENTINEL))
            .map(|(b, _)| b.clone())
            .collect()
    }

    /// Browsers subscribed to all-sessions topics (cost, workers, approvals).
    pub fn all_sessions_subscribers(&self) -> Vec<BrowserId> {
        self.subs
            .iter()
            .filter(|(_, set)| set.contains(ALL_SESSIONS_SENTINEL))
            .map(|(b, _)| b.clone())
            .collect()
    }

    /// Total connected browser count (any subscription).
    pub fn browser_count(&self) -> usize {
        self.subs.len()
    }
}

pub type SharedFanout = Arc<RwLock<FanoutState>>;

pub fn new_shared() -> SharedFanout {
    Arc::new(RwLock::new(FanoutState::default()))
}

/// Handles for the upstream pumps. Drop ends them.
pub struct FanoutPumps {
    pub agent_event_fn: FunctionRef,
    pub agent_event_trigger: Option<Trigger>,
    pub sessions_poll: tokio::task::JoinHandle<()>,
    pub approval_poll: tokio::task::JoinHandle<()>,
    pub cost_poll: tokio::task::JoinHandle<()>,
    pub workers_poll: tokio::task::JoinHandle<()>,
}

impl FanoutPumps {
    pub fn shutdown(self) {
        if let Some(t) = self.agent_event_trigger {
            t.unregister();
        }
        self.agent_event_fn.unregister();
        self.sessions_poll.abort();
        self.approval_poll.abort();
        self.cost_poll.abort();
        self.workers_poll.abort();
    }
}

/// Spin up the agent::events stream subscriber and the sessions-changed poll.
///
/// Must be called once at boot, after the harness has registered its UI
/// functions. Returns handles whose `shutdown()` ends both pumps.
pub fn spawn_subscribers(iii: &Arc<III>, fanout: SharedFanout) -> FanoutPumps {
    let agent_event_fn = register_agent_event_pump(iii.as_ref(), Arc::clone(&fanout));
    let agent_event_trigger = match iii.register_trigger(RegisterTriggerInput {
        trigger_type: "stream".into(),
        function_id: agent_event_fn.id.clone(),
        config: json!({ "stream_name": "agent::events" }),
        metadata: None,
    }) {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::warn!(error = %e, "harness fanout: failed to register agent::events stream trigger");
            None
        }
    };

    let sessions_poll = spawn_sessions_changed_poll(Arc::clone(iii), Arc::clone(&fanout));
    let approval_poll = spawn_approval_poll(Arc::clone(iii), Arc::clone(&fanout));
    let cost_poll = spawn_cost_poll(Arc::clone(iii), Arc::clone(&fanout));
    let workers_poll = spawn_workers_poll(Arc::clone(iii), fanout);

    FanoutPumps {
        agent_event_fn,
        agent_event_trigger,
        sessions_poll,
        approval_poll,
        cost_poll,
        workers_poll,
    }
}

fn register_agent_event_pump(iii: &III, fanout: SharedFanout) -> FunctionRef {
    let id = format!("harness::ui::agent-events-pump-{}", std::process::id());
    let iii_inner = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(id).with_description(
            "Internal: forwards agent::events stream frames to UI subscribers.".into(),
        ),
        move |payload: Value| {
            let iii = iii_inner.clone();
            let fanout = Arc::clone(&fanout);
            async move {
                if let Some((session_id, event_data)) = extract_event_payload(&payload) {
                    let browsers = {
                        let state = fanout.read().await;
                        state.subscribers_for(&session_id)
                    };
                    let frame = json!({
                        "session_id": session_id,
                        "event": event_data,
                    });
                    for browser_id in browsers {
                        let function_id = format!("ui::session::event::{browser_id}");
                        let frame = frame.clone();
                        let iii_for_push = iii.clone();
                        let fanout_for_gc = Arc::clone(&fanout);
                        let browser_for_gc = browser_id.clone();
                        // Fire-and-forget. The browser is allowed to be slow
                        // or absent; we don't want one stale browser to
                        // back up the whole pump. If the per-browser handler
                        // is gone (browser closed without `ui::unsubscribe`),
                        // garbage-collect its subscription so the engine
                        // stops logging `function_not_found` on every event.
                        tokio::spawn(async move {
                            if let Err(e) = iii_for_push
                                .trigger(TriggerRequest {
                                    function_id,
                                    payload: frame,
                                    action: None,
                                    timeout_ms: Some(PUSH_TIMEOUT_MS),
                                })
                                .await
                            {
                                if is_function_not_found(&e) {
                                    let evicted = {
                                        let mut state = fanout_for_gc.write().await;
                                        state.evict_browser(&browser_for_gc)
                                    };
                                    if evicted {
                                        tracing::debug!(
                                            browser_id = %browser_for_gc,
                                            "evicted stale browser subscription (handler gone)"
                                        );
                                    }
                                } else {
                                    tracing::trace!(error = %e, "ui push failed (browser likely slow)");
                                }
                            }
                        });
                    }
                }
                Ok::<_, IIIError>(json!({ "ok": true }))
            }
        },
    ))
}

/// Pull (group_id, event_data) out of the engine's stream-trigger envelope.
/// Accepts both the current camelCase nested shape and older snake_case flat
/// shape — same compatibility window the ACP fan-in uses.
fn extract_event_payload(payload: &Value) -> Option<(String, Value)> {
    let session_id = payload
        .get("groupId")
        .or_else(|| payload.get("group_id"))
        .and_then(|v| v.as_str())?
        .to_string();
    let data = payload
        .get("event")
        .and_then(|e| e.get("data"))
        .cloned()
        .or_else(|| payload.get("data").cloned())
        .unwrap_or(Value::Null);
    Some((session_id, data))
}

fn spawn_sessions_changed_poll(iii: Arc<III>, fanout: SharedFanout) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut prev: HashSet<String> = HashSet::new();
        let mut interval =
            tokio::time::interval(std::time::Duration::from_millis(SESSIONS_POLL_INTERVAL_MS));
        // The first tick fires immediately; skip it to avoid a pointless
        // empty-vs-empty diff before any browser has connected.
        interval.tick().await;

        loop {
            interval.tick().await;

            let result = iii
                .trigger(TriggerRequest {
                    function_id: "state::list".into(),
                    payload: json!({ "scope": "agent", "prefix": "session/" }),
                    action: None,
                    timeout_ms: Some(STATE_LIST_TIMEOUT_MS),
                })
                .await;

            let current = match result {
                Ok(v) => extract_session_ids(&v),
                Err(_) => continue, // transient — try again next tick
            };

            if current == prev {
                continue;
            }

            let added: Vec<String> = current.difference(&prev).cloned().collect();
            let removed: Vec<String> = prev.difference(&current).cloned().collect();

            let browsers = {
                let state = fanout.read().await;
                state.all_sessions_subscribers()
            };

            for browser_id in browsers {
                let function_id = format!("ui::sessions::changed::{browser_id}");
                let payload = json!({
                    "added": added,
                    "removed": removed,
                    "total": current.len(),
                });
                let iii_for_push = iii.clone();
                tokio::spawn(async move {
                    if let Err(e) = iii_for_push
                        .trigger(TriggerRequest {
                            function_id,
                            payload,
                            action: None,
                            timeout_ms: Some(PUSH_TIMEOUT_MS),
                        })
                        .await
                    {
                        tracing::trace!(error = %e, "sessions::changed push failed");
                    }
                });
            }

            prev = current;
        }
    })
}

/// Outcome of a backpressure-gated push attempt. Tests assert against this.
#[derive(Debug, PartialEq, Eq)]
pub enum PushOutcome {
    /// Push was sent (or spawned). Caller should expect a normal delivery.
    Sent,
    /// Per-browser queue was over the cap; we dropped the oldest in-flight
    /// push and emitted a single `ui::session::resync` instead.
    DroppedAndResynced,
    /// Cost-tick coalesce gate is still warm; push was suppressed.
    CoalescedSkipped,
}

/// Why a push is being made — affects whether the cost-tick coalesce gate
/// applies. Approval/workers/sessions pushes are unaffected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushKind {
    /// Standard fire-and-forget. Counts against the queue cap.
    Standard,
    /// `ui::cost::tick` — additionally subject to the
    /// `COST_TICK_MIN_INTERVAL_MS` coalesce gate.
    CostTick,
}

/// Decide whether a cost-tick should fire now given the prior emission instant.
/// Pure helper so the coalesce policy can be unit-tested without spawning tasks.
pub(crate) fn should_emit_cost_tick(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|prev| now.duration_since(prev).as_millis() >= COST_TICK_MIN_INTERVAL_MS)
}

/// Emit a UI push to a single browser, honoring per-browser queue cap and
/// the cost-tick coalesce gate. Spawns the actual `iii.trigger` and returns
/// immediately. Synchronous part is fast (one HashMap lookup + atomics).
fn push_to_browser(
    iii: &Arc<III>,
    fanout: &SharedFanout,
    browser_id: &str,
    function_id: String,
    payload: Value,
    kind: PushKind,
) -> tokio::task::JoinHandle<PushOutcome> {
    let iii = Arc::clone(iii);
    let fanout = Arc::clone(fanout);
    let browser_id = browser_id.to_string();
    tokio::spawn(async move {
        let outbound = {
            let mut state = fanout.write().await;
            state.outbound_for(&browser_id)
        };

        // Cost-tick coalesce gate.
        if matches!(kind, PushKind::CostTick) {
            let mut last = outbound.last_cost_tick.lock().expect("cost-tick mutex");
            let now = Instant::now();
            if !should_emit_cost_tick(*last, now) {
                return PushOutcome::CoalescedSkipped;
            }
            *last = Some(now);
        }

        // Queue cap.
        let in_flight = outbound.in_flight.fetch_add(1, Ordering::SeqCst);
        if usize::try_from(in_flight).unwrap_or(usize::MAX) >= PER_BROWSER_QUEUE_CAP {
            // Roll back the increment we made above; we are NOT going to
            // send this frame.
            outbound.in_flight.fetch_sub(1, Ordering::SeqCst);

            // Emit a single resync (deduped) and bail out.
            let already = outbound.resync_pending.swap(true, Ordering::SeqCst);
            if !already {
                let resync_id = format!("ui::session::resync::{browser_id}");
                let outbound_for_resync = Arc::clone(&outbound);
                let iii_for_resync = Arc::clone(&iii);
                tokio::spawn(async move {
                    let _ = iii_for_resync
                        .trigger(TriggerRequest {
                            function_id: resync_id,
                            payload: json!({ "reason": "queue_overflow" }),
                            action: None,
                            timeout_ms: Some(PUSH_TIMEOUT_MS),
                        })
                        .await;
                    outbound_for_resync
                        .resync_pending
                        .store(false, Ordering::SeqCst);
                });
            }
            return PushOutcome::DroppedAndResynced;
        }

        // Fire-and-forget the actual push, decrementing in_flight when it
        // resolves so the cap reflects real concurrency.
        let outbound_for_push = Arc::clone(&outbound);
        let iii_for_push = Arc::clone(&iii);
        tokio::spawn(async move {
            let res = iii_for_push
                .trigger(TriggerRequest {
                    function_id,
                    payload,
                    action: None,
                    timeout_ms: Some(PUSH_TIMEOUT_MS),
                })
                .await;
            outbound_for_push.in_flight.fetch_sub(1, Ordering::SeqCst);
            if let Err(e) = res {
                tracing::trace!(error = %e, "ui push failed (browser likely gone)");
            }
        });
        PushOutcome::Sent
    })
}

/// Pure diff helper for the workers poll. Returns `Some(payload)` when the
/// snapshot meaningfully differs from the previous and we should push, or
/// `None` when nothing changed.
pub(crate) fn diff_workers(
    prev: &BTreeMap<String, String>,
    next: &BTreeMap<String, String>,
    expected: &[&str],
) -> Option<Value> {
    if prev == next {
        return None;
    }
    let mut workers: Vec<Value> = expected
        .iter()
        .map(|name| {
            let status = next.get(*name).cloned().unwrap_or_else(|| "down".into());
            json!({ "name": name, "status": status })
        })
        .collect();
    // Stable ordering for deterministic UI diffs.
    workers.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["name"].as_str().unwrap_or(""))
    });
    let total = expected.len();
    let up = next.values().filter(|s| s.as_str() == "up").count();
    let down = total.saturating_sub(up);
    Some(json!({
        "up": up,
        "down": down,
        "total": total,
        "workers": workers,
    }))
}

/// Pure diff helper for the approval pump. Returns the IDs that newly
/// appeared in `next` and the ones that were removed since `prev`.
pub(crate) fn diff_approvals(
    prev: &HashMap<String, Value>,
    next: &HashMap<String, Value>,
) -> (Vec<(String, Value)>, Vec<String>) {
    let mut requested: Vec<(String, Value)> = next
        .iter()
        .filter(|(k, _)| !prev.contains_key(*k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    requested.sort_by(|a, b| a.0.cmp(&b.0));
    let mut resolved: Vec<String> = prev
        .keys()
        .filter(|k| !next.contains_key(*k))
        .cloned()
        .collect();
    resolved.sort();
    (requested, resolved)
}

/// Spawn the approval pump. Polls `approval::list_pending` for every known
/// session every `APPROVAL_POLL_INTERVAL_MS`. On change, pushes
/// `ui::approval::requested` (per new entry) and `ui::approval::resolved`
/// (per removed entry) to all-sessions subscribers.
fn spawn_approval_poll(iii: Arc<III>, fanout: SharedFanout) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut prev: HashMap<String, Value> = HashMap::new();
        let mut interval = tokio::time::interval(Duration::from_millis(APPROVAL_POLL_INTERVAL_MS));
        interval.tick().await;
        loop {
            interval.tick().await;

            // Discover known sessions; without them we have nowhere to ask.
            let session_ids = match iii
                .trigger(TriggerRequest {
                    function_id: "state::list".into(),
                    payload: json!({ "scope": "agent", "prefix": "session/" }),
                    action: None,
                    timeout_ms: Some(STATE_LIST_TIMEOUT_MS),
                })
                .await
            {
                Ok(v) => extract_session_ids(&v),
                Err(_) => continue,
            };

            let mut next: HashMap<String, Value> = HashMap::new();
            for sid in &session_ids {
                let resp = iii
                    .trigger(TriggerRequest {
                        function_id: "approval::list_pending".into(),
                        payload: json!({ "session_id": sid }),
                        action: None,
                        timeout_ms: Some(STATE_LIST_TIMEOUT_MS),
                    })
                    .await;
                let Ok(resp) = resp else { continue };
                let Some(arr) = resp.get("pending").and_then(|v| v.as_array()) else {
                    continue;
                };
                for entry in arr {
                    let Some(id) = entry
                        .get("function_call_id")
                        .or_else(|| entry.get("tool_call_id"))
                        .and_then(|v| v.as_str())
                    else {
                        continue;
                    };
                    // Annotate with session_id so the UI can group/filter.
                    let mut enriched = entry.clone();
                    if let Some(obj) = enriched.as_object_mut() {
                        obj.insert("session_id".into(), Value::String(sid.clone()));
                    }
                    next.insert(id.to_string(), enriched);
                }
            }

            let (requested, resolved) = diff_approvals(&prev, &next);
            if requested.is_empty() && resolved.is_empty() {
                continue;
            }

            let browsers = {
                let state = fanout.read().await;
                state.all_sessions_subscribers()
            };

            for browser_id in &browsers {
                for (_id, payload) in &requested {
                    push_to_browser(
                        &iii,
                        &fanout,
                        browser_id,
                        format!("ui::approval::requested::{browser_id}"),
                        payload.clone(),
                        PushKind::Standard,
                    );
                }
                for id in &resolved {
                    push_to_browser(
                        &iii,
                        &fanout,
                        browser_id,
                        format!("ui::approval::resolved::{browser_id}"),
                        json!({ "function_call_id": id, "tool_call_id": id }),
                        PushKind::Standard,
                    );
                }
            }

            prev = next;
        }
    })
}

/// Spawn the cost poll. Calls `budget::list` every `COST_POLL_INTERVAL_MS`,
/// computes a {usd_today, by_provider} summary, and pushes
/// `ui::cost::tick` to all-sessions subscribers when totals change.
///
/// `llm-budget::summary` does not exist in the current llm-budget worker
/// (verified via `grep "with_id" llm-budget/src/register.rs`). We synthesize
/// the summary client-side from `budget::list`, which returns the full
/// budget set including `spent_usd` per budget — the only thing we ship
/// today is the daily aggregate.
fn spawn_cost_poll(iii: Arc<III>, fanout: SharedFanout) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut prev_total: f64 = -1.0;
        let mut interval = tokio::time::interval(Duration::from_millis(COST_POLL_INTERVAL_MS));
        interval.tick().await;
        loop {
            interval.tick().await;

            let resp = iii
                .trigger(TriggerRequest {
                    function_id: "budget::list".into(),
                    payload: json!({}),
                    action: None,
                    timeout_ms: Some(STATE_LIST_TIMEOUT_MS),
                })
                .await;
            let Ok(resp) = resp else { continue };

            let summary = summarize_budgets(&resp);
            let total = summary
                .get("usd_today")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            // Compare with epsilon to avoid float drift loops.
            if (total - prev_total).abs() < 1e-6 && prev_total >= 0.0 {
                continue;
            }
            prev_total = total;

            let browsers = {
                let state = fanout.read().await;
                state.all_sessions_subscribers()
            };
            for browser_id in &browsers {
                push_to_browser(
                    &iii,
                    &fanout,
                    browser_id,
                    format!("ui::cost::tick::{browser_id}"),
                    summary.clone(),
                    PushKind::CostTick,
                );
            }
        }
    })
}

/// Reduce a `budget::list` response to a `ui::cost::tick` payload.
/// Pure helper so the shape can be unit-tested without a live engine.
pub(crate) fn summarize_budgets(resp: &Value) -> Value {
    let list = resp
        .get("budgets")
        .and_then(|v| v.as_array())
        .or_else(|| resp.as_array())
        .cloned()
        .unwrap_or_default();
    let mut total = 0.0_f64;
    let mut by_period: HashMap<String, f64> = HashMap::new();
    for b in &list {
        let spent = b.get("spent_usd").and_then(Value::as_f64).unwrap_or(0.0);
        total += spent;
        let period = b
            .get("period")
            .and_then(Value::as_str)
            .unwrap_or("daily")
            .to_string();
        *by_period.entry(period).or_insert(0.0) += spent;
    }
    json!({
        "usd_today": total,
        "by_provider": Value::Object(serde_json::Map::new()),
        "by_period": by_period,
        "budgets": list.len(),
    })
}

/// Spawn the workers poll. Calls `engine::workers::list` every
/// `WORKERS_POLL_INTERVAL_MS`, joins against `EXPECTED_WORKERS`, and pushes
/// `ui::workers::changed` on diff.
fn spawn_workers_poll(iii: Arc<III>, fanout: SharedFanout) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut prev: BTreeMap<String, String> = BTreeMap::new();
        let mut interval = tokio::time::interval(Duration::from_millis(WORKERS_POLL_INTERVAL_MS));
        interval.tick().await;
        loop {
            interval.tick().await;

            let resp = iii
                .trigger(TriggerRequest {
                    function_id: "engine::workers::list".into(),
                    payload: json!({}),
                    action: None,
                    timeout_ms: Some(STATE_LIST_TIMEOUT_MS),
                })
                .await;
            let Ok(resp) = resp else { continue };
            let next = extract_worker_status(&resp);

            let Some(payload) = diff_workers(&prev, &next, crate::EXPECTED_WORKERS) else {
                continue;
            };
            prev = next;

            let browsers = {
                let state = fanout.read().await;
                state.all_sessions_subscribers()
            };
            for browser_id in &browsers {
                push_to_browser(
                    &iii,
                    &fanout,
                    browser_id,
                    format!("ui::workers::changed::{browser_id}"),
                    payload.clone(),
                    PushKind::Standard,
                );
            }
        }
    })
}

/// Pull `{name -> status}` from an `engine::workers::list` response. The
/// engine returns a JSON array of worker descriptors with at least `name`
/// and either `status` or `state` fields. Unknown workers map to `"up"` if
/// the engine listed them at all (presence == liveness).
pub(crate) fn extract_worker_status(value: &Value) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let arr = value
        .get("workers")
        .and_then(|v| v.as_array())
        .or_else(|| value.as_array());
    let Some(arr) = arr else { return out };
    for item in arr {
        let Some(name) = item
            .get("name")
            .or_else(|| item.get("worker"))
            .or_else(|| item.get("id"))
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        let status = item
            .get("status")
            .or_else(|| item.get("state"))
            .and_then(|v| v.as_str())
            .unwrap_or("up")
            .to_string();
        out.insert(name.to_string(), status);
    }
    out
}

/// Walk a `state::list` response and pull out every `session_id` string.
///
/// `state::list` currently returns the bare `data` Values (no keys) from
/// `scope=agent prefix=session/`. The harness writes both
/// `session/<id>` (turn-orchestrator's `SessionState`) and
/// `session/<id>/workspace` / `session/<id>/messages` etc., so the only
/// reliable distinguisher is "has a `session_id` string field". This mirrors
/// the parsing in `harness/web/src/components/SessionList.tsx`'s
/// `fetchFromStateFallback`.
pub(crate) fn extract_session_ids(value: &Value) -> HashSet<String> {
    let Some(arr) = value.as_array() else {
        return HashSet::new();
    };
    let mut out = HashSet::new();
    for item in arr {
        if let Some(sid) = item.get("session_id").and_then(|v| v.as_str()) {
            out.insert(sid.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_inserts_and_routes_per_session() {
        let mut s = FanoutState::default();
        s.subscribe("browser-a".into(), Some("sess-1".into()));
        s.subscribe("browser-b".into(), Some("sess-2".into()));
        assert_eq!(s.subscribers_for("sess-1"), vec!["browser-a".to_string()]);
        assert_eq!(s.subscribers_for("sess-2"), vec!["browser-b".to_string()]);
        assert!(s.subscribers_for("sess-3").is_empty());
    }

    #[test]
    fn registry_routes_all_sessions_subscriber_to_any_session() {
        let mut s = FanoutState::default();
        s.subscribe("browser-a".into(), None);
        let subs = s.subscribers_for("any-session-id");
        assert_eq!(subs, vec!["browser-a".to_string()]);
    }

    #[test]
    fn unsubscribe_evicts_browser_when_last_sub_removed() {
        let mut s = FanoutState::default();
        s.subscribe("browser-a".into(), Some("sess-1".into()));
        s.unsubscribe("browser-a", Some("sess-1".into()));
        assert_eq!(s.browser_count(), 0);
    }

    #[test]
    fn evict_browser_drops_all_sessions_at_once() {
        let mut s = FanoutState::default();
        s.subscribe("browser-a".into(), Some("sess-1".into()));
        s.subscribe("browser-a".into(), Some("sess-2".into()));
        s.subscribe("browser-a".into(), None);
        // Touch outbound so the eviction path has something to clean up.
        let _ = s.outbound_for("browser-a");
        assert_eq!(s.browser_count(), 1);

        let removed = s.evict_browser("browser-a");
        assert!(removed, "evict must report success when the browser was present");
        assert_eq!(s.browser_count(), 0);
        assert!(
            s.subscribers_for("sess-1").is_empty(),
            "evicted browser must not appear in subscribers_for"
        );
    }

    #[test]
    fn evict_browser_is_noop_when_unknown() {
        let mut s = FanoutState::default();
        let removed = s.evict_browser("ghost");
        assert!(!removed);
        assert_eq!(s.browser_count(), 0);
    }

    #[test]
    fn is_function_not_found_matches_remote_code() {
        let e = IIIError::Remote {
            code: "function_not_found".into(),
            message: "Function not found".into(),
            stacktrace: None,
        };
        assert!(super::is_function_not_found(&e));
    }

    #[test]
    fn is_function_not_found_matches_runtime_string_form() {
        let e = IIIError::Runtime("function_not_found: ui::session::event::xyz".into());
        assert!(super::is_function_not_found(&e));
    }

    #[test]
    fn is_function_not_found_rejects_unrelated_errors() {
        assert!(!super::is_function_not_found(&IIIError::Timeout));
        assert!(!super::is_function_not_found(&IIIError::NotConnected));
        assert!(!super::is_function_not_found(&IIIError::Remote {
            code: "internal_error".into(),
            message: "boom".into(),
            stacktrace: None,
        }));
    }

    #[test]
    fn all_sessions_subscribers_returns_only_global_subs() {
        let mut s = FanoutState::default();
        s.subscribe("browser-a".into(), None);
        s.subscribe("browser-b".into(), Some("sess-1".into()));
        let g = s.all_sessions_subscribers();
        assert_eq!(g, vec!["browser-a".to_string()]);
    }

    #[test]
    fn extract_event_payload_handles_camelcase_envelope() {
        let env = json!({
            "type": "stream",
            "streamName": "agent::events",
            "groupId": "sess-1",
            "id": "sess-1-00000001",
            "event": { "type": "create", "data": { "type": "message_end" } },
        });
        let (sid, data) = extract_event_payload(&env).unwrap();
        assert_eq!(sid, "sess-1");
        assert_eq!(data, json!({ "type": "message_end" }));
    }

    #[test]
    fn extract_event_payload_handles_flat_snake_case() {
        let env = json!({
            "group_id": "sess-2",
            "data": { "type": "turn_start" },
        });
        let (sid, data) = extract_event_payload(&env).unwrap();
        assert_eq!(sid, "sess-2");
        assert_eq!(data, json!({ "type": "turn_start" }));
    }

    #[test]
    fn extract_event_payload_returns_none_when_no_session_id() {
        assert!(extract_event_payload(&json!({})).is_none());
    }

    #[test]
    fn extract_session_ids_pulls_session_id_strings() {
        let v = json!([
            { "session_id": "s1", "state": "stopped" },
            { "session_id": "s2", "state": "running" },
            { "cwd": "/tmp" }, // workspace doc — no session_id, ignored
            { "session_id": "s1", "kind": "duplicate" }, // dedup
        ]);
        let ids = extract_session_ids(&v);
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("s1"));
        assert!(ids.contains("s2"));
    }

    #[test]
    fn extract_session_ids_returns_empty_for_non_array() {
        assert!(extract_session_ids(&json!({})).is_empty());
        assert!(extract_session_ids(&Value::Null).is_empty());
    }

    // ─── Step E: pure helpers ────────────────────────────────────────────

    #[test]
    fn cost_tick_emits_when_no_prior_tick() {
        let now = Instant::now();
        assert!(should_emit_cost_tick(None, now));
    }

    #[test]
    fn cost_tick_coalesces_inside_window() {
        let now = Instant::now();
        // Two ticks 10ms apart should coalesce: ≤10/s == ≥100ms gap.
        let prev = now;
        let later = now + Duration::from_millis(10);
        assert!(!should_emit_cost_tick(Some(prev), later));
    }

    #[test]
    fn cost_tick_emits_after_window() {
        let now = Instant::now();
        let prev = now;
        let later = now + Duration::from_millis(120);
        assert!(should_emit_cost_tick(Some(prev), later));
    }

    #[test]
    fn fanout_pump_coalesces_cost_ticks_to_10_per_second() {
        // Replay the policy synchronously: feed 100 ticks across 1s of
        // virtual time and assert ≤10 emissions. Mirrors the runtime gate
        // that lives inside push_to_browser.
        let mut last: Option<Instant> = None;
        let start = Instant::now();
        let mut emitted = 0;
        for i in 0..100 {
            let now = start + Duration::from_millis(i * 10);
            if should_emit_cost_tick(last, now) {
                emitted += 1;
                last = Some(now);
            }
        }
        assert!(emitted <= 10, "expected ≤10 emissions in 1s, got {emitted}");
    }

    #[test]
    fn diff_workers_returns_none_when_unchanged() {
        let mut a = BTreeMap::new();
        a.insert("turn-orchestrator".into(), "up".into());
        let b = a.clone();
        assert!(diff_workers(&a, &b, &["turn-orchestrator"]).is_none());
    }

    #[test]
    fn fanout_workers_poll_diffs_correctly() {
        let mut prev = BTreeMap::new();
        prev.insert("turn-orchestrator".into(), "up".into());
        prev.insert("provider-router".into(), "up".into());

        let mut next = prev.clone();
        next.insert("provider-router".into(), "down".into());

        let expected = ["turn-orchestrator", "provider-router", "missing-worker"];
        let payload = diff_workers(&prev, &next, &expected).expect("change → push");
        assert_eq!(payload["total"], json!(3));
        assert_eq!(payload["up"], json!(1));
        assert_eq!(payload["down"], json!(2));
        let workers = payload["workers"].as_array().unwrap();
        assert_eq!(workers.len(), 3);
        // Sorted by name.
        assert_eq!(workers[0]["name"], json!("missing-worker"));
        assert_eq!(workers[0]["status"], json!("down"));
        assert_eq!(workers[1]["name"], json!("provider-router"));
        assert_eq!(workers[1]["status"], json!("down"));
        assert_eq!(workers[2]["name"], json!("turn-orchestrator"));
        assert_eq!(workers[2]["status"], json!("up"));
    }

    #[test]
    fn fanout_approval_pump_emits_resolved_on_removal() {
        let mut prev: HashMap<String, Value> = HashMap::new();
        prev.insert(
            "tc-1".into(),
            json!({ "function_call_id": "tc-1", "tool_call_id": "tc-1", "function_id": "write", "tool_name": "write" }),
        );
        let next: HashMap<String, Value> = HashMap::new();
        let (requested, resolved) = diff_approvals(&prev, &next);
        assert!(requested.is_empty());
        assert_eq!(resolved, vec!["tc-1".to_string()]);
    }

    #[test]
    fn fanout_approval_pump_emits_requested_then_resolved_in_sequence() {
        // Step 1: empty -> {tc-1}
        let initial: HashMap<String, Value> = HashMap::new();
        let mut after_request: HashMap<String, Value> = HashMap::new();
        after_request.insert(
            "tc-1".into(),
            json!({ "function_call_id": "tc-1", "tool_call_id": "tc-1", "function_id": "rm", "tool_name": "rm" }),
        );
        let (added, removed) = diff_approvals(&initial, &after_request);
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].0, "tc-1");
        assert!(removed.is_empty());

        // Step 2: {tc-1} -> {} after user resolves.
        let after_resolve: HashMap<String, Value> = HashMap::new();
        let (added2, removed2) = diff_approvals(&after_request, &after_resolve);
        assert!(added2.is_empty());
        assert_eq!(removed2, vec!["tc-1".to_string()]);
    }

    #[test]
    fn extract_worker_status_reads_array_form() {
        let v = json!([
            { "name": "turn-orchestrator", "status": "up" },
            { "name": "provider-router", "state": "down" },
            { "id": "session-tree" }, // no status -> defaults to "up"
            { "no_name_field": true }, // skipped
        ]);
        let m = extract_worker_status(&v);
        assert_eq!(m.get("turn-orchestrator"), Some(&"up".to_string()));
        assert_eq!(m.get("provider-router"), Some(&"down".to_string()));
        assert_eq!(m.get("session-tree"), Some(&"up".to_string()));
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn extract_worker_status_reads_workers_envelope() {
        let v = json!({
            "workers": [{ "name": "harness", "status": "up" }]
        });
        assert_eq!(extract_worker_status(&v).len(), 1);
    }

    #[test]
    fn summarize_budgets_sums_spent_and_groups_by_period() {
        let resp = json!({
            "budgets": [
                { "id": "a", "spent_usd": 1.5, "period": "daily" },
                { "id": "b", "spent_usd": 2.5, "period": "daily" },
                { "id": "c", "spent_usd": 4.0, "period": "monthly" },
            ]
        });
        let s = summarize_budgets(&resp);
        assert_eq!(s["usd_today"], json!(8.0));
        assert_eq!(s["budgets"], json!(3));
        assert_eq!(s["by_period"]["daily"], json!(4.0));
        assert_eq!(s["by_period"]["monthly"], json!(4.0));
    }

    #[test]
    fn summarize_budgets_handles_bare_array() {
        let resp = json!([{ "id": "a", "spent_usd": 1.0, "period": "daily" }]);
        let s = summarize_budgets(&resp);
        assert_eq!(s["usd_today"], json!(1.0));
    }

    #[test]
    fn outbound_for_returns_same_handle_across_calls() {
        let mut s = FanoutState::default();
        let a = s.outbound_for("browser-a");
        let b = s.outbound_for("browser-a");
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn unsubscribe_clears_outbound_when_last_sub_removed() {
        let mut s = FanoutState::default();
        s.subscribe("browser-a".into(), Some("sess-1".into()));
        let _ = s.outbound_for("browser-a");
        s.unsubscribe("browser-a", Some("sess-1".into()));
        assert_eq!(s.outbound.len(), 0);
    }

    /// fanout_pump_drops_oldest_and_emits_resync_on_overflow — we exercise
    /// the cap predicate directly, since spawning 2000 real tasks against a
    /// fake III takes the test from "unit" to "harness". The cap branch in
    /// `push_to_browser` is `(in_flight as usize) >= PER_BROWSER_QUEUE_CAP`,
    /// so we model the same comparison here against the same constant.
    #[test]
    fn fanout_pump_drops_oldest_and_emits_resync_on_overflow() {
        let in_flight = AtomicU64::new(PER_BROWSER_QUEUE_CAP as u64);
        // Saturated: next push must drop + resync.
        let observed = usize::try_from(in_flight.fetch_add(1, Ordering::SeqCst)).unwrap();
        assert!(observed >= PER_BROWSER_QUEUE_CAP);

        // After dropping (rollback), capacity reflects the rollback, not the
        // failed push.
        in_flight.fetch_sub(1, Ordering::SeqCst);
        assert_eq!(
            usize::try_from(in_flight.load(Ordering::SeqCst)).unwrap(),
            PER_BROWSER_QUEUE_CAP
        );

        // Resync dedup: first attempt sets the flag, second attempt sees it.
        let pending = std::sync::atomic::AtomicBool::new(false);
        let first = pending.swap(true, Ordering::SeqCst);
        let second = pending.swap(true, Ordering::SeqCst);
        assert!(!first, "first overflow should emit resync");
        assert!(second, "second overflow should be deduped");
    }
}
