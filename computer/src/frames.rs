//! `computer::on-frame-changed`: the observable function behind the live
//! viewport. The newest frame of each session lives in exactly one in-memory
//! slot ([`crate::session::Session::latest_frame`]); a call of this function
//! tells viewers that the slot changed, and they read it with
//! `computer::frame` (notify, then fetch). The notification carries no
//! image: frames are 60 KB-2 MB each, a payload copy per observer per frame
//! would be the very unbounded data path this replaces.
//!
//! The function is private and its handler returns the [`FrameChange`] it is
//! called with. Its hooks deliver it, and bound it:
//! - an observer names one session in its args (`{ session_id }`, required);
//!   the match hook files it under that id, so a change reaches only the
//!   observers of its session;
//! - at most [`MAX_FRAME_OBSERVERS`] observers; the added hook rejects more
//!   with `OBSERVER_LIMIT`;
//! - per observer, one [`DeliverySlot`], opened by the added hook and closed
//!   by the removed hook: at most ONE call in flight and ONE pending
//!   notification. A newer notification replaces the pending one (latest
//!   wins), so a slow or stuck viewer costs O(1) memory and one task, never
//!   a queue;
//! - the slot calls through its observer and waits for the answer, bounded
//!   by [`DELIVERY_TIMEOUT_MS`], so a slow consumer keeps its slot busy and
//!   the worker coalesces instead of pushing into engine or socket buffers.
//!
//! The deliver hook applies each observer's filters, offers the change to
//! the slots of those that match, and answers the caller with
//! `{ offered }`, the number of slots it reached.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::{
    Call, CallAction, CallRequest, IIIClient, Match, ObserveRejection, Observer, RegisterFunction,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const ON_FRAME_CHANGED_FN: &str = "computer::on-frame-changed";
pub const ON_FRAME_CHANGED_DESC: &str =
    "The newest screencast frame of one session changed (a new frame was stored, or the frame was \
     cleared because the screencast or session stopped). Args: { session_id } (required). \
     Each observer's target receives { session_id, epoch, frame_seq, change, width, height, mime?, \
     bytes?, timestamp }; no image: read it with computer::frame. Latest state only: intermediate \
     notifications are coalesced for slow consumers, nothing is stored or replayed. The caller \
     gets { offered }, the number of observers the change was offered to.";
/// Upper bound on live `computer::on-frame-changed` observers.
pub const MAX_FRAME_OBSERVERS: usize = 64;
/// Per-call timeout; a consumer slower than this only loses intermediate
/// notifications, never blocks the pump.
pub const DELIVERY_TIMEOUT_MS: u64 = 5_000;

/// The args of `computer::on-frame-changed`. `session_id` is required: a
/// viewer follows one desktop; the engine refuses unknown keys.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FrameArgs {
    /// The computer session whose frames to follow.
    pub session_id: String,
}

/// The session an observer follows: its args' `session_id`, trimmed and
/// non-empty.
pub fn followed_session(args: &FrameArgs) -> Result<String, String> {
    let session_id = args.session_id.trim();
    if session_id.is_empty() {
        return Err(format!(
            "{ON_FRAME_CHANGED_FN}: session_id must not be empty"
        ));
    }
    Ok(session_id.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FrameChangeKind {
    /// A new frame was stored; read it with `computer::frame`.
    Updated,
    /// The stored frame was dropped (screencast stopped, capture failed, or
    /// the session ended). Nothing to read until the next `updated`.
    Cleared,
}

/// The notification. Small and image-free.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrameChange {
    pub session_id: String,
    /// Identifies this incarnation of the session in this worker process
    /// (ms when it was created or restored). `frame_seq` restarts at 1 when
    /// `epoch` changes; compare `(epoch, frame_seq)` to order frames.
    pub epoch: i64,
    /// Sequence of the newest stored frame (monotonic within an epoch). For
    /// `cleared`, the sequence of the last frame that was stored.
    pub frame_seq: u64,
    pub change: FrameChangeKind,
    pub width: u32,
    pub height: u32,
    /// Image mime of the stored frame (`updated` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    /// Encoded image size in bytes (`updated` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    pub timestamp: i64,
}

/// One observer's end of its delivery slot. Production calls through the
/// observer ([`ObserverTarget`]); tests record.
#[async_trait]
pub trait FrameTarget: Send + Sync {
    /// The observer's id.
    fn id(&self) -> &str;
    /// One notification; returns once the target answered.
    async fn deliver(&self, payload: Value) -> Result<(), String>;
}

/// A call through the observer, waiting for the target's answer, bounded by
/// [`DELIVERY_TIMEOUT_MS`]. The engine applies the observer's metadata and
/// target namespace.
pub struct ObserverTarget {
    iii: IIIClient,
    observer: Observer,
}

impl ObserverTarget {
    pub fn new(iii: IIIClient, observer: Observer) -> Self {
        Self { iii, observer }
    }
}

#[async_trait]
impl FrameTarget for ObserverTarget {
    fn id(&self) -> &str {
        self.observer.id()
    }

    async fn deliver(&self, payload: Value) -> Result<(), String> {
        let request = CallRequest::observer(&self.observer, payload)
            .timeout(Duration::from_millis(DELIVERY_TIMEOUT_MS));
        self.iii
            .call(request)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Where a notification stands among those of its session: `(epoch,
/// frame_seq, cleared)`. A clear of frame N comes after frame N.
fn order_of(payload: &Value) -> (i64, u64, bool) {
    (
        payload["epoch"].as_i64().unwrap_or_default(),
        payload["frame_seq"].as_u64().unwrap_or_default(),
        payload["change"] == "cleared",
    )
}

#[derive(Default)]
struct SlotState {
    pending: Option<Value>,
    /// The newest notification offered, so one that arrives late (calls of
    /// the function can overtake each other) is dropped.
    newest: Option<(i64, u64, bool)>,
    in_flight: bool,
    closed: bool,
}

/// One observer's coalescing mailbox: one pending notification, one call
/// in flight.
pub struct DeliverySlot {
    target: Arc<dyn FrameTarget>,
    state: Mutex<SlotState>,
    delivered: AtomicU64,
    coalesced: AtomicU64,
}

impl DeliverySlot {
    fn new(target: Arc<dyn FrameTarget>) -> Arc<Self> {
        Arc::new(Self {
            target,
            state: Mutex::new(SlotState::default()),
            delivered: AtomicU64::new(0),
            coalesced: AtomicU64::new(0),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SlotState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Offer a notification. Replaces any pending one; starts the single
    /// drain task only when none is running. One older than what this slot
    /// already saw is dropped.
    fn offer(self: &Arc<Self>, payload: Value) {
        let order = order_of(&payload);
        let start = {
            let mut st = self.lock();
            if st.closed || st.newest.is_some_and(|newest| newest >= order) {
                return;
            }
            st.newest = Some(order);
            if st.pending.replace(payload).is_some() {
                self.coalesced.fetch_add(1, Ordering::Relaxed);
            }
            !std::mem::replace(&mut st.in_flight, true)
        };
        if start {
            let slot = self.clone();
            tokio::spawn(async move { slot.drain().await });
        }
    }

    async fn drain(self: Arc<Self>) {
        loop {
            let next = {
                let mut st = self.lock();
                match (st.closed, st.pending.take()) {
                    (false, Some(payload)) => payload,
                    _ => {
                        st.pending = None;
                        st.in_flight = false;
                        return;
                    }
                }
            };
            match self.target.deliver(next).await {
                Ok(()) => {
                    self.delivered.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    tracing::debug!(observer_id = %self.target.id(), error = %e, "frame change not delivered");
                }
            }
        }
    }

    fn close(&self) {
        let mut st = self.lock();
        st.closed = true;
        st.pending = None;
    }

    /// Notifications waiting in this slot: 0 or 1, by construction.
    pub fn pending_len(&self) -> usize {
        usize::from(self.lock().pending.is_some())
    }

    pub fn in_flight(&self) -> bool {
        self.lock().in_flight
    }

    pub fn delivered(&self) -> u64 {
        self.delivered.load(Ordering::Relaxed)
    }

    pub fn coalesced(&self) -> u64 {
        self.coalesced.load(Ordering::Relaxed)
    }
}

struct FrameObserver {
    session_id: String,
    slot: Arc<DeliverySlot>,
}

/// The observers of `computer::on-frame-changed` (by observer id, with the
/// session each follows) and their delivery slots.
pub struct FrameNotifier {
    observers: Mutex<HashMap<String, FrameObserver>>,
    /// Calls the function, and answers [`watching`](Self::watching) from its
    /// observers. `None` (tests): a change goes straight to the slots of its
    /// session, as the deliver hook hands it out.
    iii: Option<IIIClient>,
}

impl FrameNotifier {
    /// Changes are calls of `computer::on-frame-changed` on `iii`.
    pub fn new(iii: IIIClient) -> Arc<Self> {
        Arc::new(Self {
            observers: Mutex::new(HashMap::new()),
            iii: Some(iii),
        })
    }

    /// No engine: [`notify`](Self::notify) offers each change to the slots
    /// of its session itself.
    pub fn detached() -> Arc<Self> {
        Arc::new(Self {
            observers: Mutex::new(HashMap::new()),
            iii: None,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, FrameObserver>> {
        self.observers.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Open the slot of an observer of `session_id`. Rejects anything past
    /// [`MAX_FRAME_OBSERVERS`] (`OBSERVER_LIMIT`).
    pub fn open(
        &self,
        session_id: String,
        target: Arc<dyn FrameTarget>,
    ) -> Result<(), ObserveRejection> {
        let id = target.id().to_string();
        let mut map = self.lock();
        if !map.contains_key(&id) && map.len() >= MAX_FRAME_OBSERVERS {
            return Err(ObserveRejection::new(
                "OBSERVER_LIMIT",
                format!("{ON_FRAME_CHANGED_FN}: observer limit ({MAX_FRAME_OBSERVERS}) reached"),
            ));
        }
        let slot = DeliverySlot::new(target);
        if let Some(old) = map.insert(id, FrameObserver { session_id, slot }) {
            old.slot.close();
        }
        Ok(())
    }

    /// Close an observer's slot; its pending notification is dropped.
    pub fn close(&self, observer_id: &str) {
        if let Some(observer) = self.lock().remove(observer_id) {
            observer.slot.close();
        }
    }

    pub fn observer_count(&self) -> usize {
        self.lock().len()
    }

    /// Some console viewer follows this session's frames: an active observer
    /// is filed under its id (one atomic load while nothing observes, then
    /// one lookup).
    pub fn watching(&self, session_id: &str) -> bool {
        let Some(iii) = &self.iii else {
            return self.lock().values().any(|o| o.session_id == session_id);
        };
        if !iii.is_observed(ON_FRAME_CHANGED_FN) {
            return false;
        }
        let call = Call {
            input: json!({ "session_id": session_id }),
            ..Call::default()
        };
        iii.is_observed_for(ON_FRAME_CHANGED_FN, &call)
    }

    /// The slot of one observer (tests and diagnostics).
    pub fn slot(&self, observer_id: &str) -> Option<Arc<DeliverySlot>> {
        self.lock().get(observer_id).map(|o| o.slot.clone())
    }

    /// Publish `change` when some observer follows its session. Never
    /// blocks on a consumer: the slots coalesce and each one's single drain
    /// task delivers.
    pub fn notify(&self, change: &FrameChange) {
        if !self.watching(&change.session_id) {
            return;
        }
        let payload = match serde_json::to_value(change) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "frame change failed to serialize");
                return;
            }
        };
        let Some(iii) = self.iii.clone() else {
            let ids: Vec<String> = self
                .lock()
                .iter()
                .filter(|(_, o)| o.session_id == change.session_id)
                .map(|(id, _)| id.clone())
                .collect();
            self.offer(ids.iter().map(String::as_str), &payload);
            return;
        };
        tokio::spawn(async move {
            let request = CallRequest {
                function_id: ON_FRAME_CHANGED_FN.to_string(),
                payload,
                action: Some(CallAction::Void),
                timeout_ms: None,
            };
            if let Err(e) = iii.call(request).await {
                tracing::debug!(error = %e, "computer::on-frame-changed call failed");
            }
        });
    }

    /// Offer `payload` to the slots of `observer_ids`; returns how many it
    /// reached.
    pub fn offer<'a>(
        &self,
        observer_ids: impl IntoIterator<Item = &'a str>,
        payload: &Value,
    ) -> usize {
        let slots: Vec<Arc<DeliverySlot>> = {
            let map = self.lock();
            observer_ids
                .into_iter()
                .filter_map(|id| map.get(id).map(|o| o.slot.clone()))
                .collect()
        };
        for slot in &slots {
            slot.offer(payload.clone());
        }
        slots.len()
    }
}

/// Register `computer::on-frame-changed` with its hooks and return the
/// notifier the screencast pump feeds.
pub fn register(iii: &IIIClient) -> Arc<FrameNotifier> {
    let notifier = FrameNotifier::new(iii.clone());

    // Hooks by id before registering, so they see every observer.
    iii.on_observer_added_for(ON_FRAME_CHANGED_FN, {
        let (iii, notifier) = (iii.clone(), notifier.clone());
        move |observer: Observer| {
            let (iii, notifier) = (iii.clone(), notifier.clone());
            async move {
                let args: FrameArgs = observer.args()?;
                let session_id = followed_session(&args)
                    .map_err(|message| ObserveRejection::new("INVALID_ARGS", message))?;
                tracing::info!(observer_id = %observer.id(), session = %session_id, "frame observer added");
                notifier.open(session_id, Arc::new(ObserverTarget::new(iii, observer)))
            }
        }
    });
    iii.on_observer_removed_for(ON_FRAME_CHANGED_FN, {
        let notifier = notifier.clone();
        move |observer: Observer| {
            let notifier = notifier.clone();
            async move {
                notifier.close(observer.id());
                tracing::info!(observer_id = %observer.id(), "frame observer removed");
            }
        }
    });
    // A change goes to the observers of its session: one lookup.
    iii.on_observer_match_for(
        ON_FRAME_CHANGED_FN,
        Match::keys(
            |observer| {
                observer.args_value()["session_id"]
                    .as_str()
                    .map(str::trim)
                    .into()
            },
            |call| call.input["session_id"].as_str().into(),
        ),
    );
    iii.on_observer_deliver_for(ON_FRAME_CHANGED_FN, {
        let notifier = notifier.clone();
        move |call: Call, observers: Vec<Observer>| {
            let notifier = notifier.clone();
            async move {
                let payload = call.output.as_ref().unwrap_or(&call.input);
                let matching = observers
                    .iter()
                    .filter(|observer| observer.matches(&call))
                    .map(Observer::id);
                let offered = notifier.offer(matching, payload);
                Ok::<_, Error>(json!({ "offered": offered }))
            }
        }
    });
    iii.register_function(
        ON_FRAME_CHANGED_FN,
        RegisterFunction::new(|change: FrameChange| Ok::<_, Error>(change))
            .description(ON_FRAME_CHANGED_DESC)
            .args_format::<FrameArgs>()
            .private(),
    );
    notifier
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records deliveries; each delivery takes `delay` (a slow consumer).
    pub(crate) struct Recorder {
        pub seen: Mutex<Vec<(String, Value)>>,
        pub delay: Duration,
    }

    /// One observer whose deliveries the shared [`Recorder`] records.
    struct Viewer {
        id: String,
        recorder: Arc<Recorder>,
    }

    #[async_trait]
    impl FrameTarget for Viewer {
        fn id(&self) -> &str {
            &self.id
        }

        async fn deliver(&self, payload: Value) -> Result<(), String> {
            tokio::time::sleep(self.recorder.delay).await;
            self.recorder
                .seen
                .lock()
                .unwrap()
                .push((self.id.clone(), payload));
            Ok(())
        }
    }

    fn recorder(delay_ms: u64) -> Arc<Recorder> {
        Arc::new(Recorder {
            seen: Mutex::new(Vec::new()),
            delay: Duration::from_millis(delay_ms),
        })
    }

    fn open(n: &FrameNotifier, rec: &Arc<Recorder>, id: &str, session_id: &str) {
        n.open(
            session_id.to_string(),
            Arc::new(Viewer {
                id: id.to_string(),
                recorder: rec.clone(),
            }),
        )
        .unwrap();
    }

    fn change(session: &str, seq: u64) -> FrameChange {
        FrameChange {
            session_id: session.to_string(),
            epoch: 7,
            frame_seq: seq,
            change: FrameChangeKind::Updated,
            width: 1280,
            height: 800,
            mime: Some("image/jpeg".to_string()),
            bytes: Some(100_000),
            timestamp: 0,
        }
    }

    async fn settle(notifier: &FrameNotifier, ids: &[&str]) {
        for _ in 0..500 {
            if ids
                .iter()
                .all(|id| notifier.slot(id).map(|s| !s.in_flight()).unwrap_or(true))
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("slots did not settle");
    }

    #[test]
    fn args_require_a_non_empty_session_id_and_reject_unknown_keys() {
        let parse = |v: Value| serde_json::from_value::<FrameArgs>(v);
        assert!(parse(Value::Null).is_err());
        assert!(parse(json!({})).is_err());
        assert!(parse(json!({ "session_id": "c1", "stream_name": "x" })).is_err());
        let blank = parse(json!({ "session_id": "  " })).unwrap();
        let err = followed_session(&blank).unwrap_err();
        assert!(err.contains(ON_FRAME_CHANGED_FN), "{err}");
        let padded = parse(json!({ "session_id": " c1 " })).unwrap();
        assert_eq!(followed_session(&padded).unwrap(), "c1");
        let schema = serde_json::to_value(schemars::schema_for!(FrameArgs)).unwrap();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], json!(["session_id"]));
    }

    #[test]
    fn observer_count_is_capped() {
        let rec = recorder(0);
        let n = FrameNotifier::detached();
        for i in 0..MAX_FRAME_OBSERVERS {
            open(&n, &rec, &format!("o{i}"), "c1");
        }
        let err = n
            .open(
                "c1".to_string(),
                Arc::new(Viewer {
                    id: "one-too-many".to_string(),
                    recorder: rec.clone(),
                }),
            )
            .unwrap_err();
        assert_eq!(err.code, "OBSERVER_LIMIT");
        // Opening an existing id again replaces its slot, not a new observer.
        open(&n, &rec, "o0", "c2");
        assert_eq!(n.observer_count(), MAX_FRAME_OBSERVERS);
        n.close("o1");
        open(&n, &rec, "one-more", "c1");
        assert_eq!(n.observer_count(), MAX_FRAME_OBSERVERS);
    }

    #[tokio::test]
    async fn changes_reach_only_the_observers_of_their_session() {
        let rec = recorder(0);
        let n = FrameNotifier::detached();
        open(&n, &rec, "one", "c1");
        open(&n, &rec, "two", "c2");
        assert!(n.watching("c1") && n.watching("c2") && !n.watching("c3"));
        n.notify(&change("c1", 1));
        settle(&n, &["one", "two"]).await;
        let seen = rec.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        let (id, payload) = &seen[0];
        assert_eq!(id, "one");
        assert_eq!(payload["session_id"], "c1");
        assert_eq!(payload["frame_seq"], 1);
        assert_eq!(payload["change"], "updated");
        // Image-free: the payload stays small regardless of frame size.
        assert!(payload.get("data").is_none() && payload.get("frame").is_none());
        assert!(payload.to_string().len() < 512);
    }

    #[tokio::test]
    async fn slow_consumer_is_coalesced_to_the_latest_with_bounded_memory() {
        // Each delivery takes 50ms; 1000 frames arrive back to back.
        let rec = recorder(50);
        let n = FrameNotifier::detached();
        open(&n, &rec, "slow", "c1");
        let slot = n.slot("slow").unwrap();
        let mut max_pending = 0;
        for seq in 1..=1000 {
            n.notify(&change("c1", seq));
            max_pending = max_pending.max(slot.pending_len());
        }
        assert!(max_pending <= 1, "pending grew to {max_pending}");
        settle(&n, &["slow"]).await;
        let seqs: Vec<u64> = rec
            .seen
            .lock()
            .unwrap()
            .iter()
            .map(|(_, p)| p["frame_seq"].as_u64().unwrap())
            .collect();
        // The first frame went out immediately, everything after it was
        // coalesced into one pending slot: the consumer saw the newest.
        assert!(seqs.len() <= 3, "delivered {seqs:?}");
        assert_eq!(*seqs.last().unwrap(), 1000);
        assert!(seqs.windows(2).all(|w| w[0] < w[1]), "{seqs:?}");
        assert_eq!(slot.delivered() as usize, seqs.len());
        assert!(slot.coalesced() >= 997, "coalesced {}", slot.coalesced());
        assert_eq!(slot.pending_len(), 0);
    }

    #[tokio::test]
    async fn a_change_older_than_the_newest_offered_is_dropped() {
        let rec = recorder(0);
        let n = FrameNotifier::detached();
        open(&n, &rec, "v", "c1");
        let payload = |seq: u64, kind: FrameChangeKind| {
            let mut c = change("c1", seq);
            c.change = kind;
            serde_json::to_value(c).unwrap()
        };
        n.offer(["v"], &payload(2, FrameChangeKind::Updated));
        settle(&n, &["v"]).await;
        n.offer(["v"], &payload(1, FrameChangeKind::Updated)); // overtaken
        n.offer(["v"], &payload(2, FrameChangeKind::Cleared)); // the clear of frame 2
        n.offer(["v"], &payload(2, FrameChangeKind::Updated)); // before its clear
        settle(&n, &["v"]).await;
        let seen: Vec<(u64, String)> = rec
            .seen
            .lock()
            .unwrap()
            .iter()
            .map(|(_, p)| {
                (
                    p["frame_seq"].as_u64().unwrap(),
                    p["change"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        assert_eq!(
            seen,
            vec![(2, "updated".to_string()), (2, "cleared".to_string())]
        );
    }

    #[tokio::test]
    async fn closing_drops_the_pending_notification() {
        let rec = recorder(30);
        let n = FrameNotifier::detached();
        open(&n, &rec, "gone", "c1");
        let slot = n.slot("gone").unwrap();
        n.notify(&change("c1", 1));
        tokio::time::sleep(Duration::from_millis(5)).await; // 1 is now in flight
        n.notify(&change("c1", 2)); // pending
        n.close("gone");
        assert_eq!(n.observer_count(), 0);
        assert_eq!(slot.pending_len(), 0);
        n.notify(&change("c1", 3)); // nobody observes
        tokio::time::sleep(Duration::from_millis(80)).await;
        let seqs: Vec<u64> = rec
            .seen
            .lock()
            .unwrap()
            .iter()
            .map(|(_, p)| p["frame_seq"].as_u64().unwrap())
            .collect();
        assert_eq!(seqs, vec![1]);
        assert!(!slot.in_flight());
    }
}
