//! Shared data structures used across the state worker.

use iii_helpers::stream::UpdateOp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StateSetInput {
    /// Namespace that groups related keys (e.g. `users`, `orders`).
    pub scope: String,
    /// Identifier for the value within the scope.
    pub key: String,
    /// Arbitrary JSON value to store. Replaces any existing value at `scope`/`key`.
    #[serde(alias = "data")]
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StateGetInput {
    /// Namespace that groups related keys.
    pub scope: String,
    /// Identifier for the value within the scope.
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StateDeleteInput {
    /// Namespace that groups related keys.
    pub scope: String,
    /// Identifier for the value to delete within the scope.
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StateUpdateInput {
    /// Namespace that groups related keys.
    pub scope: String,
    /// Identifier for the value to update within the scope.
    pub key: String,
    /// Ordered list of update operations applied atomically to the existing value.
    pub ops: Vec<UpdateOp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StateGetGroupInput {
    /// Namespace whose keys should be listed as a group.
    pub scope: String,
}

/// Immutable read result. Cloning keeps the selected version alive without
/// copying its JSON tree. Serialization and schema are exactly a raw JSON Value.
/// The worker has one read path for both direct callers and registered handlers.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub struct StateValue(pub std::sync::Arc<Value>);

impl From<Value> for StateValue {
    fn from(value: Value) -> Self {
        Self(std::sync::Arc::new(value))
    }
}

impl AsRef<Value> for StateValue {
    fn as_ref(&self) -> &Value {
        &self.0
    }
}

impl std::ops::Deref for StateValue {
    type Target = Value;
    fn deref(&self) -> &Value {
        self.as_ref()
    }
}

impl JsonSchema for StateValue {
    fn schema_name() -> String {
        <Value as JsonSchema>::schema_name()
    }
    fn json_schema(generator: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        <Value as JsonSchema>::json_schema(generator)
    }
    fn is_referenceable() -> bool {
        <Value as JsonSchema>::is_referenceable()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StateListGroupsInput {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StateListGroupsResult {
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StateListKeysResult {
    /// Keys stored in the scope, in the adapter's natural order.
    pub keys: Vec<String>,
}

/// Start a bounded snapshot, or continue with the preceding opaque cursor.
/// Repeat the same limits on all pages. Identity is overwritten by the engine.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StateListEntriesInput {
    /// Snapshot scope; accepted length at most 1024 UTF-8 bytes.
    #[schemars(length(max = 1024))]
    pub scope: String,
    /// Previous opaque continuation; omit to start a new snapshot.
    #[schemars(length(max = 36))]
    pub cursor: Option<String>,
    /// Maximum entries, default 100; accepted range 1..=1000.
    #[schemars(range(min = 1, max = 1000))]
    pub limit: Option<usize>,
    /// Complete serialized UTF-8 JSON response budget, default 1,000,000.
    #[schemars(range(min = 256, max = 8000000))]
    pub max_bytes: Option<usize>,
    #[serde(rename = "_caller_worker_id", default)]
    pub caller_worker_id: Option<String>,
}

/// Private opt-in only: filter captured JSON nulls before snapshot admission.
/// The public request schema deliberately has no filter option.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StatePrivateListEntriesInput {
    #[serde(flatten)]
    pub page: StateListEntriesInput,
    /// Default false includes stored nulls. Repeat the mode on every page.
    #[serde(default)]
    pub non_null_only: bool,
}

/// Key/value pairs captured together; nulls included unless privately opted out.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct StateListEntriesResult {
    pub entries: Vec<(String, StateValue)>,
    /// Opaque single-use continuation; null exactly when done is true.
    #[schemars(required)]
    pub next_cursor: Option<String>,
    pub done: bool,
    /// Number of rows preceding this page in the immutable snapshot.
    pub offset: usize,
    /// Total rows in this snapshot, stable across all pages.
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StateEventType {
    #[serde(rename = "state:created")]
    Created,
    #[serde(rename = "state:updated")]
    Updated,
    #[serde(rename = "state:deleted")]
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateEventData {
    #[serde(rename = "type")]
    pub message_type: String,
    pub event_type: StateEventType,
    pub scope: String,
    pub key: String,
    pub old_value: Option<Value>,
    pub new_value: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn state_set_input_data_alias() {
        let json = json!({"scope": "s", "key": "k", "data": "hello"});
        let input: StateSetInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.value, json!("hello"));
    }

    #[test]
    fn state_set_input_value_field() {
        let json = json!({"scope": "s", "key": "k", "value": 42});
        let input: StateSetInput = serde_json::from_value(json).unwrap();
        assert_eq!(input.value, json!(42));
    }

    #[test]
    fn state_event_type_serde() {
        let created = StateEventType::Created;
        let json = serde_json::to_value(&created).unwrap();
        assert_eq!(json, json!("state:created"));

        let back: StateEventType = serde_json::from_value(json!("state:updated")).unwrap();
        assert!(matches!(back, StateEventType::Updated));

        let deleted: StateEventType = serde_json::from_value(json!("state:deleted")).unwrap();
        assert!(matches!(deleted, StateEventType::Deleted));
    }

    #[test]
    fn state_event_data_roundtrip() {
        let json = json!({
            "type": "state_event",
            "event_type": "state:created",
            "scope": "users",
            "key": "user-1",
            "old_value": null,
            "new_value": {"name": "Alice"}
        });
        let data: StateEventData = serde_json::from_value(json).unwrap();
        assert_eq!(data.message_type, "state_event");
        assert!(matches!(data.event_type, StateEventType::Created));
        assert!(data.old_value.is_none());
        let back = serde_json::to_value(&data).unwrap();
        assert_eq!(back["type"], "state_event");
    }

    #[test]
    fn state_event_data_serializes_runtime_message_type() {
        let data = StateEventData {
            message_type: "state".to_string(),
            event_type: StateEventType::Created,
            scope: "users".to_string(),
            key: "user-1".to_string(),
            old_value: None,
            new_value: json!({"name": "Alice"}),
        };

        let json = serde_json::to_value(data).unwrap();
        assert_eq!(json["type"], "state");
        assert_eq!(json["event_type"], "state:created");
    }

    #[test]
    fn state_get_delete_group_roundtrip() {
        let _get: StateGetInput =
            serde_json::from_value(json!({"scope": "s", "key": "k"})).unwrap();
        let _del: StateDeleteInput =
            serde_json::from_value(json!({"scope": "s", "key": "k"})).unwrap();
        let _group: StateGetGroupInput = serde_json::from_value(json!({"scope": "s"})).unwrap();
        let _list: StateListGroupsInput = serde_json::from_value(json!({})).unwrap();
    }
}
