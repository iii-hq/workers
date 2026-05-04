//! Denylist subscriber for `agent::before_tool_call`. Blocks any call whose
//! `tool_call.name` is on a configured denylist.

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::{
    FunctionRef, IIIError, RegisterFunctionMessage, RegisterTriggerInput, Trigger, TriggerRequest,
    III,
};
use serde_json::{json, Value};

const FN_DENYLIST: &str = "policy::denylist";
const TOPIC_BEFORE: &str = "agent::before_tool_call";

pub struct Subscriber {
    function: FunctionRef,
    trigger: Trigger,
}

impl Subscriber {
    pub fn unregister(self) {
        self.trigger.unregister();
        self.function.unregister();
    }
}

/// Bus surface needed by the denylist handler — reply on a stream plus
/// register-time recorders so tests can assert wiring.
///
/// Production callers use [`IiiSdkBus`] which delegates `write_reply` to
/// `iii.trigger("stream::set", ...)` and treats the recorders as no-ops.
/// Tests inject an in-memory implementation that records every call.
#[async_trait]
pub(crate) trait ReplyBus: Send + Sync {
    async fn write_reply(&self, stream_name: &str, group_id: &str, item_id: &str, data: &Value);
    fn record_function(&self, msg: &RegisterFunctionMessage);
    fn record_trigger(&self, input: &RegisterTriggerInput);
}

/// Production [`ReplyBus`] backed by a real [`III`].
pub(crate) struct IiiSdkBus(pub III);

#[async_trait]
impl ReplyBus for IiiSdkBus {
    async fn write_reply(&self, stream_name: &str, group_id: &str, item_id: &str, data: &Value) {
        let _ = self
            .0
            .trigger(TriggerRequest {
                function_id: "stream::set".into(),
                payload: json!({
                    "stream_name": stream_name,
                    "group_id": group_id,
                    "item_id": item_id,
                    "data": data,
                }),
                action: None,
                timeout_ms: None,
            })
            .await;
    }

    fn record_function(&self, _msg: &RegisterFunctionMessage) {}
    fn record_trigger(&self, _input: &RegisterTriggerInput) {}
}

/// Build the canonical [`RegisterFunctionMessage`] for the denylist function.
pub(crate) fn denylist_function_message() -> RegisterFunctionMessage {
    RegisterFunctionMessage::with_id(FN_DENYLIST.into())
        .with_description("Block tool calls whose name is on a configured denylist.".into())
}

/// Build the canonical [`RegisterTriggerInput`] for the denylist subscriber.
pub(crate) fn denylist_trigger_input() -> RegisterTriggerInput {
    RegisterTriggerInput {
        trigger_type: "subscribe".into(),
        function_id: FN_DENYLIST.into(),
        config: json!({ "topic": TOPIC_BEFORE }),
        metadata: None,
    }
}

/// Extract `(event_id, reply_stream, inner)` from the iii hook envelope.
pub(crate) fn unwrap_envelope(payload: &Value) -> (String, String, Value) {
    let event_id = payload
        .get("event_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let reply_stream = payload
        .get("reply_stream")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let inner = payload
        .get("payload")
        .cloned()
        .unwrap_or_else(|| payload.clone());
    (event_id, reply_stream, inner)
}

/// Pure check: is `tool_name` on the denylist?
pub(crate) fn check_denylist(tool_name: &str, denied: &[String]) -> bool {
    denied.iter().any(|d| d == tool_name)
}

/// Run the denylist handler logic against an arbitrary [`ReplyBus`]. Used by
/// the production closure (with [`IiiSdkBus`]) and by tests (with the
/// in-memory bus).
pub(crate) async fn handle_event(bus: &dyn ReplyBus, denied: &[String], payload: Value) -> Value {
    let (event_id, reply_stream, inner) = unwrap_envelope(&payload);
    let tool_name = inner
        .get("tool_call")
        .and_then(|tc| tc.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let reply = if check_denylist(&tool_name, denied) {
        json!({
            "block": true,
            "reason": format!("policy::denylist blocked '{tool_name}'"),
        })
    } else {
        json!({ "block": false })
    };
    write_hook_reply(bus, &reply_stream, &event_id, &reply).await;
    reply
}

pub fn subscribe_denylist(iii: &III, denied_tools: Vec<String>) -> Result<Subscriber, IIIError> {
    let bus: Arc<dyn ReplyBus> = Arc::new(IiiSdkBus(iii.clone()));
    let denied: Arc<Vec<String>> = Arc::new(denied_tools);

    let fn_msg = denylist_function_message();
    bus.record_function(&fn_msg);
    let bus_for_handler = bus.clone();
    let function = iii.register_function((fn_msg, move |payload: Value| {
        let bus = bus_for_handler.clone();
        let denied = denied.clone();
        async move {
            let reply = handle_event(bus.as_ref(), denied.as_ref(), payload).await;
            Ok(reply)
        }
    }));

    let trig_input = denylist_trigger_input();
    bus.record_trigger(&trig_input);
    let trigger = iii.register_trigger(trig_input)?;
    Ok(Subscriber { function, trigger })
}

async fn write_hook_reply(bus: &dyn ReplyBus, stream_name: &str, event_id: &str, reply: &Value) {
    if event_id.is_empty() || stream_name.is_empty() {
        return;
    }
    let item_id = uuid::Uuid::new_v4().to_string();
    bus.write_reply(stream_name, event_id, &item_id, reply)
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) struct RecordedReply {
        pub stream_name: String,
        pub group_id: String,
        pub item_id: String,
        pub data: Value,
    }

    /// Fully in-process [`ReplyBus`] for tests. Captures every reply plus
    /// every register-time function/trigger so wiring drift surfaces as a
    /// failed assertion rather than as a silent production miss.
    pub(crate) struct InMemoryBus {
        replies: Mutex<Vec<RecordedReply>>,
        functions: Mutex<Vec<RegisterFunctionMessage>>,
        triggers: Mutex<Vec<RegisterTriggerInput>>,
    }

    impl InMemoryBus {
        pub(crate) fn new() -> Self {
            Self {
                replies: Mutex::new(Vec::new()),
                functions: Mutex::new(Vec::new()),
                triggers: Mutex::new(Vec::new()),
            }
        }

        pub(crate) fn recorded_replies(&self) -> Vec<RecordedReply> {
            self.replies.lock().expect("replies poisoned").clone()
        }

        pub(crate) fn recorded_functions(&self) -> Vec<RegisterFunctionMessage> {
            self.functions.lock().expect("functions poisoned").clone()
        }

        pub(crate) fn recorded_triggers(&self) -> Vec<RegisterTriggerInput> {
            self.triggers.lock().expect("triggers poisoned").clone()
        }
    }

    #[async_trait]
    impl ReplyBus for InMemoryBus {
        async fn write_reply(
            &self,
            stream_name: &str,
            group_id: &str,
            item_id: &str,
            data: &Value,
        ) {
            self.replies
                .lock()
                .expect("replies poisoned")
                .push(RecordedReply {
                    stream_name: stream_name.to_string(),
                    group_id: group_id.to_string(),
                    item_id: item_id.to_string(),
                    data: data.clone(),
                });
        }

        fn record_function(&self, msg: &RegisterFunctionMessage) {
            self.functions
                .lock()
                .expect("functions poisoned")
                .push(msg.clone());
        }

        fn record_trigger(&self, input: &RegisterTriggerInput) {
            self.triggers
                .lock()
                .expect("triggers poisoned")
                .push(input.clone());
        }
    }

    fn record_wiring(bus: &dyn ReplyBus) {
        bus.record_function(&denylist_function_message());
        bus.record_trigger(&denylist_trigger_input());
    }

    fn envelope(event_id: &str, reply_stream: &str, inner: Value) -> Value {
        json!({
            "event_id": event_id,
            "reply_stream": reply_stream,
            "payload": inner,
        })
    }

    #[tokio::test]
    async fn wiring_records_function_id_and_trigger_topic() {
        let bus = InMemoryBus::new();
        record_wiring(&bus);

        let fns = bus.recorded_functions();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].id, FN_DENYLIST);
        assert_eq!(fns[0].id, "policy::denylist");
        assert!(fns[0].description.is_some());

        let trigs = bus.recorded_triggers();
        assert_eq!(trigs.len(), 1);
        assert_eq!(trigs[0].trigger_type, "subscribe");
        assert_eq!(trigs[0].function_id, FN_DENYLIST);
        assert_eq!(
            trigs[0].config.get("topic").and_then(Value::as_str),
            Some(TOPIC_BEFORE)
        );
    }

    #[tokio::test]
    async fn handler_blocks_denied_tool_name() {
        let bus = InMemoryBus::new();
        let denied = vec!["dangerous_tool".to_string()];
        let payload = envelope(
            "e1",
            "rs",
            json!({ "tool_call": { "name": "dangerous_tool" } }),
        );
        let reply = handle_event(&bus, &denied, payload).await;

        assert_eq!(reply.get("block"), Some(&Value::Bool(true)));
        assert!(reply
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("dangerous_tool"));

        let replies = bus.recorded_replies();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].stream_name, "rs");
        assert_eq!(replies[0].group_id, "e1");
        assert_eq!(replies[0].data.get("block"), Some(&Value::Bool(true)));
    }

    #[tokio::test]
    async fn handler_allows_unlisted_tool_name() {
        let bus = InMemoryBus::new();
        let denied = vec!["dangerous_tool".to_string()];
        let payload = envelope("e1", "rs", json!({ "tool_call": { "name": "safe_tool" } }));
        let reply = handle_event(&bus, &denied, payload).await;

        assert_eq!(reply, json!({ "block": false }));
        let replies = bus.recorded_replies();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].data, json!({ "block": false }));
    }

    #[tokio::test]
    async fn handler_treats_missing_tool_name_as_allowed() {
        let bus = InMemoryBus::new();
        let denied = vec!["dangerous_tool".to_string()];
        let payload = envelope("e1", "rs", json!({ "tool_call": {} }));
        let reply = handle_event(&bus, &denied, payload).await;

        assert_eq!(reply, json!({ "block": false }));
    }

    #[tokio::test]
    async fn handler_skips_reply_when_event_id_missing() {
        let bus = InMemoryBus::new();
        let denied = vec!["dangerous_tool".to_string()];
        let payload = json!({
            "reply_stream": "rs",
            "payload": { "tool_call": { "name": "dangerous_tool" } },
        });
        let reply = handle_event(&bus, &denied, payload).await;

        assert_eq!(reply.get("block"), Some(&Value::Bool(true)));
        assert!(bus.recorded_replies().is_empty());
    }

    #[test]
    fn check_denylist_empty_list_allows_everything() {
        assert!(!check_denylist("anything", &[]));
    }

    #[test]
    fn check_denylist_exact_match_blocks() {
        let denied = vec!["bash".to_string(), "rm".to_string()];
        assert!(check_denylist("bash", &denied));
        assert!(check_denylist("rm", &denied));
    }

    #[test]
    fn check_denylist_no_match_allows() {
        let denied = vec!["bash".to_string()];
        assert!(!check_denylist("python", &denied));
    }

    #[test]
    fn check_denylist_is_case_sensitive() {
        let denied = vec!["bash".to_string()];
        assert!(!check_denylist("Bash", &denied));
        assert!(!check_denylist("BASH", &denied));
    }

    #[test]
    fn unwrap_envelope_extracts_fields() {
        let v = json!({
            "event_id": "e1",
            "reply_stream": "rs",
            "payload": { "k": 1 },
        });
        let (ev, rs, inner) = unwrap_envelope(&v);
        assert_eq!(ev, "e1");
        assert_eq!(rs, "rs");
        assert_eq!(inner, json!({ "k": 1 }));
    }

    #[test]
    fn unwrap_envelope_defaults_missing_to_empty_strings() {
        let v = json!({});
        let (ev, rs, inner) = unwrap_envelope(&v);
        assert!(ev.is_empty());
        assert!(rs.is_empty());
        assert_eq!(inner, json!({}));
    }
}
