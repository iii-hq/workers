//! Append-only audit-log subscriber on `agent::after_tool_call`. Writes one
//! JSON object per line to a configurable path with the shape
//! `{ ts_ms, tool_call, result }`.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
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

/// Bus surface needed by the audit-log handler — reply on a stream plus
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

/// Build the canonical [`RegisterFunctionMessage`] for the audit-log function.
pub(crate) fn audit_function_message() -> RegisterFunctionMessage {
    RegisterFunctionMessage::with_id(FN_AUDIT.into())
        .with_description("Append every tool call + result to a JSON-lines audit log.".into())
}

/// Build the canonical [`RegisterTriggerInput`] for the audit-log subscriber.
pub(crate) fn audit_trigger_input() -> RegisterTriggerInput {
    RegisterTriggerInput {
        trigger_type: "subscribe".into(),
        function_id: FN_AUDIT.into(),
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

/// Run the audit handler logic against an arbitrary [`ReplyBus`]. Used by
/// the production closure (with [`IiiSdkBus`]) and by tests (with the
/// in-memory bus).
pub(crate) async fn handle_event(bus: &dyn ReplyBus, log_path: &PathBuf, payload: Value) -> Value {
    let (event_id, reply_stream, inner) = unwrap_envelope(&payload);
    let line = json!({
        "ts_ms": chrono::Utc::now().timestamp_millis(),
        "tool_call": inner.get("tool_call").cloned().unwrap_or(Value::Null),
        "result": inner.get("result").cloned().unwrap_or(Value::Null),
    });
    let _ = append_jsonl(log_path, &line).await;
    let reply = json!({ "ok": true });
    write_hook_reply(bus, &reply_stream, &event_id, &reply).await;
    reply
}

pub fn subscribe_audit_log(iii: &III, log_path: PathBuf) -> Result<Subscriber, IIIError> {
    let bus: Arc<dyn ReplyBus> = Arc::new(IiiSdkBus(iii.clone()));
    let log_path = Arc::new(log_path);

    let fn_msg = audit_function_message();
    bus.record_function(&fn_msg);
    let bus_for_handler = bus.clone();
    let function = iii.register_function((fn_msg, move |payload: Value| {
        let bus = bus_for_handler.clone();
        let log_path = log_path.clone();
        async move {
            let reply = handle_event(bus.as_ref(), log_path.as_ref(), payload).await;
            Ok(reply)
        }
    }));

    let trig_input = audit_trigger_input();
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

    /// Drive the wiring side-effects (record_function + record_trigger) against
    /// `bus`. Mirrors what `subscribe_audit_log` does at register-time but
    /// without touching a live `iii` engine.
    pub(crate) fn record_wiring(bus: &dyn ReplyBus) {
        bus.record_function(&audit_function_message());
        bus.record_trigger(&audit_trigger_input());
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
        assert_eq!(fns[0].id, FN_AUDIT);
        assert_eq!(fns[0].id, "policy::audit_log");
        assert!(fns[0].description.is_some());

        let trigs = bus.recorded_triggers();
        assert_eq!(trigs.len(), 1);
        assert_eq!(trigs[0].trigger_type, "subscribe");
        assert_eq!(trigs[0].function_id, FN_AUDIT);
        assert_eq!(
            trigs[0].config.get("topic").and_then(Value::as_str),
            Some(TOPIC_AFTER)
        );
    }

    #[tokio::test]
    async fn handler_writes_jsonl_and_replies_ok() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_path = dir.path().join("audit.jsonl");
        let bus = InMemoryBus::new();

        let inner = json!({
            "tool_call": { "name": "x" },
            "result": { "content": [], "details": {}, "terminate": false },
        });
        let payload = envelope("e1", "rs", inner.clone());
        let reply = handle_event(&bus, &log_path, payload).await;
        assert_eq!(reply, json!({ "ok": true }));

        let raw = tokio::fs::read_to_string(&log_path).await.expect("read");
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), 1, "exactly one jsonl line");
        let v: Value = serde_json::from_str(lines[0]).expect("json line");
        assert_eq!(v.get("tool_call"), Some(&inner["tool_call"]));
        assert_eq!(v.get("result"), Some(&inner["result"]));
        assert!(v.get("ts_ms").and_then(Value::as_i64).is_some());

        let replies = bus.recorded_replies();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].stream_name, "rs");
        assert_eq!(replies[0].group_id, "e1");
        assert_eq!(replies[0].data, json!({ "ok": true }));
    }

    #[tokio::test]
    async fn handler_skips_reply_when_event_id_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_path = dir.path().join("audit.jsonl");
        let bus = InMemoryBus::new();

        let inner = json!({ "tool_call": {}, "result": {} });
        let payload = json!({
            "reply_stream": "rs",
            "payload": inner,
        });
        let reply = handle_event(&bus, &log_path, payload).await;
        assert_eq!(reply, json!({ "ok": true }));
        assert!(bus.recorded_replies().is_empty());
    }

    #[tokio::test]
    async fn handler_skips_reply_when_reply_stream_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_path = dir.path().join("audit.jsonl");
        let bus = InMemoryBus::new();

        let inner = json!({ "tool_call": {}, "result": {} });
        let payload = json!({
            "event_id": "e1",
            "payload": inner,
        });
        let reply = handle_event(&bus, &log_path, payload).await;
        assert_eq!(reply, json!({ "ok": true }));
        assert!(bus.recorded_replies().is_empty());
    }

    #[tokio::test]
    async fn handler_treats_missing_inner_payload_as_top_level() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_path = dir.path().join("audit.jsonl");
        let bus = InMemoryBus::new();

        // No `payload` wrapper — unwrap_envelope falls through to the
        // top-level value, which has neither `tool_call` nor `result`.
        let payload = json!({
            "event_id": "e1",
            "reply_stream": "rs",
        });
        let reply = handle_event(&bus, &log_path, payload).await;
        assert_eq!(reply, json!({ "ok": true }));

        let raw = tokio::fs::read_to_string(&log_path).await.expect("read");
        let v: Value = serde_json::from_str(raw.lines().next().expect("line")).expect("json");
        assert_eq!(v.get("tool_call"), Some(&Value::Null));
        assert_eq!(v.get("result"), Some(&Value::Null));
    }

    #[tokio::test]
    async fn handler_creates_missing_parent_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_path = dir.path().join("nested").join("deeper").join("audit.jsonl");
        let bus = InMemoryBus::new();

        let payload = envelope("e1", "rs", json!({ "tool_call": {}, "result": {} }));
        let reply = handle_event(&bus, &log_path, payload).await;
        assert_eq!(reply, json!({ "ok": true }));
        assert!(log_path.exists());
    }

    /// Tracks the bug recorded in workers/TODOS.md: when the JSONL write
    /// fails (e.g. read-only mount), the handler today still replies
    /// `{"ok": true}` and the audit gap is invisible to subscribers.
    #[tokio::test]
    async fn handler_silent_on_unwritable_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut perms = std::fs::metadata(dir.path()).expect("meta").permissions();
        // Read-only directory — open() with create+append should fail.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o555);
        }
        #[cfg(not(unix))]
        {
            perms.set_readonly(true);
        }
        std::fs::set_permissions(dir.path(), perms).expect("chmod");

        let log_path = dir.path().join("audit.jsonl");
        let bus = InMemoryBus::new();
        let payload = envelope("e1", "rs", json!({ "tool_call": {}, "result": {} }));
        let reply = handle_event(&bus, &log_path, payload).await;

        // Bug: handler still says "ok" even though the line was never written.
        // When the bug is fixed, this assertion flips to `{"ok": false, "error": ...}`.
        assert_eq!(reply, json!({ "ok": true }));

        // Restore perms so tempdir cleanup can delete the directory.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut p = std::fs::metadata(dir.path()).expect("meta").permissions();
            p.set_mode(0o755);
            std::fs::set_permissions(dir.path(), p).expect("chmod restore");
        }
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
