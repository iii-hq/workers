//! Consumer side of the agent event feeds (`<namespace>::agent-event`).
//!
//! Every agent worker (claude-code, pi, opencode, cursor, codex, grok, devin,
//! hermes, or any brain that follows the same contract) owns a trigger type
//! `<namespace>::agent-event`. A binding is `{ session_id }` and each delivery
//! is
//!
//! ```json
//! { "session_id": "sess_..", "event_id": "..", "seq": 7, "epoch": "..",
//!   "source": "claude", "event": { "type": "message_complete", .. } }
//! ```
//!
//! `seq` is contiguous per `(session_id, epoch)` and starts at 0. Deliveries
//! are fire-and-forget and the SDK runs handlers concurrently, so frames can
//! arrive out of order or twice; [`Sequencer`] restores the order per
//! `(session_id, source, epoch)` with a bounded reorder window.
//!
//! The feed is ephemeral: nothing is replayed. acp's own history (state scope
//! `acp-v0.3`) and the brain's synchronous response stay the source of truth.

use std::collections::{BTreeMap, HashMap};

use serde_json::{Value, json};

/// How long an out-of-order frame waits for the missing predecessor before
/// the gap is skipped.
pub const REORDER_WINDOW_MS: u64 = 250;
/// Upper bound on buffered out-of-order frames per stream; overflow releases
/// the buffer in order instead of growing it.
pub const MAX_PENDING_PER_STREAM: usize = 256;
/// Suffix of every producer's normalized feed trigger type.
pub const AGENT_EVENT_SUFFIX: &str = "agent-event";

/// One delivery from a `<namespace>::agent-event` trigger.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedEvent {
    pub session_id: String,
    pub event_id: String,
    pub seq: u64,
    pub epoch: String,
    pub source: String,
    pub event: Value,
}

/// Parse a delivery. Returns `None` when the payload does not follow the
/// contract (missing session id, sequence, epoch or event).
pub fn parse_feed_payload(payload: &Value) -> Option<FeedEvent> {
    let session_id = payload.get("session_id")?.as_str()?.to_string();
    if session_id.is_empty() {
        return None;
    }
    let seq = payload.get("seq")?.as_u64()?;
    let epoch = payload.get("epoch")?.as_str()?.to_string();
    let event = payload.get("event")?.clone();
    if !event.is_object() {
        return None;
    }
    let source = payload
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let event_id = payload
        .get("event_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("{session_id}-{epoch}-{seq:08}"));
    Some(FeedEvent {
        session_id,
        event_id,
        seq,
        epoch,
        source,
        event,
    })
}

/// The binding config acp registers per owned session.
pub fn binding_config(session_id: &str) -> Value {
    json!({ "session_id": session_id })
}

/// The feed a brain function's worker provides: `claude::run` ->
/// `claude::agent-event`. `None` for an id without a namespace.
pub fn default_events_trigger_type(brain_fn: &str) -> Option<String> {
    let (namespace, rest) = brain_fn.split_once("::")?;
    if namespace.is_empty() || rest.is_empty() {
        return None;
    }
    Some(format!("{namespace}::{AGENT_EVENT_SUFFIX}"))
}

/// Resolve the trigger types acp binds: the explicit list when given
/// (trimmed, empty entries dropped, deduplicated), else the brain's default.
pub fn resolve_events_trigger_types(explicit: &[String], brain_fn: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for entry in explicit {
        let entry = entry.trim();
        if !entry.is_empty() && !out.iter().any(|seen| seen == entry) {
            out.push(entry.to_string());
        }
    }
    if out.is_empty()
        && let Some(default) = brain_fn.and_then(default_events_trigger_type)
    {
        out.push(default);
    }
    out
}

/// Why an external brain cannot run yet, or `None` when the event feed is
/// ready. `missing` lists trigger types the engine does not know.
pub fn readiness_error(
    function_registered: bool,
    trigger_types: &[String],
    missing: &[String],
) -> Option<String> {
    if !function_registered {
        return Some(
            "iii-acp: the agent event handler failed to register at startup; external brain \
             updates would not reach the editor. Check the engine logs and restart iii-acp."
                .to_string(),
        );
    }
    if trigger_types.is_empty() {
        return Some(
            "iii-acp: no agent event trigger type is configured for the external brain; set \
             --events-trigger-type (IIIACP_EVENTS_TRIGGER_TYPE), e.g. claude::agent-event."
                .to_string(),
        );
    }
    if missing.is_empty() {
        return None;
    }
    let workers: Vec<String> = missing
        .iter()
        .map(|id| {
            let namespace = id.split_once("::").map(|(ns, _)| ns).unwrap_or(id.as_str());
            format!("`{id}` (provided by the `{namespace}` agent worker)")
        })
        .collect();
    Some(format!(
        "iii-acp: agent event trigger type {} is not registered with the engine; external \
         brain updates would not reach the editor. Start the agent worker that provides it \
         before retrying.",
        workers.join(", ")
    ))
}

/// True when an engine error means "this trigger type does not exist".
pub fn is_not_found_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("not_found") || message.contains("not found")
}

/// The final assistant text of a brain response, used only when no live frame
/// reached the editor during the prompt. Accepts `{ result: ".." }` (agent
/// workers' `run`) and `{ messages: [..] }` (turn-orchestrator shape).
pub fn final_text_from_brain_result(result: &Value) -> Option<String> {
    let result = result
        .get("value")
        .filter(|v| v.is_object())
        .unwrap_or(result);
    if result
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    if let Some(text) = result.get("result").and_then(Value::as_str)
        && !text.trim().is_empty()
    {
        return Some(text.to_string());
    }
    let message = result
        .get("messages")?
        .as_array()?
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))?;
    let content = message.get("content")?;
    if let Some(text) = content.as_str() {
        return (!text.trim().is_empty()).then(|| text.to_string());
    }
    let text = content
        .as_array()?
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("");
    (!text.trim().is_empty()).then_some(text)
}

#[derive(Default)]
struct StreamState {
    next: Option<u64>,
    pending: BTreeMap<u64, FeedEvent>,
    waiting_since_ms: Option<u64>,
}

impl StreamState {
    fn drain_contiguous(&mut self, out: &mut Vec<FeedEvent>) {
        while let Some(next) = self.next {
            let Some(event) = self.pending.remove(&next) else {
                break;
            };
            out.push(event);
            self.next = Some(next + 1);
        }
    }

    fn release_all(&mut self, out: &mut Vec<FeedEvent>) {
        let pending = std::mem::take(&mut self.pending);
        for (seq, event) in pending {
            out.push(event);
            self.next = Some(seq + 1);
        }
        self.waiting_since_ms = None;
    }

    fn settle_wait(&mut self, now_ms: u64) {
        if self.pending.is_empty() {
            self.waiting_since_ms = None;
        } else if self.waiting_since_ms.is_none() {
            self.waiting_since_ms = Some(now_ms);
        }
    }
}

/// Restores per-stream order and drops duplicates. One stream is
/// `(session_id, source, epoch)`.
#[derive(Default)]
pub struct Sequencer {
    streams: HashMap<(String, String, String), StreamState>,
}

impl Sequencer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Offer one delivery; returns the frames that are now ready, in order.
    /// Duplicates and frames older than what was already released are dropped.
    pub fn offer(&mut self, event: FeedEvent, now_ms: u64) -> Vec<FeedEvent> {
        let key = (
            event.session_id.clone(),
            event.source.clone(),
            event.epoch.clone(),
        );
        let state = self.streams.entry(key).or_default();
        let mut out = Vec::new();
        match state.next {
            Some(next) if event.seq < next => return out,
            _ if state.pending.contains_key(&event.seq) => return out,
            Some(next) if event.seq == next => {
                out.push(event);
                state.next = Some(next + 1);
                state.drain_contiguous(&mut out);
            }
            None if event.seq == 0 => {
                out.push(event);
                state.next = Some(1);
                state.drain_contiguous(&mut out);
            }
            _ => {
                state.pending.insert(event.seq, event);
                if state.pending.len() > MAX_PENDING_PER_STREAM {
                    state.release_all(&mut out);
                }
            }
        }
        state.settle_wait(now_ms);
        out
    }

    /// Release, in order, the buffered frames of `session_id` whose reorder
    /// window has expired (or all of them when `force`).
    pub fn flush(&mut self, session_id: &str, now_ms: u64, force: bool) -> Vec<FeedEvent> {
        let mut out = Vec::new();
        for ((sid, _, _), state) in self.streams.iter_mut() {
            if sid != session_id {
                continue;
            }
            let due = state
                .waiting_since_ms
                .is_some_and(|since| now_ms >= since + REORDER_WINDOW_MS);
            if force || due {
                state.release_all(&mut out);
            }
        }
        out
    }

    /// Earliest time a buffered frame of `session_id` becomes due.
    pub fn next_deadline(&self, session_id: &str) -> Option<u64> {
        self.streams
            .iter()
            .filter(|((sid, _, _), _)| sid == session_id)
            .filter_map(|(_, state)| state.waiting_since_ms)
            .min()
            .map(|since| since + REORDER_WINDOW_MS)
    }

    /// Drop all state for a session the connection no longer owns.
    pub fn forget_session(&mut self, session_id: &str) {
        self.streams.retain(|(sid, _, _), _| sid != session_id);
    }

    #[cfg(test)]
    fn stream_count(&self) -> usize {
        self.streams.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(session: &str, epoch: &str, seq: u64) -> FeedEvent {
        FeedEvent {
            session_id: session.to_string(),
            event_id: format!("{session}-{epoch}-{seq:08}"),
            seq,
            epoch: epoch.to_string(),
            source: "claude".to_string(),
            event: json!({ "type": "message_update", "n": seq }),
        }
    }

    fn seqs(events: &[FeedEvent]) -> Vec<u64> {
        events.iter().map(|e| e.seq).collect()
    }

    #[test]
    fn parses_the_feed_contract() {
        let payload = json!({
            "session_id": "sess_1",
            "event_id": "cursor-stable",
            "seq": 3,
            "epoch": "e1",
            "source": "cursor",
            "event": { "type": "message_complete" }
        });
        let parsed = parse_feed_payload(&payload).unwrap();
        assert_eq!(parsed.session_id, "sess_1");
        assert_eq!(parsed.event_id, "cursor-stable");
        assert_eq!(parsed.seq, 3);
        assert_eq!(parsed.source, "cursor");
        assert_eq!(parsed.event["type"], "message_complete");
    }

    #[test]
    fn synthesizes_event_id_and_rejects_partial_payloads() {
        let payload = json!({ "session_id": "s", "seq": 1, "epoch": "e", "event": {} });
        assert_eq!(
            parse_feed_payload(&payload).unwrap().event_id,
            "s-e-00000001"
        );
        // Legacy stream envelope is not this contract.
        let legacy = json!({ "groupId": "s", "id": "x", "event": { "data": {} } });
        assert!(parse_feed_payload(&legacy).is_none());
        assert!(
            parse_feed_payload(&json!({ "session_id": "", "seq": 0, "epoch": "e", "event": {} }))
                .is_none()
        );
        assert!(
            parse_feed_payload(&json!({ "session_id": "s", "epoch": "e", "event": {} })).is_none()
        );
    }

    #[test]
    fn in_order_frames_pass_straight_through() {
        let mut s = Sequencer::new();
        assert_eq!(seqs(&s.offer(ev("a", "e", 0), 0)), vec![0]);
        assert_eq!(seqs(&s.offer(ev("a", "e", 1), 0)), vec![1]);
        assert_eq!(seqs(&s.offer(ev("a", "e", 2), 0)), vec![2]);
        assert_eq!(s.next_deadline("a"), None);
    }

    #[test]
    fn out_of_order_frames_are_reordered() {
        let mut s = Sequencer::new();
        assert_eq!(seqs(&s.offer(ev("a", "e", 0), 0)), vec![0]);
        assert!(s.offer(ev("a", "e", 2), 10).is_empty());
        assert!(s.offer(ev("a", "e", 3), 11).is_empty());
        assert_eq!(s.next_deadline("a"), Some(10 + REORDER_WINDOW_MS));
        assert_eq!(seqs(&s.offer(ev("a", "e", 1), 12)), vec![1, 2, 3]);
        assert_eq!(s.next_deadline("a"), None);
    }

    #[test]
    fn duplicates_are_dropped_whether_released_or_buffered() {
        let mut s = Sequencer::new();
        assert_eq!(seqs(&s.offer(ev("a", "e", 0), 0)), vec![0]);
        assert!(s.offer(ev("a", "e", 0), 0).is_empty());
        assert!(s.offer(ev("a", "e", 2), 0).is_empty());
        assert!(s.offer(ev("a", "e", 2), 0).is_empty());
        assert_eq!(seqs(&s.offer(ev("a", "e", 1), 0)), vec![1, 2]);
        assert!(s.offer(ev("a", "e", 1), 0).is_empty());
    }

    #[test]
    fn a_lost_frame_is_skipped_after_the_window() {
        let mut s = Sequencer::new();
        s.offer(ev("a", "e", 0), 0);
        assert!(s.offer(ev("a", "e", 5), 100).is_empty());
        assert!(s.flush("a", 100 + REORDER_WINDOW_MS - 1, false).is_empty());
        assert_eq!(seqs(&s.flush("a", 100 + REORDER_WINDOW_MS, false)), vec![5]);
        // The skipped frame arriving late is stale now.
        assert!(s.offer(ev("a", "e", 3), 400).is_empty());
        assert_eq!(seqs(&s.offer(ev("a", "e", 6), 400)), vec![6]);
    }

    #[test]
    fn a_late_bind_waits_the_window_then_follows_the_stream() {
        let mut s = Sequencer::new();
        // Bound mid-epoch: seq 40 first. It waits for the window, then flows.
        assert!(s.offer(ev("a", "e", 40), 0).is_empty());
        assert_eq!(seqs(&s.flush("a", 0, true)), vec![40]);
        assert_eq!(seqs(&s.offer(ev("a", "e", 41), 1)), vec![41]);
    }

    #[test]
    fn epochs_sources_and_sessions_are_independent_streams() {
        let mut s = Sequencer::new();
        assert_eq!(seqs(&s.offer(ev("a", "e1", 0), 0)), vec![0]);
        assert_eq!(seqs(&s.offer(ev("a", "e2", 0), 0)), vec![0]);
        assert_eq!(seqs(&s.offer(ev("b", "e1", 0), 0)), vec![0]);
        let mut other = ev("a", "e1", 0);
        other.source = "codex".into();
        assert_eq!(seqs(&s.offer(other, 0)), vec![0]);
        assert_eq!(s.stream_count(), 4);
        s.forget_session("a");
        assert_eq!(s.stream_count(), 1);
    }

    #[test]
    fn the_reorder_buffer_is_bounded() {
        let mut s = Sequencer::new();
        s.offer(ev("a", "e", 0), 0);
        let mut released = Vec::new();
        for seq in 2..(MAX_PENDING_PER_STREAM as u64 + 3) {
            released.extend(s.offer(ev("a", "e", seq), 0));
        }
        assert_eq!(released.len(), MAX_PENDING_PER_STREAM + 1);
        assert!(released.windows(2).all(|w| w[0].seq < w[1].seq));
        assert_eq!(s.next_deadline("a"), None);
    }

    #[test]
    fn trigger_type_defaults_follow_the_brain_namespace() {
        assert_eq!(
            default_events_trigger_type("claude::run").as_deref(),
            Some("claude::agent-event")
        );
        assert_eq!(
            default_events_trigger_type("run::start_and_wait").as_deref(),
            Some("run::agent-event")
        );
        assert_eq!(default_events_trigger_type("plain"), None);
        assert_eq!(
            resolve_events_trigger_types(&[], Some("codex::run")),
            vec!["codex::agent-event"]
        );
        assert_eq!(
            resolve_events_trigger_types(
                &[
                    " pi::agent-event ".into(),
                    "".into(),
                    "pi::agent-event".into(),
                    "x::agent-event".into()
                ],
                Some("codex::run")
            ),
            vec!["pi::agent-event", "x::agent-event"]
        );
        assert!(resolve_events_trigger_types(&[], None).is_empty());
    }

    #[test]
    fn readiness_names_the_missing_provider_and_never_iii_stream() {
        let types = vec!["claude::agent-event".to_string()];
        assert_eq!(readiness_error(true, &types, &[]), None);
        let missing = readiness_error(true, &types, &types).unwrap();
        assert!(missing.contains("`claude::agent-event`"));
        assert!(missing.contains("`claude` agent worker"));
        assert!(readiness_error(false, &types, &[]).is_some());
        assert!(readiness_error(true, &[], &[]).is_some());
        for message in [
            missing,
            readiness_error(false, &types, &[]).unwrap(),
            readiness_error(true, &[], &[]).unwrap(),
        ] {
            assert!(!message.contains("iii-stream"), "{message}");
            assert!(!message.contains("stream"), "{message}");
        }
        assert!(is_not_found_error("Remote error NOT_FOUND: trigger type x"));
        assert!(!is_not_found_error("FORBIDDEN"));
    }

    #[test]
    fn final_text_reads_both_brain_shapes() {
        assert_eq!(
            final_text_from_brain_result(&json!({ "session_id": "s", "result": "hi" })).as_deref(),
            Some("hi")
        );
        assert_eq!(
            final_text_from_brain_result(&json!({ "messages": [
                { "role": "user", "content": [{ "type": "text", "text": "q" }] },
                { "role": "assistant", "content": [{ "type": "text", "text": "a" }, { "type": "thinking", "text": "t" }] }
            ] }))
            .as_deref(),
            Some("a")
        );
        assert_eq!(
            final_text_from_brain_result(&json!({ "result": "boom", "is_error": true })),
            None
        );
        assert_eq!(
            final_text_from_brain_result(&json!({ "result": "  " })),
            None
        );
        assert_eq!(final_text_from_brain_result(&json!({})), None);
    }
}
