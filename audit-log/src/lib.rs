//! Append-only audit-log subscriber on `agent::after_tool_call`. Writes one
//! JSON object per line to a configurable path with the shape
//! `{ ts_ms, tool_call, result }`.

use std::path::PathBuf;
use std::sync::Arc;

use iii_sdk::{
    FunctionRef, IIIError, RegisterFunctionMessage, RegisterTriggerInput, Trigger, TriggerRequest,
    III,
};
use serde_json::{json, Value};

const FN_AUDIT: &str = "policy::audit_log";
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

pub fn subscribe_audit_log(iii: &III, log_path: PathBuf) -> Result<Subscriber, IIIError> {
    let log_path = Arc::new(log_path);
    let iii_for_handler = iii.clone();
    let function = iii.register_function((
        RegisterFunctionMessage::with_id(FN_AUDIT.into())
            .with_description("Append every tool call + result to a JSON-lines audit log.".into()),
        move |payload: Value| {
            let log_path = log_path.clone();
            let iii = iii_for_handler.clone();
            async move {
                let (event_id, reply_stream, inner) = unwrap_envelope(&payload);
                let line = json!({
                    "ts_ms": chrono::Utc::now().timestamp_millis(),
                    "tool_call": inner.get("tool_call").cloned().unwrap_or(Value::Null),
                    "result": inner.get("result").cloned().unwrap_or(Value::Null),
                });
                let _ = append_jsonl(&log_path, &line).await;
                let reply = json!({ "ok": true });
                write_hook_reply(&iii, &reply_stream, &event_id, &reply).await;
                Ok(reply)
            }
        },
    ));
    let trigger = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "subscribe".into(),
        function_id: FN_AUDIT.into(),
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

/// Per-path mutex map. POSIX `O_APPEND` is atomic only up to PIPE_BUF (4096
/// bytes). Tool results routinely exceed that, so concurrent subscribers
/// writing the same audit log can interleave bytes. We serialise writes per
/// path with a process-wide mutex map. Different paths still write
/// concurrently.
fn audit_log_locks(
) -> &'static std::sync::Mutex<std::collections::HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>> {
    static LOCKS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
    > = std::sync::OnceLock::new();
    LOCKS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

async fn append_jsonl(path: &PathBuf, line: &Value) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;

    let lock = {
        let mut map = audit_log_locks().lock().expect("audit_log_locks poisoned");
        map.entry(path.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    let _guard = lock.lock().await;

    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    let mut s = serde_json::to_vec(line).unwrap_or_default();
    s.push(b'\n');
    f.write_all(&s).await?;
    Ok(())
}
