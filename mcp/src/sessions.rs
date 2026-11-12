//! MCP sessions and their observers.
//!
//! Streamable HTTP gives each client an `Mcp-Session-Id` at `initialize`.
//! A session owns the observers it created with `iii_observe`; every
//! matching invocation the engine delivers to the shared sink
//! (`mcp::observed`) is routed here by `observer_id` and buffered until the
//! client reads it with `iii_events`. Claude Code and other clients cannot be
//! woken mid-turn, so buffering is the delivery.
//!
//! Sessions idle longer than the TTL are reaped, disposing their observers:
//! HTTP clients do not always send `DELETE` when they go away.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub use crate::bus::BusError;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// An observer the engine is setting up (an iii-sdk `ObserverHandle`).
pub trait ObserverLike: Send + Sync {
    fn id(&self) -> String;
    fn state(&self) -> String;
    fn ready(&self) -> BoxFuture<'_, Result<(), BusError>>;
    fn dispose(&self) -> BoxFuture<'_, ()>;
}

/// Creates observers on the engine.
pub trait Observe: Send + Sync {
    fn observe(&self, req: &ObserveRequest, target: &str) -> Box<dyn ObserverLike>;
}

/// The `iii_observe` tool arguments.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ObserveRequest {
    #[serde(default)]
    pub function_id: String,
    #[serde(default)]
    pub filters: Option<Value>,
    #[serde(default)]
    pub args: Option<Value>,
    #[serde(default)]
    pub capture: Option<Vec<String>>,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub max_events: Option<u64>,
    #[serde(default)]
    pub expires_in_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub struct SessionLimits {
    /// Per-observer buffer; the oldest events are dropped beyond it.
    pub max_buffered: usize,
    /// A session nobody used for this long is ended.
    pub idle_ttl: Duration,
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            max_buffered: 500,
            idle_ttl: Duration::from_secs(30 * 60),
        }
    }
}

/// How long `iii_observe` waits for the engine to activate an observer. An
/// observer of a function nobody hosts yet stays `pending`, which is valid.
const READY_WAIT: Duration = Duration::from_secs(5);

struct Entry {
    function_id: String,
    filters: Option<Value>,
    handle: Arc<dyn ObserverLike>,
    events: VecDeque<(u64, Value)>,
    received: u64,
    dropped: u64,
}

struct Session {
    last_seen: Instant,
    order: Vec<String>,
    observers: HashMap<String, Entry>,
}

#[derive(Default)]
struct Inner {
    sessions: HashMap<String, Session>,
    /// observer id -> session id, for routing deliveries.
    owner: HashMap<String, String>,
    seq: u64,
}

pub struct Sessions {
    limits: SessionLimits,
    inner: Mutex<Inner>,
}

impl Sessions {
    pub fn new(limits: SessionLimits) -> Self {
        Self {
            limits,
            inner: Mutex::new(Inner::default()),
        }
    }

    pub fn create(&self) -> String {
        let id = Uuid::new_v4().to_string();
        self.inner.lock().unwrap().sessions.insert(
            id.clone(),
            Session {
                last_seen: Instant::now(),
                order: vec![],
                observers: HashMap::new(),
            },
        );
        id
    }

    /// Mark a session used; `false` when it does not exist.
    pub fn touch(&self, session_id: &str) -> bool {
        match self.inner.lock().unwrap().sessions.get_mut(session_id) {
            Some(s) => {
                s.last_seen = Instant::now();
                true
            }
            None => false,
        }
    }

    pub async fn observe<O: Observe + ?Sized>(
        &self,
        observe: &O,
        session_id: &str,
        req: ObserveRequest,
        target: &str,
    ) -> Result<Value, String> {
        let function_id = req.function_id.trim().to_string();
        if function_id.is_empty() {
            return Err("function_id is required".into());
        }
        if !self.touch(session_id) {
            return Err(format!("unknown session: {session_id}"));
        }
        let handle: Arc<dyn ObserverLike> = Arc::from(observe.observe(
            &ObserveRequest {
                function_id: function_id.clone(),
                ..req.clone()
            },
            target,
        ));
        let observer_id = handle.id();
        {
            // Recorded before activation, so an early delivery is routed.
            let mut inner = self.inner.lock().unwrap();
            let Some(session) = inner.sessions.get_mut(session_id) else {
                return Err(format!("unknown session: {session_id}"));
            };
            session.order.push(observer_id.clone());
            session.observers.insert(
                observer_id.clone(),
                Entry {
                    function_id: function_id.clone(),
                    filters: req.filters.clone(),
                    handle: handle.clone(),
                    events: VecDeque::new(),
                    received: 0,
                    dropped: 0,
                },
            );
            inner
                .owner
                .insert(observer_id.clone(), session_id.to_string());
        }
        if let Ok(Err(e)) = tokio::time::timeout(READY_WAIT, handle.ready()).await {
            self.forget(session_id, &observer_id);
            handle.dispose().await;
            return Err(format!("observe refused: {}: {}", e.code, e.message));
        }
        Ok(
            json!({ "observer_id": observer_id, "function_id": function_id, "state": handle.state() }),
        )
    }

    /// Remove an observer from its session; returns its entry.
    fn forget(&self, session_id: &str, observer_id: &str) -> Option<Entry> {
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.sessions.get_mut(session_id).and_then(|s| {
            s.order.retain(|o| o != observer_id);
            s.observers.remove(observer_id)
        })?;
        inner.owner.remove(observer_id);
        Some(entry)
    }

    /// Route one observed invocation to its session; `false` when it belongs
    /// to no live observer (dropped, never resurrected).
    pub fn deliver(&self, event: Value) -> bool {
        let Some(observer_id) = event
            .get("observer_id")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return false;
        };
        let mut inner = self.inner.lock().unwrap();
        let Some(session_id) = inner.owner.get(&observer_id).cloned() else {
            return false;
        };
        inner.seq += 1;
        let seq = inner.seq;
        let max = self.limits.max_buffered;
        let Some(entry) = inner
            .sessions
            .get_mut(&session_id)
            .and_then(|s| s.observers.get_mut(&observer_id))
        else {
            return false;
        };
        entry.received += 1;
        entry.events.push_back((seq, event));
        while entry.events.len() > max {
            entry.events.pop_front();
            entry.dropped += 1;
        }
        true
    }

    pub fn events(
        &self,
        session_id: &str,
        observer_id: Option<&str>,
        max: usize,
    ) -> Result<Value, String> {
        let max = if max == 0 { 50 } else { max };
        let mut inner = self.inner.lock().unwrap();
        let session = inner
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session: {session_id}"))?;
        session.last_seen = Instant::now();
        let ids: Vec<String> = match observer_id {
            Some(id) if session.observers.contains_key(id) => vec![id.to_string()],
            Some(id) => return Err(format!("unknown observer: {id}")),
            None => session.order.clone(),
        };
        let mut queued: Vec<(u64, String)> = ids
            .iter()
            .flat_map(|id| {
                session.observers[id]
                    .events
                    .iter()
                    .map(move |(seq, _)| (*seq, id.clone()))
            })
            .collect();
        queued.sort_by_key(|(seq, _)| *seq);
        let total = queued.len();
        let mut events = Vec::new();
        for (_, id) in queued.into_iter().take(max) {
            if let Some((_, e)) = session
                .observers
                .get_mut(&id)
                .and_then(|en| en.events.pop_front())
            {
                events.push(e);
            }
        }
        let mut dropped = 0;
        for id in &ids {
            if let Some(en) = session.observers.get_mut(id) {
                dropped += en.dropped;
                en.dropped = 0;
            }
        }
        let remaining = total - events.len();
        Ok(json!({ "events": events, "remaining": remaining, "dropped": dropped }))
    }

    pub fn list(&self, session_id: &str) -> Result<Vec<Value>, String> {
        let mut inner = self.inner.lock().unwrap();
        let session = inner
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session: {session_id}"))?;
        session.last_seen = Instant::now();
        Ok(session
            .order
            .iter()
            .filter_map(|id| session.observers.get(id).map(|e| (id, e)))
            .map(|(id, e)| {
                let mut v = json!({
                    "observer_id": id,
                    "function_id": e.function_id,
                    "state": e.handle.state(),
                    "buffered": e.events.len(),
                    "received": e.received,
                    "dropped": e.dropped,
                });
                if let Some(f) = &e.filters {
                    v["filters"] = f.clone();
                }
                v
            })
            .collect())
    }

    pub async fn unobserve(&self, session_id: &str, observer_id: &str) -> Result<Value, String> {
        if !self.touch(session_id) {
            return Err(format!("unknown session: {session_id}"));
        }
        let entry = self
            .forget(session_id, observer_id)
            .ok_or_else(|| format!("unknown observer: {observer_id}"))?;
        entry.handle.dispose().await;
        Ok(json!({ "observer_id": observer_id, "removed": true, "discarded": entry.events.len() }))
    }

    /// End a session and dispose its observers; `false` when unknown.
    pub async fn end(&self, session_id: &str) -> bool {
        let handles: Vec<Arc<dyn ObserverLike>> = {
            let mut inner = self.inner.lock().unwrap();
            let Some(session) = inner.sessions.remove(session_id) else {
                return false;
            };
            for id in session.observers.keys() {
                inner.owner.remove(id);
            }
            session.observers.into_values().map(|e| e.handle).collect()
        };
        for h in handles {
            h.dispose().await;
        }
        true
    }

    /// End every session idle longer than the TTL; returns how many.
    pub async fn reap(&self) -> usize {
        let expired: Vec<String> = {
            let inner = self.inner.lock().unwrap();
            inner
                .sessions
                .iter()
                .filter(|(_, s)| s.last_seen.elapsed() > self.limits.idle_ttl)
                .map(|(id, _)| id.clone())
                .collect()
        };
        let mut n = 0;
        for id in expired {
            if self.end(&id).await {
                n += 1;
            }
        }
        n
    }

    pub async fn close_all(&self) {
        let ids: Vec<String> = self
            .inner
            .lock()
            .unwrap()
            .sessions
            .keys()
            .cloned()
            .collect();
        for id in ids {
            self.end(&id).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// An observer the engine accepted (or refused) at once.
    struct FakeObserver {
        id: String,
        refused: Option<BusError>,
        disposed: Arc<AtomicUsize>,
    }

    impl ObserverLike for FakeObserver {
        fn id(&self) -> String {
            self.id.clone()
        }
        fn state(&self) -> String {
            if self.refused.is_some() {
                "error".into()
            } else {
                "active".into()
            }
        }
        fn ready(&self) -> BoxFuture<'_, Result<(), BusError>> {
            let r = self.refused.clone();
            Box::pin(async move { r.map_or(Ok(()), Err) })
        }
        fn dispose(&self) -> BoxFuture<'_, ()> {
            self.disposed.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {})
        }
    }

    /// Creates observers `obs-1`, `obs-2`, ... and records what was asked.
    #[derive(Default)]
    struct FakeObserve {
        next: AtomicUsize,
        refuse: Option<BusError>,
        disposed: Arc<AtomicUsize>,
        asked: std::sync::Mutex<Vec<(ObserveRequest, String)>>,
    }

    impl Observe for FakeObserve {
        fn observe(&self, req: &ObserveRequest, target: &str) -> Box<dyn ObserverLike> {
            self.asked
                .lock()
                .unwrap()
                .push((req.clone(), target.to_string()));
            let n = self.next.fetch_add(1, Ordering::SeqCst) + 1;
            Box::new(FakeObserver {
                id: format!("obs-{n}"),
                refused: self.refuse.clone(),
                disposed: self.disposed.clone(),
            })
        }
    }

    fn store() -> Sessions {
        Sessions::new(SessionLimits {
            max_buffered: 500,
            idle_ttl: Duration::from_secs(60),
        })
    }

    fn event(observer_id: &str, input: Value) -> Value {
        json!({ "observer_id": observer_id, "function_id": "state::set", "status": "success", "input": input })
    }

    fn req(function_id: &str) -> ObserveRequest {
        serde_json::from_value(json!({ "function_id": function_id })).unwrap()
    }

    #[test]
    fn sessions_are_created_with_unique_ids_and_known_afterwards() {
        let s = store();
        let a = s.create();
        let b = s.create();
        assert_ne!(a, b);
        assert!(s.touch(&a));
        assert!(!s.touch("nope"));
    }

    #[test]
    fn observe_request_parses_the_tool_arguments() {
        let r: ObserveRequest = serde_json::from_value(json!({
            "function_id": "orders::create",
            "filters": { "input.amount": { "$gte": 500 } },
            "args": { "table": "orders" },
            "capture": ["input", "error"],
            "namespace": "default",
            "max_events": 3,
            "expires_in_ms": 60000
        }))
        .unwrap();
        assert_eq!(r.function_id, "orders::create");
        assert_eq!(
            r.capture.as_deref(),
            Some(&["input".to_string(), "error".to_string()][..])
        );
        assert_eq!(r.max_events, Some(3));
    }

    #[tokio::test]
    async fn observe_targets_the_sink_and_records_the_observer_in_the_session() {
        let s = store();
        let o = FakeObserve::default();
        let sid = s.create();
        let r = s
            .observe(&o, &sid, req("state::set"), "mcp::observed")
            .await
            .unwrap();
        assert_eq!(
            r,
            json!({ "observer_id": "obs-1", "function_id": "state::set", "state": "active" })
        );
        assert_eq!(o.asked.lock().unwrap()[0].1, "mcp::observed");
        assert_eq!(s.list(&sid).unwrap()[0]["observer_id"], "obs-1");
    }

    #[tokio::test]
    async fn observe_rejects_an_empty_function_id_and_an_unknown_session() {
        let s = store();
        let o = FakeObserve::default();
        let sid = s.create();
        assert!(s
            .observe(&o, &sid, req(" "), "mcp::observed")
            .await
            .unwrap_err()
            .contains("function_id"));
        assert!(s
            .observe(&o, "nope", req("a::b"), "mcp::observed")
            .await
            .unwrap_err()
            .contains("unknown session"));
        assert!(o.asked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_refused_observer_is_disposed_and_reported_with_its_code() {
        let s = store();
        let o = FakeObserve {
            refuse: Some(BusError::new("INVALID_ARGS", "bad filter")),
            ..Default::default()
        };
        let sid = s.create();
        let e = s
            .observe(&o, &sid, req("a::b"), "mcp::observed")
            .await
            .unwrap_err();
        assert!(
            e.contains("INVALID_ARGS") && e.contains("bad filter"),
            "{e}"
        );
        assert_eq!(o.disposed.load(Ordering::SeqCst), 1);
        assert!(s.list(&sid).unwrap().is_empty());
    }

    #[tokio::test]
    async fn deliveries_are_routed_to_the_owning_session_only() {
        let s = store();
        let o = FakeObserve::default();
        let a = s.create();
        let b = s.create();
        s.observe(&o, &a, req("x::y"), "mcp::observed")
            .await
            .unwrap();
        s.observe(&o, &b, req("x::y"), "mcp::observed")
            .await
            .unwrap();
        assert!(s.deliver(event("obs-1", json!(1))));
        assert!(s.deliver(event("obs-2", json!(2))));
        assert!(
            !s.deliver(event("obs-999", json!(3))),
            "unknown observer must be dropped"
        );
        assert!(!s.deliver(json!("not an event")));
        let ea = s.events(&a, None, 50).unwrap();
        assert_eq!(
            ea["events"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["input"].clone())
                .collect::<Vec<_>>(),
            vec![json!(1)]
        );
        // Another session cannot read or remove it.
        assert!(s
            .events(&b, Some("obs-1"), 50)
            .unwrap_err()
            .contains("unknown observer"));
    }

    #[tokio::test]
    async fn events_drain_oldest_first_with_max_remaining_and_dropped() {
        let s = Sessions::new(SessionLimits {
            max_buffered: 3,
            idle_ttl: Duration::from_secs(60),
        });
        let o = FakeObserve::default();
        let sid = s.create();
        s.observe(&o, &sid, req("a::b"), "mcp::observed")
            .await
            .unwrap();
        s.observe(&o, &sid, req("c::d"), "mcp::observed")
            .await
            .unwrap();
        for i in 0..5 {
            s.deliver(event("obs-1", json!(i)));
        }
        s.deliver(event("obs-2", json!("b")));
        let first = s.events(&sid, None, 2).unwrap();
        let inputs = |v: &Value| {
            v["events"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["input"].clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(inputs(&first), vec![json!(2), json!(3)]);
        assert_eq!(first["remaining"], 2);
        assert_eq!(first["dropped"], 2);
        let rest = s.events(&sid, None, 50).unwrap();
        assert_eq!(inputs(&rest), vec![json!(4), json!("b")]);
        assert_eq!(rest["dropped"], 0);
        assert_eq!(
            s.events(&sid, Some("obs-1"), 50).unwrap()["events"],
            json!([])
        );
    }

    #[tokio::test]
    async fn list_reports_counts_and_filters() {
        let s = store();
        let o = FakeObserve::default();
        let sid = s.create();
        let mut r = req("a::b");
        r.filters = Some(json!({ "input.x": { "$eq": 1 } }));
        s.observe(&o, &sid, r, "mcp::observed").await.unwrap();
        s.deliver(event("obs-1", json!(1)));
        s.deliver(event("obs-1", json!(2)));
        s.events(&sid, Some("obs-1"), 1).unwrap();
        assert_eq!(
            s.list(&sid).unwrap(),
            vec![json!({
                "observer_id": "obs-1", "function_id": "a::b",
                "filters": { "input.x": { "$eq": 1 } },
                "state": "active", "buffered": 1, "received": 2, "dropped": 0
            })]
        );
    }

    #[tokio::test]
    async fn unobserve_disposes_only_the_sessions_own_observer() {
        let s = store();
        let o = FakeObserve::default();
        let a = s.create();
        let b = s.create();
        s.observe(&o, &a, req("a::b"), "mcp::observed")
            .await
            .unwrap();
        s.deliver(event("obs-1", json!(1)));
        assert!(s
            .unobserve(&b, "obs-1")
            .await
            .unwrap_err()
            .contains("unknown observer"));
        assert_eq!(
            s.unobserve(&a, "obs-1").await.unwrap(),
            json!({ "observer_id": "obs-1", "removed": true, "discarded": 1 })
        );
        assert_eq!(o.disposed.load(Ordering::SeqCst), 1);
        assert!(
            !s.deliver(event("obs-1", json!(2))),
            "late delivery is not resurrected"
        );
    }

    #[tokio::test]
    async fn ending_a_session_disposes_its_observers() {
        let s = store();
        let o = FakeObserve::default();
        let sid = s.create();
        s.observe(&o, &sid, req("a::b"), "mcp::observed")
            .await
            .unwrap();
        s.observe(&o, &sid, req("c::d"), "mcp::observed")
            .await
            .unwrap();
        assert!(s.end(&sid).await);
        assert_eq!(o.disposed.load(Ordering::SeqCst), 2);
        assert!(!s.touch(&sid));
        assert!(!s.end(&sid).await);
    }

    #[tokio::test]
    async fn idle_sessions_are_reaped_and_active_ones_kept() {
        let s = Sessions::new(SessionLimits {
            max_buffered: 10,
            idle_ttl: Duration::from_millis(30),
        });
        let o = FakeObserve::default();
        let idle = s.create();
        let busy = s.create();
        s.observe(&o, &idle, req("a::b"), "mcp::observed")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(s.touch(&busy));
        assert_eq!(s.reap().await, 1);
        assert!(!s.touch(&idle));
        assert!(s.touch(&busy));
        assert_eq!(o.disposed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn close_all_disposes_everything() {
        let s = store();
        let o = FakeObserve::default();
        let a = s.create();
        let b = s.create();
        s.observe(&o, &a, req("a::b"), "mcp::observed")
            .await
            .unwrap();
        s.observe(&o, &b, req("a::b"), "mcp::observed")
            .await
            .unwrap();
        s.close_all().await;
        assert_eq!(o.disposed.load(Ordering::SeqCst), 2);
        assert!(!s.touch(&a));
    }
}
