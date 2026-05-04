//! DLP scrubber subscriber on `agent::after_tool_call`. Redacts common
//! secret shapes (AWS / OpenAI / GitHub / Stripe / Google) in the result's
//! text content. Replies with `{ content: [<redacted>] }` so the runtime's
//! `merge_after` overrides the result.

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::{
    FunctionRef, IIIError, RegisterFunctionMessage, RegisterTriggerInput, Trigger, TriggerRequest,
    III,
};
use serde_json::{json, Value};

const FN_DLP: &str = "policy::dlp_scrubber";
const TOPIC_AFTER: &str = "agent::after_tool_call";

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

/// Bus surface needed by the DLP-scrubber handler — reply on a stream plus
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

/// Build the canonical [`RegisterFunctionMessage`] for the DLP-scrubber function.
pub(crate) fn dlp_function_message() -> RegisterFunctionMessage {
    RegisterFunctionMessage::with_id(FN_DLP.into())
        .with_description("Redact common secret shapes in tool result text content.".into())
}

/// Build the canonical [`RegisterTriggerInput`] for the DLP-scrubber subscriber.
pub(crate) fn dlp_trigger_input() -> RegisterTriggerInput {
    RegisterTriggerInput {
        trigger_type: "subscribe".into(),
        function_id: FN_DLP.into(),
        config: json!({ "topic": TOPIC_AFTER }),
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

/// Run the DLP-scrubber handler logic against an arbitrary [`ReplyBus`]. Used
/// by the production closure (with [`IiiSdkBus`]) and by tests (with the
/// in-memory bus).
pub(crate) async fn handle_event(bus: &dyn ReplyBus, payload: Value) -> Value {
    let (event_id, reply_stream, inner) = unwrap_envelope(&payload);
    let original = inner.get("result").cloned().unwrap_or(Value::Null);
    let scrubbed = scrub_result_value(&original);
    let changed = scrubbed.ne(&original);
    let reply = if changed {
        json!({ "content": scrubbed.get("content").cloned().unwrap_or(Value::Null) })
    } else {
        json!({ "ok": true, "scrubbed": false })
    };
    write_hook_reply(bus, &reply_stream, &event_id, &reply).await;
    reply
}

pub fn subscribe_dlp_scrubber(iii: &III) -> Result<Subscriber, IIIError> {
    let bus: Arc<dyn ReplyBus> = Arc::new(IiiSdkBus(iii.clone()));

    let fn_msg = dlp_function_message();
    bus.record_function(&fn_msg);
    let bus_for_handler = bus.clone();
    let function = iii.register_function((fn_msg, move |payload: Value| {
        let bus = bus_for_handler.clone();
        async move {
            let reply = handle_event(bus.as_ref(), payload).await;
            Ok(reply)
        }
    }));

    let trig_input = dlp_trigger_input();
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

/// Pure scrubber over the JSON shape `{ content: [{ type, text }, ...] }`.
pub fn scrub_result_value(result: &Value) -> Value {
    let Some(content) = result.get("content").and_then(Value::as_array) else {
        return result.clone();
    };
    let scrubbed: Vec<Value> = content
        .iter()
        .map(|block| {
            let mut block = block.clone();
            if block.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    let redacted = scrub_text(text);
                    if redacted != text {
                        block["text"] = Value::String(redacted);
                    }
                }
            }
            block
        })
        .collect();
    let mut out = result.clone();
    if let Some(obj) = out.as_object_mut() {
        obj.insert("content".into(), Value::Array(scrubbed));
    }
    out
}

/// Pure secret-redaction over a string. Returns the original when no pattern
/// matches.
pub fn scrub_text(input: &str) -> String {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static AWS: Lazy<Regex> = Lazy::new(|| Regex::new(r"AKIA[0-9A-Z]{16}").unwrap());
    static OPENAI: Lazy<Regex> = Lazy::new(|| Regex::new(r"sk-[A-Za-z0-9]{32,}").unwrap());
    static GITHUB: Lazy<Regex> = Lazy::new(|| Regex::new(r"ghp_[A-Za-z0-9]{36}").unwrap());
    static STRIPE: Lazy<Regex> = Lazy::new(|| Regex::new(r"sk_live_[A-Za-z0-9]{24,}").unwrap());
    static GOOGLE: Lazy<Regex> = Lazy::new(|| Regex::new(r"AIza[0-9A-Za-z_\-]{35}").unwrap());

    let mut out = AWS.replace_all(input, "[REDACTED:aws]").to_string();
    out = OPENAI.replace_all(&out, "[REDACTED:openai]").to_string();
    out = GITHUB.replace_all(&out, "[REDACTED:github]").to_string();
    out = STRIPE.replace_all(&out, "[REDACTED:stripe]").to_string();
    out = GOOGLE.replace_all(&out, "[REDACTED:google]").to_string();
    out
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
        bus.record_function(&dlp_function_message());
        bus.record_trigger(&dlp_trigger_input());
    }

    #[tokio::test]
    async fn wiring_records_function_id_and_trigger_topic() {
        let bus = InMemoryBus::new();
        record_wiring(&bus);

        let fns = bus.recorded_functions();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].id, FN_DLP);
        assert_eq!(fns[0].id, "policy::dlp_scrubber");
        assert!(fns[0].description.is_some());

        let trigs = bus.recorded_triggers();
        assert_eq!(trigs.len(), 1);
        assert_eq!(trigs[0].trigger_type, "subscribe");
        assert_eq!(trigs[0].function_id, FN_DLP);
        assert_eq!(
            trigs[0].config.get("topic").and_then(Value::as_str),
            Some(TOPIC_AFTER)
        );
    }

    #[tokio::test]
    async fn handler_round_trips_mixed_text_and_image_content() {
        let bus = InMemoryBus::new();
        let aws = format!("AKIA{}", "Y".repeat(16));
        let payload = json!({
            "event_id": "e1",
            "reply_stream": "rs",
            "payload": {
                "result": {
                    "content": [
                        { "type": "text", "text": format!("leaked {aws}") },
                        { "type": "image", "data": "base64==" },
                    ],
                    "details": {},
                }
            }
        });
        let reply = handle_event(&bus, payload).await;

        let content = reply
            .get("content")
            .and_then(Value::as_array)
            .expect("content");
        assert_eq!(content.len(), 2);
        let text = content[0]
            .get("text")
            .and_then(Value::as_str)
            .expect("text");
        assert!(text.contains("[REDACTED:aws]"));
        assert!(!text.contains(&aws));
        // Image block passes through verbatim.
        assert_eq!(
            content[1].get("type").and_then(Value::as_str),
            Some("image")
        );
        assert_eq!(
            content[1].get("data").and_then(Value::as_str),
            Some("base64==")
        );

        let replies = bus.recorded_replies();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].stream_name, "rs");
        assert_eq!(replies[0].group_id, "e1");
    }

    #[test]
    fn scrub_text_redacts_aws() {
        let key = format!("AKIA{}", "X".repeat(16));
        let input = format!("found key {key} in log");
        let out = scrub_text(&input);
        assert!(out.contains("[REDACTED:aws]"));
        assert!(!out.contains(&key));
    }

    #[test]
    fn scrub_text_redacts_multiple_kinds() {
        let openai = format!("sk-{}", "0".repeat(40));
        let github = format!("ghp_{}", "1".repeat(36));
        let input = format!("openai={openai} github={github}");
        let out = scrub_text(&input);
        assert!(out.contains("[REDACTED:openai]"));
        assert!(out.contains("[REDACTED:github]"));
    }

    #[test]
    fn scrub_text_passthrough_when_no_secrets() {
        let s = "nothing sensitive here";
        assert_eq!(scrub_text(s), s);
    }

    #[test]
    fn scrub_result_value_rewrites_text_blocks() {
        let aws = format!("AKIA{}", "Z".repeat(16));
        let result = json!({
            "content": [
                { "type": "text", "text": format!("leaked {aws}") },
                { "type": "image", "data": "ignored" },
            ],
            "details": {},
        });
        let out = scrub_result_value(&result);
        let text = out["content"][0]["text"].as_str().expect("text block");
        assert!(text.contains("[REDACTED:aws]"));
        assert_eq!(out["content"][1]["data"].as_str(), Some("ignored"));
    }

    #[test]
    fn scrub_result_value_passthrough_when_no_content() {
        let v = json!({ "details": { "exit_code": 0 } });
        assert_eq!(scrub_result_value(&v), v);
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
}
