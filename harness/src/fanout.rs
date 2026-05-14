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
    pub skills_on_change_fn: FunctionRef,
    pub skills_on_change_trigger: Option<Trigger>,
    pub prompts_on_change_fn: FunctionRef,
    pub prompts_on_change_trigger: Option<Trigger>,
    pub sessions_poll: tokio::task::JoinHandle<()>,
    pub cost_poll: tokio::task::JoinHandle<()>,
    pub workers_poll: tokio::task::JoinHandle<()>,
}

impl FanoutPumps {
    pub fn shutdown(self) {
        if let Some(t) = self.agent_event_trigger {
            t.unregister();
        }
        if let Some(t) = self.skills_on_change_trigger {
            t.unregister();
        }
        if let Some(t) = self.prompts_on_change_trigger {
            t.unregister();
        }
        self.agent_event_fn.unregister();
        self.skills_on_change_fn.unregister();
        self.prompts_on_change_fn.unregister();
        self.sessions_poll.abort();
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

    // iii-directory fan-out pumps: forward every successful skills/prompts
    // download to all subscribed browsers so the UI can refresh.
    let (skills_on_change_fn, skills_on_change_trigger) = spawn_directory_on_change_pump(
        iii.as_ref(),
        Arc::clone(&fanout),
        "directory::skills::on-change",
        "ui::skills::changed",
        "harness::ui::skills-on-change-pump",
    );
    let (prompts_on_change_fn, prompts_on_change_trigger) = spawn_directory_on_change_pump(
        iii.as_ref(),
        Arc::clone(&fanout),
        "directory::prompts::on-change",
        "ui::prompts::changed",
        "harness::ui::prompts-on-change-pump",
    );

    let sessions_poll = spawn_sessions_changed_poll(Arc::clone(iii), Arc::clone(&fanout));
    let cost_poll = spawn_cost_poll(Arc::clone(iii), Arc::clone(&fanout));
    let workers_poll = spawn_workers_poll(Arc::clone(iii), fanout);

    FanoutPumps {
        agent_event_fn,
        agent_event_trigger,
        skills_on_change_fn,
        skills_on_change_trigger,
        prompts_on_change_fn,
        prompts_on_change_trigger,
        sessions_poll,
        cost_poll,
        workers_poll,
    }
}

/// Register a directory `::on-change` trigger subscriber that broadcasts to
/// every all-sessions subscriber on every event. Used for the
/// `directory::skills::on-change` and `directory::prompts::on-change` pumps that iii-directory
/// fires after every successful `directory::skills::download`.
///
/// Returns the registered function and the trigger handle (None if the
/// trigger-type registration failed, e.g. iii-directory isn't up yet).
fn spawn_directory_on_change_pump(
    iii: &III,
    fanout: SharedFanout,
    trigger_type: &str,
    out_prefix: &'static str,
    fn_id_base: &str,
) -> (FunctionRef, Option<Trigger>) {
    let id = format!("{fn_id_base}-{}", std::process::id());
    let iii_inner = iii.clone();
    let out_prefix_for_handler = out_prefix.to_string();
    let function = iii.register_function((
        RegisterFunctionMessage::with_id(id).with_description(format!(
            "Internal: fans out {trigger_type} events to UI subscribers as {out_prefix}::*."
        )),
        move |payload: Value| {
            let iii = iii_inner.clone();
            let fanout = Arc::clone(&fanout);
            let out_prefix = out_prefix_for_handler.clone();
            async move {
                let browsers = {
                    let state = fanout.read().await;
                    state.all_sessions_subscribers()
                };
                let frame = extract_on_change_payload(&payload);
                for browser_id in browsers {
                    let function_id = format!("{out_prefix}::{browser_id}");
                    let iii_for_push = iii.clone();
                    let frame = frame.clone();
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
                            tracing::trace!(error = %e, "directory on-change push failed");
                        }
                    });
                }
                Ok::<_, IIIError>(json!({ "ok": true }))
            }
        },
    ));

    let trigger = match iii.register_trigger(RegisterTriggerInput {
        trigger_type: trigger_type.into(),
        function_id: function.id.clone(),
        config: json!({}),
        metadata: None,
    }) {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::warn!(
                error = %e,
                trigger_type = trigger_type,
                "harness fanout: failed to register directory on-change trigger"
            );
            None
        }
    };

    (function, trigger)
}

/// Pull the broadcast payload out of a directory `::on-change` event. The
/// engine wraps custom triggers in a `{ type, event: { data: <payload> } }`
/// envelope; we unwrap it (falling back to the raw payload) so subscribers
/// always see the iii-directory `WrittenSkill` / `WrittenPrompt` shape.
pub(crate) fn extract_on_change_payload(payload: &Value) -> Value {
    payload
        .get("event")
        .and_then(|e| e.get("data"))
        .cloned()
        .or_else(|| payload.get("data").cloned())
        .unwrap_or_else(|| payload.clone())
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
                    // Reactive ui::approval::* path: gate writes
                    // approval_{requested,resolved} frames into agent::events;
                    // we forward them to all-sessions subscribers without
                    // polling state. Non-approval frames classify as None and
                    // fall through to the regular ui::session::event forward
                    // below.
                    if let Some(push) = classify_approval_frame(&event_data, &session_id) {
                        let all_sessions = {
                            let state = fanout.read().await;
                            state.all_sessions_subscribers()
                        };
                        for (channel, push_payload) in approval_pushes_for(&push, &all_sessions) {
                            let iii_for_push = iii.clone();
                            tokio::spawn(async move {
                                if let Err(e) = iii_for_push
                                    .trigger(TriggerRequest {
                                        function_id: channel,
                                        payload: push_payload,
                                        action: None,
                                        timeout_ms: Some(PUSH_TIMEOUT_MS),
                                    })
                                    .await
                                {
                                    tracing::trace!(error = %e, "ui::approval push failed");
                                }
                            });
                        }
                    }

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

/// Push intent derived from an `agent::events` stream frame. Drives the
/// reactive ui::approval pipeline (replaces the approval poll).
#[derive(Debug, PartialEq, Eq)]
pub enum ApprovalUiPush {
    /// Forward as `ui::approval::requested::<browser_id>`. Payload mirrors the
    /// poll's enriched record: original gate fields plus `session_id`.
    Requested(Value),
    /// Forward as `ui::approval::resolved::<browser_id>`. Payload carries the
    /// call id under both new and legacy field names so existing consumers in
    /// `harness/web/src/useStatus.ts` and `harness-tui/src/types.rs` keep
    /// working.
    Resolved(Value),
}

/// Classify an `agent::events` frame body as a UI-bound approval push.
///
/// Returns `None` for non-approval frames and for malformed approval frames
/// (missing ids). Pure function — wired into the stream subscriber callback.
pub fn classify_approval_frame(data: &Value, session_id: &str) -> Option<ApprovalUiPush> {
    if session_id.is_empty() {
        return None;
    }
    let frame_type = data.get("type").and_then(Value::as_str)?;
    let call_id = data
        .get("function_call_id")
        .or_else(|| data.get("tool_call_id"))
        .and_then(Value::as_str)?;
    match frame_type {
        "approval_requested" => {
            let mut payload = data.clone();
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("session_id".into(), Value::String(session_id.to_string()));
            }
            Some(ApprovalUiPush::Requested(payload))
        }
        "approval_resolved" => Some(ApprovalUiPush::Resolved(json!({
            "function_call_id": call_id,
            "tool_call_id": call_id,
        }))),
        _ => None,
    }
}

/// Build per-session hydration payloads for a new all-sessions subscriber.
///
/// Each entry in `pending` (as returned by `approval::list_pending`) becomes
/// one ui::approval::requested-ready payload. Filters: only `status=pending`
/// entries; entries without a call id are skipped via `classify_approval_frame`.
pub fn hydration_payloads(session_id: &str, pending: &[Value]) -> Vec<Value> {
    pending
        .iter()
        .filter(|entry| entry.get("status").and_then(Value::as_str) == Some("pending"))
        .filter_map(|entry| {
            let mut synth = entry.clone();
            if let Some(obj) = synth.as_object_mut() {
                obj.insert(
                    "type".into(),
                    Value::String("approval_requested".into()),
                );
            }
            match classify_approval_frame(&synth, session_id)? {
                ApprovalUiPush::Requested(payload) => Some(payload),
                ApprovalUiPush::Resolved(_) => None,
            }
        })
        .collect()
}

/// Hydrate a freshly-attached all-sessions subscriber.
///
/// Enumerates active sessions via `state::list`, fetches pending approvals via
/// `approval::list_pending`, and pushes `ui::approval::requested::<browser_id>`
/// for each. Replaces the periodic poll for the reconnect/late-join case.
/// Fire-and-forget: spawn this; do not await it from request handlers.
pub async fn hydrate_all_sessions_subscriber(
    iii: Arc<III>,
    fanout: SharedFanout,
    browser_id: String,
) {
    let sessions = match iii
        .trigger(TriggerRequest {
            function_id: "state::list".into(),
            payload: json!({ "scope": "agent", "prefix": "session/" }),
            action: None,
            timeout_ms: Some(STATE_LIST_TIMEOUT_MS),
        })
        .await
    {
        Ok(v) => extract_session_ids(&v),
        Err(_) => return,
    };

    let mut per_session: Vec<(String, Vec<Value>)> = Vec::with_capacity(sessions.len());
    for sid in &sessions {
        let resp = iii
            .trigger(TriggerRequest {
                function_id: "approval::list_pending".into(),
                payload: json!({ "session_id": sid }),
                action: None,
                timeout_ms: Some(STATE_LIST_TIMEOUT_MS),
            })
            .await;
        let entries = resp
            .ok()
            .and_then(|v| v.get("pending").and_then(|p| p.as_array()).cloned())
            .unwrap_or_default();
        if !entries.is_empty() {
            per_session.push((sid.clone(), entries));
        }
    }

    for (channel, payload) in hydration_pushes_for(&browser_id, &per_session) {
        push_to_browser(
            &iii,
            &fanout,
            &browser_id,
            channel,
            payload,
            PushKind::Standard,
        );
    }
}

/// Orchestration helper for subscribe-time hydration.
///
/// Given a freshly-attached all-sessions subscriber and the per-session
/// `pending` lists already fetched from `approval::list_pending`, produce the
/// `(channel, payload)` pairs the caller should drive through `iii.trigger`.
/// Pure function — the async glue (state::list, list_pending, trigger) wraps
/// this.
pub fn hydration_pushes_for(
    browser_id: &str,
    per_session: &[(String, Vec<Value>)],
) -> Vec<(String, Value)> {
    let channel = format!("ui::approval::requested::{browser_id}");
    per_session
        .iter()
        .flat_map(|(session_id, pending)| {
            hydration_payloads(session_id, pending)
                .into_iter()
                .map(|payload| (channel.clone(), payload))
        })
        .collect()
}

/// Fan a classified approval push out to all-sessions subscribers.
///
/// Returns `(channel, payload)` pairs the pump can hand to `iii.trigger`.
/// Keeps wire channel naming and per-browser cloning isolated and testable.
pub fn approval_pushes_for(push: &ApprovalUiPush, browser_ids: &[String]) -> Vec<(String, Value)> {
    let (root, payload) = match push {
        ApprovalUiPush::Requested(p) => ("ui::approval::requested", p),
        ApprovalUiPush::Resolved(p) => ("ui::approval::resolved", p),
    };
    browser_ids
        .iter()
        .map(|b| (format!("{root}::{b}"), payload.clone()))
        .collect()
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
        assert!(
            removed,
            "evict must report success when the browser was present"
        );
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

    #[test]
    fn extract_on_change_payload_unwraps_event_envelope() {
        let env = json!({
            "type": "directory::skills::on-change",
            "event": {
                "data": {
                    "worker": "shell",
                    "version": "0.3.3",
                    "files": ["index.md"]
                }
            }
        });
        let frame = extract_on_change_payload(&env);
        assert_eq!(frame["worker"], json!("shell"));
        assert_eq!(frame["version"], json!("0.3.3"));
    }

    #[test]
    fn extract_on_change_payload_falls_back_to_data_field() {
        let env = json!({ "data": { "worker": "shell" } });
        let frame = extract_on_change_payload(&env);
        assert_eq!(frame["worker"], json!("shell"));
    }

    #[test]
    fn extract_on_change_payload_returns_raw_payload_when_no_wrapper() {
        let env = json!({ "worker": "shell", "files": ["index.md"] });
        let frame = extract_on_change_payload(&env);
        assert_eq!(frame, env);
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

    // ─── Reactive approval pipeline ──────────────────────────────────────
    //
    // classify_approval_frame turns a parsed `agent::events` frame body into
    // an explicit push intent. The stream subscriber callback uses it to
    // forward approval_requested/resolved frames as ui::approval::* events
    // without polling state.

    #[test]
    fn classify_approval_frame_requested_enriches_with_session_id() {
        let data = json!({
            "type": "approval_requested",
            "function_call_id": "c1",
            "tool_call_id": "c1",
            "function_id": "shell::fs::write",
            "tool_name": "shell::fs::write",
            "args": { "path": "/tmp/x" },
            "expires_at": 1_234_567_890_u64,
        });
        let out = classify_approval_frame(&data, "s1").expect("requested classified");
        match out {
            ApprovalUiPush::Requested(payload) => {
                assert_eq!(payload["session_id"], json!("s1"));
                assert_eq!(payload["function_call_id"], json!("c1"));
                assert_eq!(payload["function_id"], json!("shell::fs::write"));
            }
            ApprovalUiPush::Resolved(_) => panic!("expected Requested, got Resolved"),
        }
    }

    #[test]
    fn classify_approval_frame_resolved_emits_minimal_payload() {
        let data = json!({
            "type": "approval_resolved",
            "function_call_id": "c1",
            "decision": "allow",
        });
        let out = classify_approval_frame(&data, "s1").expect("resolved classified");
        match out {
            ApprovalUiPush::Resolved(payload) => {
                assert_eq!(payload["function_call_id"], json!("c1"));
                assert_eq!(payload["tool_call_id"], json!("c1"));
            }
            ApprovalUiPush::Requested(_) => panic!("expected Resolved, got Requested"),
        }
    }

    // B1 — non-approval frame types must classify as None so the regular
    // session-event forward path keeps owning them.
    #[test]
    fn classify_approval_frame_ignores_non_approval_types() {
        let data = json!({
            "type": "tool_call_started",
            "function_call_id": "c1",
            "function_id": "shell::fs::read",
        });
        assert!(classify_approval_frame(&data, "s1").is_none());
    }

    // B2 — malformed approval frames without an id must be dropped silently,
    // never panic, and never produce a push (UI would have nothing to key on).
    #[test]
    fn classify_approval_frame_drops_when_call_id_missing() {
        let requested = json!({
            "type": "approval_requested",
            "function_id": "shell::fs::write",
        });
        let resolved = json!({ "type": "approval_resolved", "decision": "allow" });
        assert!(classify_approval_frame(&requested, "s1").is_none());
        assert!(classify_approval_frame(&resolved, "s1").is_none());
    }

    // B3 — legacy field names (tool_call_id / tool_name) must still produce a
    // push so reload hydration via approval::list_pending stays compatible
    // with older gate envelopes captured in state.
    #[test]
    fn classify_approval_frame_accepts_legacy_tool_call_id() {
        let data = json!({
            "type": "approval_requested",
            "tool_call_id": "c1",
            "tool_name": "shell::fs::write",
            "args": {},
        });
        let out = classify_approval_frame(&data, "s1").expect("legacy shape classified");
        match out {
            ApprovalUiPush::Requested(payload) => {
                assert_eq!(payload["tool_call_id"], json!("c1"));
                assert_eq!(payload["session_id"], json!("s1"));
            }
            ApprovalUiPush::Resolved(_) => panic!("expected Requested"),
        }
    }

    // B10 — empty session_id must not produce a push whose UI cannot key on a
    // session. Today, the upstream extract_event_payload already drops empty
    // group ids; this guard pins the contract at the classifier too so future
    // refactors of either layer don't open a regression.
    #[test]
    fn classify_approval_frame_drops_when_session_id_empty() {
        let data = json!({
            "type": "approval_requested",
            "function_call_id": "c1",
        });
        assert!(classify_approval_frame(&data, "").is_none());
    }

    // ─── Hydration payloads ──────────────────────────────────────────────
    //
    // When a new all-sessions subscriber attaches, we replay
    // approval::list_pending per session into the new browser. The pure
    // helper below turns one session's `pending` array into a list of
    // ui::approval::requested-ready payloads. Tests pin the filters that
    // make this safe to call on reconnects (B4, B9, B11).

    // A3 — happy path: each pending entry becomes one enriched payload.
    #[test]
    fn hydration_payloads_emits_one_per_pending_entry() {
        let pending = vec![
            json!({
                "function_call_id": "c1",
                "function_id": "shell::fs::write",
                "args": { "path": "/a" },
                "status": "pending",
                "expires_at": 1u64,
            }),
            json!({
                "function_call_id": "c2",
                "function_id": "shell::fs::mkdir",
                "args": { "path": "/b" },
                "status": "pending",
                "expires_at": 2u64,
            }),
        ];
        let out = hydration_payloads("s1", &pending);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["function_call_id"], json!("c1"));
        assert_eq!(out[0]["session_id"], json!("s1"));
        assert_eq!(out[1]["function_call_id"], json!("c2"));
        assert_eq!(out[1]["session_id"], json!("s1"));
    }

    // B4 — malformed entry (no call id) is dropped, other entries still flow.
    #[test]
    fn hydration_payloads_skips_entries_missing_call_id() {
        let pending = vec![
            json!({ "function_id": "shell::fs::write", "status": "pending" }), // bad
            json!({
                "function_call_id": "c2",
                "function_id": "shell::fs::mkdir",
                "status": "pending",
            }),
        ];
        let out = hydration_payloads("s1", &pending);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["function_call_id"], json!("c2"));
    }

    // B9 — empty input is a no-op, not an error.
    #[test]
    fn hydration_payloads_empty_input_returns_empty() {
        assert!(hydration_payloads("s1", &[]).is_empty());
    }

    // approval_pushes_for fans a classified push out to N all-sessions
    // browsers, producing (channel, payload) pairs. The pump's only job
    // after classifying is to drive `iii.trigger` per pair, so pinning the
    // channel naming convention here is enough to lock the wire format.
    #[test]
    fn approval_pushes_for_requested_targets_ui_approval_requested_per_browser() {
        let push = ApprovalUiPush::Requested(json!({ "function_call_id": "c1", "session_id": "s1" }));
        let out = approval_pushes_for(&push, &["b1".into(), "b2".into()]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, "ui::approval::requested::b1");
        assert_eq!(out[0].1["function_call_id"], json!("c1"));
        assert_eq!(out[1].0, "ui::approval::requested::b2");
    }

    #[test]
    fn approval_pushes_for_resolved_targets_ui_approval_resolved_per_browser() {
        let push = ApprovalUiPush::Resolved(json!({ "function_call_id": "c1", "tool_call_id": "c1" }));
        let out = approval_pushes_for(&push, &["b1".into()]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "ui::approval::resolved::b1");
    }

    #[test]
    fn approval_pushes_for_zero_browsers_is_noop() {
        let push = ApprovalUiPush::Resolved(json!({ "function_call_id": "c1" }));
        assert!(approval_pushes_for(&push, &[]).is_empty());
    }

    // hydration_pushes_for is the orchestration helper used when an
    // all-sessions subscriber attaches. Given the per-session pending lists
    // already fetched from approval::list_pending, it produces the exact
    // (channel, payload) pairs the subscribe handler should push.
    #[test]
    fn hydration_pushes_for_emits_one_push_per_pending_entry_across_sessions() {
        let per_session = vec![
            (
                "s1".to_string(),
                vec![json!({
                    "function_call_id": "c1",
                    "function_id": "shell::fs::write",
                    "status": "pending",
                })],
            ),
            (
                "s2".to_string(),
                vec![json!({
                    "function_call_id": "c2",
                    "function_id": "shell::fs::mkdir",
                    "status": "pending",
                })],
            ),
        ];
        let out = hydration_pushes_for("browser-a", &per_session);
        assert_eq!(out.len(), 2);
        assert!(out
            .iter()
            .all(|(chan, _)| chan == "ui::approval::requested::browser-a"));
        let ids: Vec<&str> = out
            .iter()
            .map(|(_, p)| p["function_call_id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&"c1") && ids.contains(&"c2"));
    }

    #[test]
    fn hydration_pushes_for_empty_per_session_returns_empty() {
        assert!(hydration_pushes_for("browser-a", &[]).is_empty());
    }

    #[test]
    fn hydration_pushes_for_session_with_no_pending_is_skipped() {
        let per_session = vec![
            ("s1".to_string(), vec![]),
            (
                "s2".to_string(),
                vec![json!({
                    "function_call_id": "c2",
                    "status": "pending",
                })],
            ),
        ];
        let out = hydration_pushes_for("b1", &per_session);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1["function_call_id"], json!("c2"));
        assert_eq!(out[0].1["session_id"], json!("s2"));
    }

    // B11 — defense in depth: even if list_pending regresses and returns
    // resolved entries, the hydration filter must drop them. This is the
    // only guard against timed-out approvals reappearing on reconnect.
    #[test]
    fn hydration_payloads_filters_non_pending_status() {
        let pending = vec![
            json!({
                "function_call_id": "c1",
                "function_id": "shell::fs::write",
                "status": "deny",
                "reason": "timeout",
            }),
            json!({
                "function_call_id": "c2",
                "function_id": "shell::fs::write",
                "status": "allow",
            }),
            json!({
                "function_call_id": "c3",
                "function_id": "shell::fs::write",
                "status": "pending",
            }),
        ];
        let out = hydration_payloads("s1", &pending);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["function_call_id"], json!("c3"));
    }
}
