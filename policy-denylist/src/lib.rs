//! Denylist subscriber for `agent::before_tool_call`. Blocks any call whose
//! `tool_call.name` is on a configured denylist.

use std::sync::Arc;

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

pub fn subscribe_denylist(iii: &III, denied_tools: Vec<String>) -> Result<Subscriber, IIIError> {
    let denied: Arc<Vec<String>> = Arc::new(denied_tools);
    let iii_for_handler = iii.clone();
    let function = iii.register_function((
        RegisterFunctionMessage::with_id(FN_DENYLIST.into())
            .with_description("Block tool calls whose name is on a configured denylist.".into()),
        move |payload: Value| {
            let denied = denied.clone();
            let iii = iii_for_handler.clone();
            async move {
                let (event_id, reply_stream, inner) = unwrap_envelope(&payload);
                let tool_name = inner
                    .get("tool_call")
                    .and_then(|tc| tc.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let reply = if denied.iter().any(|d| d == &tool_name) {
                    json!({
                        "block": true,
                        "reason": format!("policy::denylist blocked '{tool_name}'"),
                    })
                } else {
                    json!({ "block": false })
                };
                write_hook_reply(&iii, &reply_stream, &event_id, &reply).await;
                Ok(reply)
            }
        },
    ));
    let trigger = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "subscribe".into(),
        function_id: FN_DENYLIST.into(),
        config: json!({ "topic": TOPIC_BEFORE }),
        metadata: None,
    })?;
    Ok(Subscriber { function, trigger })
}

fn unwrap_envelope(payload: &Value) -> (String, String, Value) {
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

async fn write_hook_reply(iii: &III, stream_name: &str, event_id: &str, reply: &Value) {
    if event_id.is_empty() || stream_name.is_empty() {
        return;
    }
    let item_id = uuid::Uuid::new_v4().to_string();
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "stream::set".into(),
            payload: json!({
                "stream_name": stream_name,
                "group_id": event_id,
                "item_id": item_id,
                "data": reply,
            }),
            action: None,
            timeout_ms: None,
        })
        .await;
}
