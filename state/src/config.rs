//! Worker configuration. Port of the builtin's StateModuleConfig
//! (engine/src/workers/state/config.rs) minus the `bridge` adapter branch.

use schemars::r#gen::SchemaGenerator;
use schemars::schema::{InstanceType, Schema, SchemaObject};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT_ADAPTER: &str = "kv";

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdapterEntry {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StateConfig {
    /// Storage adapter selection: `kv` (default; in-process, in_memory or
    /// file_based) or `redis`. Restart-tier: changing it at runtime is logged
    /// and takes effect at the next worker start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "state_adapter_schema")]
    pub adapter: Option<AdapterEntry>,

    /// Globally enable or disable state change-trigger fan-out. Applied live.
    /// Defaults to `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triggers_enabled: Option<bool>,

    /// Reject `state::set` writes whose JSON-serialized value exceeds this many
    /// bytes (VALUE_TOO_LARGE). Applied live. Unset means no limit.
    /// (Incremental `state::update` is not size-guarded — builtin parity.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1))]
    pub max_value_bytes: Option<usize>,

    /// Persistence flush cadence (ms) for the file-backed `kv` adapter.
    /// Applied live by respawning the adapter's save loop. Defaults to 5000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 100, max = 3_600_000))]
    pub save_interval_ms: Option<u64>,
}

impl Default for StateConfig {
    fn default() -> Self {
        Self {
            adapter: None,
            triggers_enabled: Some(true),
            max_value_bytes: None,
            save_interval_ms: None,
        }
    }
}

impl StateConfig {
    /// Same clamping as the builtin's `normalized()` (config.rs:77-83): a
    /// hand-edited out-of-range knob falls back to its built-in default.
    pub fn normalized(mut self) -> Self {
        self.max_value_bytes = self.max_value_bytes.filter(|&n| n > 0);
        self.save_interval_ms = self
            .save_interval_ms
            .filter(|&n| (100..=3_600_000).contains(&n));
        self
    }

    pub fn effective_adapter_name(&self) -> &str {
        self.adapter
            .as_ref()
            .map(|a| a.name.as_str())
            .unwrap_or(DEFAULT_ADAPTER)
    }

    /// The adapter config blob the store is built from, with the authoritative
    /// top-level `save_interval_ms` folded in (port of
    /// `adapter_config_from_config`, state.rs:225-239).
    pub fn adapter_config(&self) -> Option<Value> {
        let mut blob = self.adapter.as_ref().and_then(|a| a.config.clone());
        if let Some(interval) = self.save_interval_ms {
            let entry = blob.get_or_insert_with(|| Value::Object(Default::default()));
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("save_interval_ms".to_string(), Value::from(interval));
            }
        }
        blob
    }

    pub fn json_schema() -> Value {
        serde_json::to_value(schemars::schema_for!(StateConfig)).unwrap_or(Value::Null)
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn from_json(value: &Value) -> Result<Self, String> {
        serde_json::from_value::<Self>(value.clone())
            .map(Self::normalized)
            .map_err(|e| format!("invalid state configuration: {e}"))
    }
}

/// Storage backend for the file-backed `kv` adapter: `in_memory` (volatile,
/// process-lifetime storage, lost on shutdown — not for production) or
/// `file_based` (persisted under `file_path`, flushed on the `save_interval_ms`
/// cadence). Variants are intentionally doc-free so schemars emits a flat
/// string `enum` (a single select) rather than a per-variant `oneOf` that a
/// schema-driven UI renders as "variant 1", "variant 2".
#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KvStoreMethod {
    InMemory,
    FileBased,
}

/// Configuration for the built-in `kv` storage adapter.
#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct KvAdapterConfig {
    /// Storage backend. `in_memory` (the default) keeps data only for the
    /// process lifetime; `file_based` persists it under `file_path`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_method: Option<KvStoreMethod>,

    /// Directory for file-based storage. Only used when `store_method` is
    /// `file_based`. Defaults to `kv_store_data.db`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,

    /// Persistence flush cadence in milliseconds for file-based storage;
    /// in-memory stores ignore it. Defaults to 5000. The top-level
    /// `save_interval_ms` overrides this at construction time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 100, max = 3_600_000))]
    pub save_interval_ms: Option<u64>,
}

/// Configuration for the built-in `redis` storage adapter.
#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct RedisAdapterConfig {
    /// Redis connection URL. Defaults to `redis://localhost:6379`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redis_url: Option<String>,
}

/// Build the `oneOf` schema for [`StateConfig::adapter`]: one branch per
/// built-in adapter, each pinned to its `name` discriminator and carrying that
/// adapter's concrete `config` schema. The set is closed — `configuration::set`
/// rejects any other adapter name — so the console renders per-adapter fields
/// instead of a free-form object. Deserialization stays permissive via the
/// `AdapterEntry` field type, so a hand-edited persisted file is still tolerated
/// at boot.
fn state_adapter_schema(generator: &mut SchemaGenerator) -> Schema {
    let branches = vec![
        adapter_branch("kv", generator.subschema_for::<KvAdapterConfig>()),
        adapter_branch("redis", generator.subschema_for::<RedisAdapterConfig>()),
    ];

    let mut schema = SchemaObject::default();
    schema.metadata().description = Some(
        "Storage adapter selection and its adapter-specific config, advertised as a \
         discriminated union keyed on `name` over the built-in adapters `kv` (default) and \
         `redis`. Restart-tier: changing it at runtime is logged and takes \
         effect at the next engine start (the persisted entry is read at boot)."
            .to_string(),
    );
    schema.subschemas().one_of = Some(branches);
    Schema::Object(schema)
}

/// One `oneOf` branch: an object pinned to `name` and carrying the adapter's
/// `config` sub-schema. `config` is optional (every adapter has working
/// defaults) and no other keys are permitted.
fn adapter_branch(name: &str, config_schema: Schema) -> Schema {
    let name_schema = SchemaObject {
        instance_type: Some(InstanceType::String.into()),
        enum_values: Some(vec![serde_json::Value::String(name.to_string())]),
        ..Default::default()
    };

    let mut branch = SchemaObject {
        instance_type: Some(InstanceType::Object.into()),
        ..Default::default()
    };
    // The console labels each `oneOf` option by its `title`; without it the
    // form shows the bare type ("object") for every adapter branch.
    branch.metadata().title = Some(name.to_string());
    {
        let object = branch.object();
        object
            .properties
            .insert("name".to_string(), Schema::Object(name_schema));
        object
            .properties
            .insert("config".to_string(), config_schema);
        object.required.insert("name".to_string());
        object.additional_properties = Some(Box::new(Schema::Bool(false)));
    }
    Schema::Object(branch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_config_enables_triggers_and_uses_kv() {
        let c = StateConfig::default();
        assert_eq!(c.triggers_enabled, Some(true));
        assert_eq!(c.effective_adapter_name(), "kv");
        assert!(c.max_value_bytes.is_none());
        assert!(c.save_interval_ms.is_none());
    }

    #[test]
    fn deny_unknown_fields_rejects_typos() {
        let r: Result<StateConfig, _> = serde_json::from_value(json!({"triggers_enabledd": true}));
        assert!(r.is_err());
    }

    #[test]
    fn normalized_zeroes_out_invalid_knobs() {
        let c = StateConfig {
            max_value_bytes: Some(0),
            save_interval_ms: Some(1), // below the 100ms floor
            ..Default::default()
        }
        .normalized();
        assert!(c.max_value_bytes.is_none());
        assert!(c.save_interval_ms.is_none());
    }

    #[test]
    fn adapter_config_folds_top_level_save_interval() {
        let c: StateConfig = serde_json::from_value(json!({
            "adapter": {"name": "kv", "config": {"store_method": "file_based"}},
            "save_interval_ms": 750
        }))
        .unwrap();
        let blob = c.adapter_config().expect("adapter config present");
        assert_eq!(blob["save_interval_ms"], 750);
        assert_eq!(blob["store_method"], "file_based");
    }

    #[test]
    fn adapter_config_injects_save_interval_without_inner_block() {
        let c: StateConfig = serde_json::from_value(json!({
            "adapter": {"name": "kv"}, "save_interval_ms": 900
        }))
        .unwrap();
        assert_eq!(c.adapter_config().unwrap()["save_interval_ms"], 900);
    }

    #[test]
    fn schema_is_closed_oneof_over_kv_and_redis() {
        let schema = StateConfig::json_schema();
        let branches = schema["properties"]["adapter"]["oneOf"].as_array().unwrap();
        assert_eq!(branches.len(), 2);
        let mut names: Vec<&str> = branches
            .iter()
            .map(|b| b["properties"]["name"]["enum"][0].as_str().unwrap())
            .collect();
        names.sort_unstable();
        assert_eq!(names, vec!["kv", "redis"]);
        assert_eq!(schema["properties"]["max_value_bytes"]["minimum"], json!(1.0));
        assert_eq!(schema["properties"]["save_interval_ms"]["minimum"], json!(100.0));
        assert_eq!(schema["additionalProperties"], json!(false));
    }

    #[test]
    fn json_roundtrip() {
        let c: StateConfig = serde_json::from_value(json!({
            "adapter": {"name": "redis", "config": {"redis_url": "redis://x:6379"}}
        }))
        .unwrap();
        let back = StateConfig::from_json(&c.to_json()).unwrap();
        assert_eq!(back.effective_adapter_name(), "redis");
    }
}
