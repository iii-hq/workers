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

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

use iii_sdk::{
    FunctionRef, IIIError, RegisterFunctionMessage, RegisterTriggerInput, Trigger, TriggerRequest,
    III,
};
use serde_json::{json, Value};

/// Identity of a connected browser worker. Caller-supplied; we don't mint it.
pub type BrowserId = String;

/// `None` means "subscribe to all sessions / non-session topics".
pub type Subscription = Option<String>;

#[derive(Debug, Default)]
pub struct FanoutState {
    /// browser_id -> set of subscribed session ids ("__all__" sentinel = all-sessions)
    pub subs: HashMap<BrowserId, HashSet<String>>,
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
            }
        }
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
}

impl FanoutPumps {
    pub fn shutdown(self) {
        if let Some(t) = self.agent_event_trigger {
            t.unregister();
        }
        self.agent_event_fn.unregister();
        self.sessions_poll.abort();
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

    let sessions_poll = spawn_sessions_changed_poll(Arc::clone(iii), fanout);

    FanoutPumps {
        agent_event_fn,
        agent_event_trigger,
        sessions_poll,
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
                        // Fire-and-forget. The browser is allowed to be slow
                        // or absent; we don't want one stale browser to
                        // back up the whole pump.
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
                                tracing::trace!(error = %e, "ui push failed (browser likely gone)");
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
}
