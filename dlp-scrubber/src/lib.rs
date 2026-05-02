//! DLP scrubber subscriber on `agent::after_tool_call`. Redacts common
//! secret shapes (AWS / OpenAI / GitHub / Stripe / Google) in the result's
//! text content. Replies with `{ content: [<redacted>] }` so the runtime's
//! `merge_after` overrides the result.

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

pub fn subscribe_dlp_scrubber(iii: &III) -> Result<Subscriber, IIIError> {
    let iii_for_handler = iii.clone();
    let function = iii.register_function((
        RegisterFunctionMessage::with_id(FN_DLP.into())
            .with_description("Redact common secret shapes in tool result text content.".into()),
        move |payload: Value| {
            let iii = iii_for_handler.clone();
            async move {
                let (event_id, reply_stream, inner) = unwrap_envelope(&payload);
                let original = inner.get("result").cloned().unwrap_or(Value::Null);
                let scrubbed = scrub_result_value(&original);
                let changed = scrubbed.ne(&original);
                let reply = if changed {
                    json!({ "content": scrubbed.get("content").cloned().unwrap_or(Value::Null) })
                } else {
                    json!({ "ok": true, "scrubbed": false })
                };
                write_hook_reply(&iii, &reply_stream, &event_id, &reply).await;
                Ok(reply)
            }
        },
    ));
    let trigger = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "subscribe".into(),
        function_id: FN_DLP.into(),
        config: json!({ "topic": TOPIC_AFTER }),
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
}
