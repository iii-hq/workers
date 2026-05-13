use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

pub mod config;
pub mod handler;

pub const PUSH_ID: &str = "session-inbox::push";
pub const DRAIN_ID: &str = "session-inbox::drain";
pub const PEEK_ID: &str = "session-inbox::peek";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxRequest {
    pub name: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushRequest {
    pub name: String,
    pub session_id: String,
    pub item: Value,
}

pub fn inbox_key(name: &str, session_id: &str) -> String {
    format!("session/{session_id}/{name}")
}

pub fn build_push_update_op(item: Value) -> Value {
    serde_json::json!({ "type": "append", "path": "", "value": item })
}

pub fn build_clear_set_op() -> Value {
    serde_json::json!({ "type": "set", "path": "", "value": [] })
}

pub fn register_with_iii(iii: &iii_sdk::III, cfg: &Arc<config::WorkerConfig>) {
    handler::register(iii, cfg);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use serde_json::json;

    #[derive(Default)]
    struct InMemoryInbox {
        items: BTreeMap<String, Vec<Value>>,
    }

    impl InMemoryInbox {
        fn push(&mut self, key: &str, item: Value) {
            self.items.entry(key.to_string()).or_default().push(item);
        }

        fn peek(&self, key: &str) -> Vec<Value> {
            self.items.get(key).cloned().unwrap_or_default()
        }

        fn drain(&mut self, key: &str) -> Vec<Value> {
            self.items.remove(key).unwrap_or_default()
        }
    }

    #[test]
    fn inbox_key_uses_session_scoped_namespace() {
        assert_eq!(inbox_key("steering", "s1"), "session/s1/steering");
        assert_eq!(inbox_key("followup", "s1"), "session/s1/followup");
    }

    #[test]
    fn push_then_peek_keeps_item() {
        let mut store = InMemoryInbox::default();
        let key = inbox_key("steering", "s1");

        store.push(&key, json!({ "role": "user" }));

        assert_eq!(store.peek(&key), vec![json!({ "role": "user" })]);
        assert_eq!(store.peek(&key), vec![json!({ "role": "user" })]);
    }

    #[test]
    fn push_then_drain_removes_item() {
        let mut store = InMemoryInbox::default();
        let key = inbox_key("steering", "s1");

        store.push(&key, json!({ "role": "user" }));

        assert_eq!(store.drain(&key), vec![json!({ "role": "user" })]);
        assert!(store.peek(&key).is_empty());
    }

    #[test]
    fn drain_empty_returns_empty_array() {
        let mut store = InMemoryInbox::default();

        assert!(store.drain("missing").is_empty());
    }

    #[test]
    fn multiple_pushes_preserve_order() {
        let mut store = InMemoryInbox::default();
        let key = inbox_key("followup", "s1");

        store.push(&key, json!(1));
        store.push(&key, json!(2));
        store.push(&key, json!(3));

        assert_eq!(store.drain(&key), vec![json!(1), json!(2), json!(3)]);
    }

    #[test]
    fn push_builds_append_update_op() {
        let op = build_push_update_op(json!({"role": "user"}));

        assert_eq!(op["type"], "append");
        assert_eq!(op["path"], "");
        assert_eq!(op["value"]["role"], "user");
    }

    #[test]
    fn clear_builds_empty_set_op() {
        let op = build_clear_set_op();

        assert_eq!(op["type"], "set");
        assert_eq!(op["path"], "");
        assert_eq!(op["value"], json!([]));
    }
}
