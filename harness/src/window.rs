//! The model-facing message window, derived from the durable log alone so
//! every step replays exactly the prefix earlier steps sent.
//!
//! Claude models that bind thinking to the conversation prefix (Opus 5.5,
//! Sonnet 5.5, Haiku 5.5, Fable 5.1) invalidate every later thinking block when an earlier message
//! changes, moves, or disappears between requests; every provider's prompt
//! cache loses the same prefix. So nothing the harness shows the model is
//! ephemeral:
//!
//! - text the harness or a pre_generate hook adds for one step (contract
//!   notices, the discovery hint, memory recall, runtime-context changes) is
//!   persisted as a [`MODEL_NOTICE`] custom entry at the position it was sent
//!   and replayed there as a user message on every later step;
//! - a user message that arrived while a reply was generating is shown after
//!   that reply, and the move is persisted as a [`MESSAGE_ORDER`] entry so
//!   every later window keeps it there.

use std::collections::HashSet;

use serde_json::{json, Value};

use crate::clients::session::LoadedEntry;
use crate::types::content::ContentBlock;
use crate::types::message::{AgentMessage, UserMessage, UserRoleTag};

/// Custom entry: text the model was shown at this position (data:
/// `{kind, text, message}`), replayed as a user message.
pub const MODEL_NOTICE: &str = "model_notice";
/// Custom entry: `{after, moved}` — show the `moved` message entries right
/// after the `after` entry.
pub const MESSAGE_ORDER: &str = "message_order";

/// Custom entry: the runtime context first rendered into the session's
/// system prompt (`{aid}`); every later step renders the same bytes.
pub const RUNTIME_CONTEXT: &str = "runtime_context";

/// The aid of the first [`RUNTIME_CONTEXT`] entry on the path, if any.
pub fn frozen_runtime_context(entries: &[LoadedEntry]) -> Option<String> {
    entries
        .iter()
        .filter_map(|e| e.custom.as_ref())
        .find(|c| c.custom_type == RUNTIME_CONTEXT)?
        .data
        .get("aid")?
        .as_str()
        .map(str::to_string)
}

/// Models that bind each thinking block to the exact prefix it was produced
/// under (Claude Opus 5.5, Sonnet 5.5, Haiku 5.5, Fable 5.1; Mythos 5.1 does
/// not run the check): rewriting earlier history — pruning aged function
/// results included — drops their reasoning.
// ponytail: hardcoded ids; catalog capability flag when the next binding model ships
pub fn binds_thinking(model: &str) -> bool {
    [
        "claude-opus-5-5",
        "claude-sonnet-5-5",
        "claude-haiku-5-5",
        "claude-fable-5-1",
    ]
    .iter()
    .any(|m| model.contains(m))
}

/// A compaction puts the summary in the system prompt and drops the head, so
/// on a binding model every thinking block logged before the compaction
/// record — the kept tail's included — was signed over a prefix that is
/// gone: the API drops it and every block after it, on every later request.
/// Strip those blocks (text and calls stay), the same bytes on every step,
/// so reasoning produced after the compaction stays valid.
pub fn strip_thinking_logged_before(window: &mut Window, before: &[LoadedEntry]) {
    let stale: HashSet<&str> = before.iter().map(|e| e.entry_id.as_str()).collect();
    for (id, message) in &mut window.candidate {
        if stale.contains(id.as_str()) {
            message.strip_thinking();
        }
    }
}

/// A reorder decision, persisted as a [`MESSAGE_ORDER`] entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageOrder {
    pub after: String,
    pub moved: Vec<String>,
}

impl MessageOrder {
    pub fn to_data(&self) -> Value {
        json!({ "after": self.after, "moved": self.moved })
    }

    fn from_data(data: &Value) -> Option<Self> {
        let after = data.get("after")?.as_str()?.to_string();
        let moved = data
            .get("moved")?
            .as_array()?
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect::<Vec<_>>();
        (!moved.is_empty()).then_some(Self { after, moved })
    }
}

/// The data of a [`MODEL_NOTICE`] entry for `message` (a user-role message
/// value as sent to the model). `text` is its text, for readers of the log.
pub fn notice_data(kind: &str, message: &Value) -> Value {
    let text = message
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    json!({ "kind": kind, "text": text, "message": message })
}

/// The user message a [`MODEL_NOTICE`] entry replays: the stored message,
/// else its text.
fn notice_message(data: &Value) -> Option<AgentMessage> {
    if let Some(mut message) = data.get("message").cloned() {
        // A hook-appended message may omit the timestamp the type requires.
        if let Some(object) = message.as_object_mut() {
            object.entry("timestamp").or_insert(json!(0));
        }
        if let Ok(parsed @ AgentMessage::User(_)) = serde_json::from_value(message) {
            return Some(parsed);
        }
    }
    let text = data.get("text")?.as_str().filter(|t| !t.is_empty())?;
    Some(AgentMessage::User(UserMessage {
        role: UserRoleTag::User,
        content: vec![ContentBlock::text(text)],
        timestamp: 0,
    }))
}

/// `message` exactly as every later step replays it.
pub fn replayed(message: &Value) -> Option<Value> {
    serde_json::to_value(notice_message(&notice_data("", message))?).ok()
}

/// The latest [`MODEL_NOTICE`] text of `kind` still in `window`, if any: a
/// notice a compaction summarized away no longer tells the model anything.
pub fn latest_notice_text<'a>(
    entries: &'a [LoadedEntry],
    window: &Window,
    kind: &str,
) -> Option<&'a str> {
    entries
        .iter()
        .rev()
        .filter_map(|e| Some((e.entry_id.as_str(), e.custom.as_ref()?)))
        .filter(|(_, c)| {
            c.custom_type == MODEL_NOTICE
                && c.data.get("kind").and_then(Value::as_str) == Some(kind)
        })
        .find(|(id, _)| window.candidate.iter().any(|(shown, _)| shown == id))
        .and_then(|(_, c)| c.data.get("text").and_then(Value::as_str))
}

pub struct Window {
    /// `(entry_id, message)` in the order the model sees them.
    pub candidate: Vec<(String, AgentMessage)>,
    /// A move decided on this step; persist it before the request goes out.
    pub new_order: Option<MessageOrder>,
}

/// Where messages that arrived while the last reply generated belong: after
/// that reply (no calls) or after its last result once every call has one.
/// `None` while calls still wait for results — a message between a call and
/// its result is a shape providers reject.
fn move_anchor(list: &[(String, AgentMessage)]) -> Option<usize> {
    let last = list
        .iter()
        .rposition(|(_, m)| !matches!(m, AgentMessage::User(_)))?;
    let reply = list[..=last]
        .iter()
        .rposition(|(_, m)| matches!(m, AgentMessage::Assistant(_)))?;
    let AgentMessage::Assistant(a) = &list[reply].1 else {
        return None;
    };
    let answered: HashSet<&str> = list[reply + 1..=last]
        .iter()
        .filter_map(|(_, m)| match m {
            AgentMessage::FunctionResult(r) => Some(r.function_call_id.as_str()),
            _ => None,
        })
        .collect();
    let complete = a.content.iter().all(|b| match b {
        ContentBlock::FunctionCall { id, .. } => answered.contains(id.as_str()),
        _ => true,
    });
    (last > 0 && complete).then_some(last)
}

/// Build the window: every message entry (and replayed notice) in log order,
/// `file` references stripped, every persisted [`MESSAGE_ORDER`] applied —
/// the order the model saw them in — then cut at `tail_start` (compaction
/// records its tail in that same order). User messages logged after
/// `prev_watermark` (while the last reply generated) but before that reply or
/// its results move after them in a new order: the model answered without
/// seeing them, its thinking is bound to the prefix it saw, and the request
/// must end with the user. `own_reply`, the entry this step's generation
/// writes, is not shown while none of its calls has a logged result: on a
/// redelivered step it holds the dead attempt's stale reply, which this
/// generation overwrites. A reply whose calls already ran stays, so the model
/// sees what executed instead of re-issuing it.
pub fn build(
    entries: &[LoadedEntry],
    window_start: usize,
    prev_watermark: Option<&str>,
    own_reply: Option<&str>,
) -> Window {
    let mut orders: Vec<MessageOrder> = Vec::new();
    let mut list: Vec<(String, AgentMessage)> = Vec::new();
    // Log position of every model-facing entry, for the window cut.
    let mut logged_at: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    // Real user messages logged after the previous step's watermark.
    let mut arrived: HashSet<&str> = HashSet::new();
    let mut past_watermark = false;
    let own_reply = own_reply.filter(|own| !dispatched(entries, own));

    for (index, entry) in entries.iter().enumerate() {
        let model_message = match (&entry.message, &entry.custom) {
            _ if own_reply == Some(entry.entry_id.as_str()) => None,
            (Some(AgentMessage::Custom(_)), _) => None,
            (Some(message), _) => {
                let mut message = message.clone();
                // The console also sends the <attached-file> text expansion;
                // persisted entries keep the refs.
                message.strip_file_blocks();
                if past_watermark && matches!(message, AgentMessage::User(_)) {
                    arrived.insert(entry.entry_id.as_str());
                }
                Some(message)
            }
            (None, Some(custom)) if custom.custom_type == MODEL_NOTICE => {
                notice_message(&custom.data)
            }
            (None, Some(custom)) if custom.custom_type == MESSAGE_ORDER => {
                orders.extend(MessageOrder::from_data(&custom.data));
                None
            }
            _ => None,
        };
        if let Some(message) = model_message {
            logged_at.insert(entry.entry_id.clone(), index);
            list.push((entry.entry_id.clone(), message));
        }
        if prev_watermark == Some(entry.entry_id.as_str()) {
            past_watermark = true;
        }
    }

    for order in &orders {
        apply_order(&mut list, order);
    }
    // Cut in model order: a compaction tail id names a model-order position
    // (messages a recorded order moved past it stay with it); a window that
    // opens on a non-message entry (after a record) starts at the first
    // message logged from there on.
    let start = entries
        .get(window_start)
        .and_then(|first| list.iter().position(|(id, _)| *id == first.entry_id))
        .or_else(|| {
            list.iter()
                .position(|(id, _)| logged_at[id] >= window_start)
        })
        .unwrap_or(list.len());
    list.drain(..start);

    let new_order = move_anchor(&list).and_then(|anchor| {
        // Never move the window's opening message: it starts with the user.
        let moved: Vec<String> = list[1..anchor]
            .iter()
            .filter(|(id, _)| arrived.contains(id.as_str()))
            .map(|(id, _)| id.clone())
            .collect();
        (!moved.is_empty()).then(|| MessageOrder {
            after: list[anchor].0.clone(),
            moved,
        })
    });
    if let Some(order) = &new_order {
        apply_order(&mut list, order);
    }
    Window {
        candidate: list,
        new_order,
    }
}

/// Whether a call in entry `id`'s assistant message has a result logged after
/// it (call ids can repeat across replies, so an earlier one never counts).
fn dispatched(entries: &[LoadedEntry], id: &str) -> bool {
    let Some(at) = entries.iter().position(|e| e.entry_id == id) else {
        return false;
    };
    let Some(AgentMessage::Assistant(reply)) = &entries[at].message else {
        return false;
    };
    let calls: HashSet<&str> = reply
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::FunctionCall { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    entries[at + 1..].iter().any(|e| {
        matches!(&e.message, Some(AgentMessage::FunctionResult(r)) if calls.contains(r.function_call_id.as_str()))
    })
}

/// The notices this step still has to send. Once the step's own reply entry
/// exists its request went out with every notice it persisted (the window
/// replays them), so none. Before that the request never went out: a notice
/// an earlier attempt persisted is already in the window, so only the ones it
/// left out are sent (the per-notice appends are not atomic).
pub fn unsent_notices(
    entries: &[LoadedEntry],
    own_reply: &str,
    notice_prefix: &str,
    candidates: impl IntoIterator<Item = (&'static str, Value)>,
) -> Vec<(&'static str, Value)> {
    if entries.iter().any(|e| e.entry_id == own_reply) {
        return Vec::new();
    }
    let persisted: Vec<&Value> = entries
        .iter()
        .filter(|e| e.entry_id.starts_with(notice_prefix))
        .filter_map(|e| e.custom.as_ref().map(|c| &c.data))
        .collect();
    candidates
        .into_iter()
        .filter(|(kind, message)| !persisted.contains(&&notice_data(kind, message)))
        .collect()
}

/// Move `order.moved` right after `order.after`. A no-op when the anchor is
/// not in `list`; moved ids not in `list`, and the anchor itself (the log is
/// caller-writable), are skipped.
fn apply_order(list: &mut Vec<(String, AgentMessage)>, order: &MessageOrder) {
    if !list.iter().any(|(id, _)| *id == order.after) {
        return;
    }
    let mut moved: Vec<(String, AgentMessage)> = Vec::new();
    for id in order.moved.iter().filter(|id| **id != order.after) {
        if let Some(pos) = list.iter().position(|(lid, _)| lid == id) {
            moved.push(list.remove(pos));
        }
    }
    let at = list
        .iter()
        .position(|(id, _)| *id == order.after)
        .map_or(list.len(), |anchor| anchor + 1);
    list.splice(at..at, moved);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::session::LoadedCustom;
    use crate::types::message::empty_assistant;

    fn msg(id: &str, message: AgentMessage) -> LoadedEntry {
        LoadedEntry {
            entry_id: id.into(),
            message: Some(message),
            custom: None,
        }
    }

    fn custom(id: &str, custom_type: &str, data: Value) -> LoadedEntry {
        LoadedEntry {
            entry_id: id.into(),
            message: None,
            custom: Some(LoadedCustom {
                custom_type: custom_type.into(),
                data,
            }),
        }
    }

    fn user(text: &str) -> AgentMessage {
        AgentMessage::user_text(text)
    }

    fn reply(text: &str) -> AgentMessage {
        let mut a = empty_assistant("p", "m");
        a.content = vec![ContentBlock::text(text)];
        AgentMessage::Assistant(a)
    }

    fn ids(window: &Window) -> Vec<&str> {
        window.candidate.iter().map(|(id, _)| id.as_str()).collect()
    }

    #[test]
    fn a_persisted_notice_replays_where_it_was_sent() {
        let notice = json!({"role": "user", "content": [{"type": "text", "text": "[harness] registry changed"}], "timestamp": 1});
        let entries = vec![
            msg("u1", user("hi")),
            custom("n1", MODEL_NOTICE, notice_data("registry-changed", &notice)),
            msg("a1", reply("done")),
        ];
        let w = build(&entries, 0, None, None);
        assert_eq!(ids(&w), ["u1", "n1", "a1"]);
        match &w.candidate[1].1 {
            AgentMessage::User(u) => assert_eq!(
                u.content,
                vec![ContentBlock::text("[harness] registry changed")]
            ),
            other => panic!("{other:?}"),
        }
        assert!(w.new_order.is_none());
        assert_eq!(
            latest_notice_text(&entries, &w, "registry-changed"),
            Some("[harness] registry changed")
        );
        // Summarized away (the window opens after it): no longer told.
        let compacted = build(&entries, 2, None, None);
        assert_eq!(
            latest_notice_text(&entries, &compacted, "registry-changed"),
            None
        );
    }

    #[test]
    fn a_mid_generation_user_moves_after_the_reply_and_stays_there() {
        // u2 arrived while a1 generated (after watermark u1), so it sits
        // before a1 in the log.
        let entries = vec![
            msg("u1", user("go")),
            msg("u2", user("steer")),
            msg("a1", reply("ok")),
        ];
        let first = build(&entries, 0, Some("u1"), None);
        assert_eq!(ids(&first), ["u1", "a1", "u2"]);
        let order = first.new_order.clone().expect("a move is decided");
        assert_eq!(
            order,
            MessageOrder {
                after: "a1".into(),
                moved: vec!["u2".into()]
            }
        );

        // Later steps: the move is in the log; the watermark has advanced.
        let mut later = entries.clone();
        later.push(custom("o1", MESSAGE_ORDER, order.to_data()));
        later.push(msg("a2", reply("answer")));
        let next = build(&later, 0, Some("a1"), None);
        assert_eq!(ids(&next), ["u1", "a1", "u2", "a2"]);
        assert!(next.new_order.is_none());
    }

    #[test]
    fn harness_notices_never_move_and_the_opening_message_stays_first() {
        let notice =
            json!({"role": "user", "content": [{"type": "text", "text": "hint"}], "timestamp": 1});
        let entries = vec![
            msg("u1", user("go")),
            custom("n1", MODEL_NOTICE, notice_data("hook", &notice)),
            msg("a1", reply("ok")),
        ];
        // The notice was logged after the watermark but was sent before a1.
        let w = build(&entries, 0, Some("u1"), None);
        assert_eq!(ids(&w), ["u1", "n1", "a1"]);
        assert!(w.new_order.is_none());
    }

    #[test]
    fn the_compaction_tail_is_cut_in_the_order_the_model_saw() {
        let order = MessageOrder {
            after: "a1".into(),
            moved: vec!["u2".into()],
        };
        let entries = vec![
            msg("u1", user("go")),
            msg("u2", user("steer")),
            msg("a1", reply("ok")),
            custom("o1", MESSAGE_ORDER, order.to_data()),
            msg("a2", reply("answer")),
        ];
        // Model order is u1 a1 u2 a2. A tail from a1 keeps u2 (logged
        // before a1); a tail from u2 drops a1 (summarized before it); a
        // window opening after o1 (a null boundary) starts at a2.
        assert_eq!(
            ids(&build(&entries, 2, Some("a1"), None)),
            ["a1", "u2", "a2"]
        );
        assert_eq!(ids(&build(&entries, 1, Some("a1"), None)), ["u2", "a2"]);
        assert_eq!(ids(&build(&entries, 4, Some("a1"), None)), ["a2"]);
        assert!(build(&entries, 5, Some("a1"), None).candidate.is_empty());
    }

    fn call(tag: &str) -> AgentMessage {
        let mut a = empty_assistant("p", "m");
        a.content = vec![
            ContentBlock::text(tag),
            serde_json::from_value(
                json!({"type": "function_call", "id": "c1", "function_id": "f", "arguments": {}}),
            )
            .expect("call fixture"),
        ];
        AgentMessage::Assistant(a)
    }

    fn result(tag: &str) -> AgentMessage {
        serde_json::from_value(json!({
            "role": "function_result", "function_call_id": "c1", "function_id": "f",
            "content": [{"type": "text", "text": tag}], "details": null, "is_error": false, "timestamp": 0
        }))
        .expect("result fixture")
    }

    /// The live prefill-400 repro: a notification logged while a2 generated
    /// sits before it; the re-generate must end with the notification.
    #[test]
    fn a_notification_logged_mid_generation_ends_the_window() {
        let entries = vec![
            msg("u", user("task")),
            msg("a1", reply("a1")),
            msg("n", user("notif")),
            msg("a2", reply("a2")),
        ];
        assert_eq!(
            ids(&build(&entries, 0, Some("a1"), None)),
            ["u", "a1", "a2", "n"]
        );
    }

    #[test]
    fn calls_results_and_trailing_users_stay_in_place() {
        // Pending calls: results follow, so the wire never ends on the reply.
        let pending = vec![
            msg("u", user("task")),
            msg("n", user("notif")),
            msg("a1", call("a1")),
        ];
        let w = build(&pending, 0, Some("u"), None);
        assert_eq!(ids(&w), ["u", "n", "a1"]);
        assert!(w.new_order.is_none());
        // A call answered after the arrival: it moves after the result.
        let answered = vec![
            msg("u", user("task")),
            msg("n", user("notif")),
            msg("a1", call("a1")),
            msg("r1", result("r1")),
        ];
        assert_eq!(
            ids(&build(&answered, 0, Some("u"), None)),
            ["u", "a1", "r1", "n"]
        );
        // Only the user message moves; the call/result pairing stays intact.
        let paired = vec![
            msg("u", user("task")),
            msg("a1", call("a1")),
            msg("r1", result("r1")),
            msg("n", user("notif")),
            msg("a2", reply("a2")),
        ];
        assert_eq!(
            ids(&build(&paired, 0, Some("a1"), None)),
            ["u", "a1", "r1", "a2", "n"]
        );
        // Already ending on the user: nothing to do.
        let steer = vec![
            msg("u", user("task")),
            msg("a1", reply("a1")),
            msg("s", user("steer")),
        ];
        assert!(build(&steer, 0, Some("u"), None).new_order.is_none());
    }

    /// A redelivered step whose first attempt left its reply entry (a2)
    /// empty: this generation overwrites a2, so the window never shows it and
    /// the notification logged before it trails without a move. Once a2
    /// holds a call and its result, the next window extends that one.
    #[test]
    fn a_redelivered_step_never_shows_its_own_stale_reply() {
        let mut empty = empty_assistant("p", "m");
        empty.content.clear();
        let mut entries = vec![
            msg("u", user("task")),
            msg("a1", reply("a1")),
            msg("n", user("notif")),
            msg("a2", AgentMessage::Assistant(empty)),
        ];
        // Redelivered with the old watermark: n counts as arrived.
        let redelivered = build(&entries, 0, Some("u"), Some("a2"));
        assert_eq!(ids(&redelivered), ["u", "a1", "n"]);
        assert!(redelivered.new_order.is_none());

        entries[3] = msg("a2", call("a2"));
        entries.push(msg("r2", result("r2")));
        let next = build(&entries, 0, Some("a2"), Some("a3"));
        assert_eq!(ids(&next), ["u", "a1", "n", "a2", "r2"]);
        assert!(next.new_order.is_none());
        assert_eq!(
            next.candidate[..redelivered.candidate.len()],
            redelivered.candidate[..]
        );

        // A dead attempt whose calls already ran keeps its reply (hiding it
        // would orphan r2 and re-issue the side effect); n, logged while it
        // generated, moves after it and its result.
        assert_eq!(
            ids(&build(&entries, 0, Some("u"), Some("a2"))),
            ["u", "a1", "a2", "r2", "n"]
        );
    }

    /// A result for an earlier reply that reused the call id is not this
    /// step's dispatch: the stale reply stays hidden.
    #[test]
    fn an_earlier_result_with_the_same_call_id_is_not_a_dispatch() {
        let entries = vec![
            msg("u", user("task")),
            msg("a1", call("a1")),
            msg("r1", result("r1")),
            msg("a2", call("a2")),
        ];
        assert_eq!(
            ids(&build(&entries, 0, Some("r1"), Some("a2"))),
            ["u", "a1", "r1"]
        );
    }

    #[test]
    fn a_retried_step_sends_only_the_notices_it_has_not_persisted() {
        let a = json!({"role": "user", "content": [{"type": "text", "text": "A"}], "timestamp": 1});
        let b = json!({"role": "user", "content": [{"type": "text", "text": "B"}], "timestamp": 1});
        let mut entries = vec![
            msg("u", user("task")),
            custom("e_t_0_notice_0", MODEL_NOTICE, notice_data("hook", &a)),
        ];
        let fresh = unsent_notices(
            &entries,
            "e_t_0_assistant",
            "e_t_0_notice_",
            [("hook", a.clone()), ("hook", b.clone())],
        );
        assert_eq!(fresh, [("hook", b.clone())]);
        // Once the reply entry exists the request went out: nothing more.
        entries.push(msg("e_t_0_assistant", reply("")));
        assert!(
            unsent_notices(&entries, "e_t_0_assistant", "e_t_0_notice_", [("hook", b)]).is_empty()
        );
    }

    /// The log is caller-writable: an order that lists its own anchor keeps
    /// the anchor in place and still moves the rest.
    #[test]
    fn an_order_listing_its_own_anchor_moves_only_the_rest() {
        let entries = vec![
            msg("u1", user("go")),
            msg("u2", user("steer")),
            msg("a1", reply("ok")),
            custom(
                "o1",
                MESSAGE_ORDER,
                json!({"after": "a1", "moved": ["a1", "u2"]}),
            ),
            custom("o2", MESSAGE_ORDER, json!({"after": "u1", "moved": ["u1"]})),
        ];
        assert_eq!(ids(&build(&entries, 0, None, None)), ["u1", "a1", "u2"]);
    }

    #[test]
    fn thinking_logged_before_a_compaction_record_is_stripped_and_later_thinking_kept() {
        let thought = |text: &str| {
            let mut a = empty_assistant("p", "m");
            a.content = vec![
                ContentBlock::Thinking {
                    text: String::new(),
                    signature: Some("sig".into()),
                },
                ContentBlock::RedactedThinking { data: "x".into() },
                ContentBlock::text(text),
            ];
            AgentMessage::Assistant(a)
        };
        let entries = vec![
            msg("u1", user("hi")),
            msg("a1", thought("kept tail")),
            custom(
                "c1",
                "compaction",
                json!({"summary": "s", "tail_start_entry_id": "u1"}),
            ),
            msg("a2", thought("after the summary")),
        ];
        let mut w = build(&entries, 0, None, None);
        strip_thinking_logged_before(&mut w, &entries[..2]);
        let blocks = |i: usize| match &w.candidate[i].1 {
            AgentMessage::Assistant(a) => a.content.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(blocks(1), vec![ContentBlock::text("kept tail")]);
        assert_eq!(blocks(2).len(), 3, "thinking signed after the record stays");
    }

    #[test]
    fn binding_models_are_matched_by_id_and_dated_or_prefixed_forms() {
        assert!(binds_thinking("claude-opus-5-5"));
        assert!(binds_thinking("claude-opus-5-5-20260901"));
        assert!(binds_thinking("anthropic.claude-fable-5-1"));
        assert!(binds_thinking("claude-code/claude-opus-5-5"));
        assert!(binds_thinking("claude-sonnet-5-5"));
        assert!(binds_thinking("anthropic.claude-sonnet-5-5"));
        assert!(binds_thinking("claude-haiku-5-5"));
        assert!(!binds_thinking("claude-haiku-4-5"));
        assert!(!binds_thinking("claude-mythos-5-1"));
        assert!(!binds_thinking("claude-opus-5"));
        assert!(!binds_thinking("claude-sonnet-5"));
    }

    #[test]
    fn a_non_user_hook_append_is_sent_as_it_replays() {
        let assistant =
            json!({"role": "assistant", "content": [{"type": "text", "text": "recall"}]});
        let sent = replayed(&assistant).expect("its text replays as a user message");
        assert_eq!(sent["role"], "user");
        let entries = vec![
            msg("u1", user("go")),
            custom("n1", MODEL_NOTICE, notice_data("hook", &sent)),
        ];
        let replay = serde_json::to_value(&build(&entries, 0, None, None).candidate[1].1).unwrap();
        assert_eq!(replay, sent);
        // No text to show: never sent, rather than sent as an empty block.
        assert!(replayed(&json!({"role": "assistant", "content": []})).is_none());
    }
}
