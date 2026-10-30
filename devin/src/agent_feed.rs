//! Owned event feeds: `devin::on-agent-event` carries the translated
//! AgentEvent frames and `devin::on-raw-event` the raw Devin CLI stdout lines
//! (`{ type: "stdout", line }`), verbatim. Both are private functions the worker registers at startup; each
//! handler returns the frame envelope it is called with, which is what
//! observers observe. Nothing goes through a stream.
//!
//! Contract (shared by every agent worker's feed):
//! - observer args `{ session_id }`: required, non-empty, at most 512 chars
//!   ([`FeedArgs`]); the engine refuses any other args with `INVALID_ARGS`.
//!   At most [`MAX_OBSERVERS`] observers per function; one more is rejected
//!   with [`OBSERVER_LIMIT`]. Metadata a consumer wants back goes in its
//!   observe options and reaches its target as the metadata sidecar.
//! - event `{ session_id, event_id, seq, epoch, source, event }`
//!   ([`FeedPayload`]): `event` is the frame unchanged, `seq` is contiguous
//!   per (feed, session, epoch) starting at 0 with a separate counter per
//!   feed, `epoch` is a per-process uuid, `event_id` =
//!   `<session_id>-<epoch>-<seq:08>` and `source` = [`SOURCE`].
//! - each frame is one fire-and-forget (Void) call of the feed's function.
//!   Observers are matched by key: an observer is filed under its
//!   `session_id`, a frame matches the observers filed under the frame's, and
//!   iii delivers it to them. A frame of a session nobody observes is never
//!   built: it costs one hash lookup and makes no call; its `seq` still
//!   advances. A failed call is logged and swallowed, so a turn never fails
//!   because a consumer is gone.
//! - ephemeral: frames are not stored or replayed. Consumers order by
//!   `(epoch, seq)` and dedup by `event_id`; history lives in the session
//!   record and the run result.
//!
//! The counters and the observer count live behind a `std::sync::Mutex`,
//! never held across an await.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use iii_sdk::errors::Error;
use iii_sdk::{
    Call, CallAction, CallRequest, IIIClient, Match, ObserveRejection, Observer, RegisterFunction,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Producer namespace stamped on every frame as `source`.
pub const SOURCE: &str = "devin";
/// The function carrying the translated AgentEvent frames.
pub const ON_AGENT_EVENT_FN: &str = "devin::on-agent-event";
/// The function carrying the raw Devin CLI stdout lines, verbatim.
pub const ON_RAW_EVENT_FN: &str = "devin::on-raw-event";
/// Observers accepted per function; one more is rejected.
pub const MAX_OBSERVERS: usize = 256;
/// Longest accepted `session_id`, in characters.
pub const MAX_SESSION_ID_CHARS: usize = 512;
/// Rejection code of an observer past [`MAX_OBSERVERS`].
pub const OBSERVER_LIMIT: &str = "OBSERVER_LIMIT";

const AGENT_EVENT_DESCRIPTION: &str = "Translated AgentEvent frames of one devin session. \
     Args: { session_id }. Event: { session_id, event_id, seq, epoch, source, event }. \
     Ephemeral (not stored, not replayed); order by (epoch, seq).";
const RAW_EVENT_DESCRIPTION: &str = "Raw Devin CLI stdout lines ({ type: stdout, line }, \
     verbatim) of one devin session. Args: { session_id }. Event: { session_id, event_id, seq, \
     epoch, source, event }. Ephemeral (not stored, not replayed); order by (epoch, seq).";

/// The args an observer of either feed passes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FeedArgs {
    /// The iii session whose frames this observer receives.
    #[schemars(length(min = 1, max = 512))]
    pub session_id: String,
}

/// One frame as observers observe it: the feed function's input and output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FeedPayload {
    pub session_id: String,
    /// `<session_id>-<epoch>-<seq:08>`: dedup key.
    pub event_id: String,
    /// Contiguous per (feed, session, epoch), from 0.
    pub seq: u64,
    /// Per-process uuid; a restart starts a new epoch.
    pub epoch: String,
    /// The producing worker.
    pub source: String,
    /// The frame, unchanged.
    pub event: Value,
}

/// The session a feed observer is filed under, or a frame matches: the
/// `session_id` of its args or of the frame.
pub fn session_key(value: &Value) -> Option<&str> {
    value.get("session_id").and_then(Value::as_str)
}

/// A call of a feed for `session_id`, as far as matching reads it: what
/// [`IIIClient::is_observed_for`] looks up before a frame is built.
pub fn session_call(session_id: &str) -> Call {
    Call {
        input: serde_json::json!({ "session_id": session_id }),
        ..Call::default()
    }
}

#[derive(Default)]
struct FeedState {
    observers: HashSet<String>,
    seq_by_session: HashMap<String, u64>,
}

/// One owned feed function: its per-session sequence counters and the ids of
/// its accepted observers.
pub struct Feed {
    id: String,
    source: String,
    epoch: String,
    state: Mutex<FeedState>,
}

impl Feed {
    pub fn new(id: impl Into<String>, source: impl Into<String>, epoch: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            epoch: epoch.into(),
            state: Mutex::new(FeedState::default()),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    fn lock(&self) -> MutexGuard<'_, FeedState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Count an observer being added; past [`MAX_OBSERVERS`] a new id is
    /// rejected with [`OBSERVER_LIMIT`]. An id already counted (the engine
    /// re-announcing it) is accepted again.
    pub fn admit(&self, observer_id: &str) -> Result<(), ObserveRejection> {
        let mut state = self.lock();
        if !state.observers.contains(observer_id) && state.observers.len() >= MAX_OBSERVERS {
            return Err(ObserveRejection::new(
                OBSERVER_LIMIT,
                format!("{}: observer limit ({MAX_OBSERVERS}) reached", self.id),
            ));
        }
        state.observers.insert(observer_id.to_string());
        Ok(())
    }

    /// Stop counting a removed observer; true when it was counted.
    pub fn release(&self, observer_id: &str) -> bool {
        self.lock().observers.remove(observer_id)
    }

    pub fn observer_count(&self) -> usize {
        self.lock().observers.len()
    }

    /// Assign the next sequence number of `session_id` on this feed. Runs for
    /// every frame, observed or not, so `seq` stays contiguous per
    /// (feed, session, epoch).
    pub fn next_seq(&self, session_id: &str) -> u64 {
        let mut state = self.lock();
        let counter = state
            .seq_by_session
            .entry(session_id.to_string())
            .or_insert(0);
        let seq = *counter;
        *counter += 1;
        seq
    }

    /// The envelope of frame `seq` of `session_id`.
    pub fn payload(&self, session_id: &str, seq: u64, event: Value) -> FeedPayload {
        FeedPayload {
            session_id: session_id.to_string(),
            event_id: format!("{session_id}-{}-{seq:08}", self.epoch),
            seq,
            epoch: self.epoch.clone(),
            source: self.source.clone(),
            event,
        }
    }

    /// Emit one frame: advance the session's `seq`, ask `observed` whether a
    /// call for the session would reach an observer, and only then build the
    /// frame with `event` and the envelope around it and `send` it. A frame
    /// nobody observes is never built. A failed send is logged and swallowed.
    /// True when the call was made and succeeded.
    pub async fn emit_with<E, O, S, Fut>(
        &self,
        session_id: &str,
        event: E,
        observed: O,
        send: S,
    ) -> bool
    where
        E: FnOnce() -> Value,
        O: FnOnce(&str) -> bool,
        S: FnOnce(FeedPayload) -> Fut,
        Fut: Future<Output = Result<Value, Error>>,
    {
        let seq = self.next_seq(session_id);
        if !observed(session_id) {
            return false;
        }
        match send(self.payload(session_id, seq, event())).await {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!(
                    function_id = %self.id,
                    session_id,
                    error = %e,
                    "event feed call failed"
                );
                false
            }
        }
    }

    /// Emit one frame through the engine: one Void call of this feed's
    /// function, made only when an observer of the session exists. `event`
    /// builds the frame and runs only then.
    pub async fn emit(
        &self,
        iii: &IIIClient,
        session_id: &str,
        event: impl FnOnce() -> Value,
    ) -> bool {
        self.emit_with(
            session_id,
            event,
            |session_id| {
                iii.is_observed(&self.id)
                    && iii.is_observed_for(&self.id, &session_call(session_id))
            },
            |payload| iii.call(self.request(payload)),
        )
        .await
    }

    /// The fire-and-forget (Void) call of this feed's function with `payload`.
    pub fn request(&self, payload: FeedPayload) -> CallRequest {
        CallRequest {
            function_id: self.id.clone(),
            payload: serde_json::to_value(payload).expect("a feed payload serializes"),
            action: Some(CallAction::Void),
            timeout_ms: None,
        }
    }

    /// Set this feed's hooks by id, then register its function: private,
    /// `{ session_id }` args, and a handler that returns the envelope.
    fn register(self: &Arc<Self>, iii: &IIIClient, description: &'static str) {
        let feed = Arc::clone(self);
        iii.on_observer_added_for(&self.id, move |obs: Observer| {
            let feed = Arc::clone(&feed);
            async move { feed.admit(obs.id()) }
        });
        let feed = Arc::clone(self);
        iii.on_observer_removed_for(&self.id, move |obs: Observer| {
            let feed = Arc::clone(&feed);
            async move {
                feed.release(obs.id());
            }
        });
        iii.on_observer_match_for(
            &self.id,
            Match::keys(
                |obs| session_key(obs.args_value()).into(),
                |call| session_key(&call.input).into(),
            ),
        );
        iii.register_function(
            self.id.clone(),
            RegisterFunction::new(|payload: FeedPayload| Ok::<_, Error>(payload))
                .description(description)
                .args_format::<FeedArgs>()
                .private(),
        );
    }
}

/// The worker's two feeds; both share the process epoch, each keeps its own
/// counters.
pub struct Feeds {
    pub agent: Arc<Feed>,
    pub raw: Arc<Feed>,
}

impl Feeds {
    pub fn new(epoch: &str) -> Self {
        Self {
            agent: Arc::new(Feed::new(ON_AGENT_EVENT_FN, SOURCE, epoch)),
            raw: Arc::new(Feed::new(ON_RAW_EVENT_FN, SOURCE, epoch)),
        }
    }

    /// Register both feed functions and their hooks on `iii`.
    pub fn register(&self, iii: &IIIClient) {
        self.agent.register(iii, AGENT_EVENT_DESCRIPTION);
        self.raw.register(iii, RAW_EVENT_DESCRIPTION);
    }
}

/// The process-wide feeds (epoch = a uuid minted once per process).
pub fn feeds() -> &'static Feeds {
    static FEEDS: OnceLock<Feeds> = OnceLock::new();
    FEEDS.get_or_init(|| Feeds::new(&uuid::Uuid::new_v4().to_string()))
}

/// Register the process-wide feed functions and their hooks. Call once at
/// startup.
pub fn register(iii: &IIIClient) {
    feeds().register(iii);
}

/// Emit a translated AgentEvent frame on `devin::on-agent-event`.
/// `event` builds the frame and runs only when the session is observed.
pub async fn emit_agent_event(
    iii: &IIIClient,
    session_id: &str,
    event: impl FnOnce() -> Value,
) -> bool {
    feeds().agent.emit(iii, session_id, event).await
}

/// Emit a raw CLI frame on `devin::on-raw-event`.
/// `event` builds the frame and runs only when the session is observed.
pub async fn emit_raw_event(
    iii: &IIIClient,
    session_id: &str,
    event: impl FnOnce() -> Value,
) -> bool {
    feeds().raw.emit(iii, session_id, event).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const EPOCH: &str = "epoch-test";

    /// Emit with every session in `observed` observed; returns
    /// (sent, captured envelopes).
    async fn capture(
        feed: &Feed,
        observed: &[&str],
        session_id: &str,
        event: Value,
    ) -> (bool, Vec<FeedPayload>) {
        let sent = Mutex::new(Vec::new());
        let ok = feed
            .emit_with(
                session_id,
                || event,
                |session_id| observed.contains(&session_id),
                |payload| {
                    sent.lock().unwrap().push(payload);
                    std::future::ready(Ok::<Value, Error>(Value::Null))
                },
            )
            .await;
        (ok, sent.into_inner().unwrap())
    }

    #[tokio::test]
    async fn an_observed_session_gets_one_void_call_with_the_envelope() {
        let feeds = Feeds::new(EPOCH);
        let event = json!({ "type": "message_complete", "message": { "role": "assistant" } });
        let (ok, sent) = capture(&feeds.agent, &["s1"], "s1", event.clone()).await;
        assert!(ok);
        assert_eq!(
            serde_json::to_value(&sent[0]).unwrap(),
            json!({
                "session_id": "s1",
                "event_id": format!("s1-{EPOCH}-00000000"),
                "seq": 0,
                "epoch": EPOCH,
                "source": SOURCE,
                "event": event,
            })
        );

        let request = feeds.agent.request(sent[0].clone());
        assert_eq!(request.function_id, ON_AGENT_EVENT_FN);
        assert!(matches!(request.action, Some(CallAction::Void)));
        assert_eq!(request.payload["event_id"], format!("s1-{EPOCH}-00000000"));
    }

    #[tokio::test]
    async fn a_session_nobody_observes_makes_no_call_but_advances_seq() {
        let feed = Feed::new(ON_AGENT_EVENT_FN, SOURCE, EPOCH);
        let (ok, sent) = capture(&feed, &["A"], "B", json!({ "type": "agent_end" })).await;
        assert!(!ok);
        assert!(sent.is_empty(), "no observer, no call");
        let (_, sent) = capture(&feed, &["A", "B"], "B", json!({ "type": "agent_end" })).await;
        assert_eq!(sent[0].seq, 1);
        assert_eq!(sent[0].session_id, "B");
    }

    #[tokio::test]
    async fn a_frame_nobody_observes_is_never_built() {
        let feed = Feed::new(ON_RAW_EVENT_FN, SOURCE, EPOCH);
        let built = std::cell::Cell::new(false);
        let ok = feed
            .emit_with(
                "B",
                || {
                    built.set(true);
                    json!({ "type": "raw" })
                },
                |_| false,
                |_| std::future::ready(Ok::<Value, Error>(Value::Null)),
            )
            .await;
        assert!(!ok);
        assert!(
            !built.get(),
            "the frame of an unobserved session is not built"
        );
        let (_, sent) = capture(&feed, &["B"], "B", json!({ "type": "raw" })).await;
        assert_eq!(sent[0].seq, 1, "the unobserved frame still took seq 0");
    }

    #[test]
    fn observers_and_frames_are_keyed_by_session_id() {
        assert_eq!(session_key(&json!({ "session_id": "s1" })), Some("s1"));
        assert_eq!(session_key(&json!({})), None);
        assert_eq!(session_key(&json!({ "session_id": 7 })), None);
        assert_eq!(session_key(&session_call("s2").input), Some("s2"));
    }

    #[test]
    fn args_format_requires_a_bounded_session_id_and_nothing_else() {
        let schema = serde_json::to_value(schemars::schema_for!(FeedArgs)).unwrap();
        assert_eq!(schema["required"], json!(["session_id"]));
        assert_eq!(schema["additionalProperties"], json!(false));
        let session_id = &schema["properties"]["session_id"];
        assert_eq!(session_id["type"], "string");
        assert_eq!(session_id["minLength"], 1);
        assert_eq!(session_id["maxLength"], MAX_SESSION_ID_CHARS);
        assert!(serde_json::from_value::<FeedArgs>(json!({ "session_id": "s", "x": 1 })).is_err());
    }

    #[test]
    fn the_observer_limit_rejects_new_ids_and_frees_on_release() {
        let feed = Feed::new(ON_AGENT_EVENT_FN, SOURCE, EPOCH);
        for i in 0..MAX_OBSERVERS {
            feed.admit(&format!("o{i}")).unwrap();
        }
        assert_eq!(feed.observer_count(), MAX_OBSERVERS);
        let err = feed.admit("overflow").unwrap_err();
        assert_eq!(err.code, OBSERVER_LIMIT);
        assert!(err.message.contains("observer limit (256)"), "{err}");
        // The engine re-announcing a counted observer is accepted.
        feed.admit("o0").unwrap();
        assert_eq!(feed.observer_count(), MAX_OBSERVERS);
        assert!(feed.release("o0"));
        assert!(!feed.release("o0"));
        feed.admit("overflow").unwrap();
        assert_eq!(feed.observer_count(), MAX_OBSERVERS);
    }

    #[tokio::test]
    async fn seq_is_contiguous_per_feed_and_session() {
        let feeds = Feeds::new(EPOCH);
        let observed = ["s1", "s2"];
        let mut agent_s1 = Vec::new();
        let mut raw_s1 = Vec::new();
        for _ in 0..3 {
            raw_s1.extend(
                capture(&feeds.raw, &observed, "s1", json!({ "type": "raw" }))
                    .await
                    .1,
            );
            agent_s1.extend(
                capture(&feeds.agent, &observed, "s1", json!({ "type": "a" }))
                    .await
                    .1,
            );
        }
        raw_s1.extend(
            capture(&feeds.raw, &observed, "s1", json!({ "type": "raw" }))
                .await
                .1,
        );
        let agent_s2 = capture(&feeds.agent, &observed, "s2", json!({ "type": "a" }))
            .await
            .1;

        let seqs = |sent: &[FeedPayload]| -> Vec<u64> { sent.iter().map(|p| p.seq).collect() };
        assert_eq!(seqs(&agent_s1), vec![0, 1, 2]);
        assert_eq!(seqs(&raw_s1), vec![0, 1, 2, 3]);
        assert_eq!(seqs(&agent_s2), vec![0]);
        assert_eq!(agent_s1[2].event_id, format!("s1-{EPOCH}-00000002"));
        assert_eq!(raw_s1[3].event_id, format!("s1-{EPOCH}-00000003"));
        assert_eq!(agent_s1[0].epoch, raw_s1[0].epoch);
    }

    #[tokio::test]
    async fn a_failing_call_is_swallowed_and_the_next_frame_flows() {
        let feed = Feed::new(ON_AGENT_EVENT_FN, SOURCE, EPOCH);
        let ok = feed
            .emit_with(
                "s",
                || json!({ "type": "agent_end" }),
                |_| true,
                |_| std::future::ready(Err::<Value, Error>(Error::NotConnected)),
            )
            .await;
        assert!(!ok);
        let (ok, sent) = capture(&feed, &["s"], "s", json!({ "type": "x" })).await;
        assert!(ok);
        assert_eq!(sent[0].seq, 1);
    }

    #[test]
    fn process_feeds_use_the_owned_function_ids() {
        let feeds = feeds();
        assert_eq!(feeds.agent.id(), "devin::on-agent-event");
        assert_eq!(feeds.raw.id(), "devin::on-raw-event");
        assert_eq!(feeds.agent.epoch(), feeds.raw.epoch());
        assert!(uuid::Uuid::parse_str(feeds.agent.epoch()).is_ok());
    }
}
