//! Operator-facing A2UI worker configuration.
//!
//! The `configuration` worker owns the authoritative value. An optional YAML
//! file only seeds first registration; all handlers read the hot-swapped
//! snapshot on every call.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// Optional model override for the UI composer. When unset, the worker
    /// reads the current Harness turn's routed model from durable turn state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composer_model: Option<String>,

    /// Optional provider override paired with `composer_model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composer_provider: Option<String>,

    /// Maximum output tokens for one composition or repair call.
    #[serde(default = "default_max_output_tokens")]
    pub max_output_tokens: u64,

    /// Maximum UTF-8 bytes sent as one composer user prompt.
    #[serde(default = "default_max_composer_input_bytes")]
    pub max_composer_input_bytes: usize,

    /// Additional correction calls after the first invalid model response.
    #[serde(default = "default_repair_attempts")]
    pub repair_attempts: u8,

    /// Maximum surfaces retained for one Harness session.
    #[serde(default = "default_max_surfaces")]
    pub max_surfaces_per_session: usize,

    /// Maximum restorable snapshots kept for one surface.
    #[serde(default = "default_max_history")]
    pub max_history_per_surface: usize,

    /// Maximum reusable templates retained for one Harness session.
    #[serde(default = "default_max_templates")]
    pub max_templates_per_session: usize,

    /// Maximum flat A2UI components retained on one surface.
    #[serde(default = "default_max_components")]
    pub max_components_per_surface: usize,

    /// Maximum UTF-8 bytes in a generative UI description.
    #[serde(default = "default_max_description_bytes")]
    pub max_description_bytes: usize,

    /// Maximum serialized bytes in any A2UI data model or action context.
    #[serde(default = "default_max_data_bytes")]
    pub max_data_bytes: usize,

    /// Maximum serialized bytes retained for one surface, including history.
    #[serde(default = "default_max_surface_bytes")]
    pub max_surface_bytes: usize,

    /// Maximum serialized bytes retained for one Harness session.
    #[serde(default = "default_max_session_bytes")]
    pub max_session_bytes: usize,

    /// Send Console actions back into the originating Harness session as a
    /// structured custom message.
    #[serde(default = "default_forward_actions")]
    pub forward_actions: bool,
}

fn default_max_output_tokens() -> u64 {
    8_192
}

fn default_max_composer_input_bytes() -> usize {
    768 * 1024
}

fn default_repair_attempts() -> u8 {
    1
}

fn default_max_surfaces() -> usize {
    16
}

fn default_max_history() -> usize {
    64
}

fn default_max_templates() -> usize {
    32
}

fn default_max_components() -> usize {
    160
}

fn default_max_description_bytes() -> usize {
    32 * 1024
}

fn default_max_data_bytes() -> usize {
    512 * 1024
}

fn default_max_surface_bytes() -> usize {
    2 * 1024 * 1024
}

fn default_max_session_bytes() -> usize {
    16 * 1024 * 1024
}

fn default_forward_actions() -> bool {
    true
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            composer_model: None,
            composer_provider: None,
            max_output_tokens: default_max_output_tokens(),
            max_composer_input_bytes: default_max_composer_input_bytes(),
            repair_attempts: default_repair_attempts(),
            max_surfaces_per_session: default_max_surfaces(),
            max_history_per_surface: default_max_history(),
            max_templates_per_session: default_max_templates(),
            max_components_per_surface: default_max_components(),
            max_description_bytes: default_max_description_bytes(),
            max_data_bytes: default_max_data_bytes(),
            max_surface_bytes: default_max_surface_bytes(),
            max_session_bytes: default_max_session_bytes(),
            forward_actions: default_forward_actions(),
        }
    }
}

impl WorkerConfig {
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        let expanded = expand_env(yaml);
        let parsed: Self =
            serde_yaml::from_str(&expanded).map_err(|e| format!("yaml parse: {e}"))?;
        parsed.validate()
    }

    pub fn from_file(path: &str) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::from_yaml(&raw)
    }

    pub fn from_json(value: &Value) -> Result<Self, String> {
        let parsed: Self =
            serde_json::from_value(value.clone()).map_err(|e| format!("json parse: {e}"))?;
        parsed.validate()
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("WorkerConfig serializes")
    }

    pub fn json_schema() -> Value {
        let root = schemars::schema_for!(WorkerConfig);
        let mut schema =
            serde_json::to_value(&root.schema).expect("WorkerConfig JSON Schema serializes");
        if let Some(obj) = schema.as_object_mut() {
            if !root.definitions.is_empty() {
                obj.insert(
                    "definitions".into(),
                    serde_json::to_value(&root.definitions).expect("definitions serialize"),
                );
            }
            obj.insert("example".into(), WorkerConfig::default().to_json());
        }
        schema
    }

    fn validate(self) -> Result<Self, String> {
        if self.max_output_tokens == 0 {
            return Err("max_output_tokens must be at least 1".into());
        }
        if self.repair_attempts > 3 {
            return Err("repair_attempts must be between 0 and 3".into());
        }
        for (name, value) in [
            ("max_surfaces_per_session", self.max_surfaces_per_session),
            ("max_composer_input_bytes", self.max_composer_input_bytes),
            ("max_history_per_surface", self.max_history_per_surface),
            ("max_templates_per_session", self.max_templates_per_session),
            (
                "max_components_per_surface",
                self.max_components_per_surface,
            ),
            ("max_description_bytes", self.max_description_bytes),
            ("max_data_bytes", self.max_data_bytes),
            ("max_surface_bytes", self.max_surface_bytes),
            ("max_session_bytes", self.max_session_bytes),
        ] {
            if value == 0 {
                return Err(format!("{name} must be at least 1"));
            }
        }
        if self.max_surface_bytes < self.max_data_bytes {
            return Err("max_surface_bytes must be at least max_data_bytes".into());
        }
        if self.max_session_bytes < self.max_surface_bytes {
            return Err("max_session_bytes must be at least max_surface_bytes".into());
        }
        Ok(self)
    }
}

fn expand_env(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let spec = &after[..end];
                let (name, fallback) = spec
                    .split_once(':')
                    .map_or((spec, None), |(name, fallback)| (name, Some(fallback)));
                match (std::env::var(name), fallback) {
                    (Ok(value), _) => out.push_str(&value),
                    (Err(_), Some(value)) => out.push_str(value),
                    (Err(_), None) => {
                        tracing::warn!(var = %name, "config references undefined env var")
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str("${");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_yields_defaults() {
        assert_eq!(
            WorkerConfig::from_yaml("{}").unwrap(),
            WorkerConfig::default()
        );
    }

    #[test]
    fn rejects_unknown_and_zero_fields() {
        assert!(WorkerConfig::from_yaml("composer_modle: x\n").is_err());
        assert!(
            WorkerConfig::from_json(&serde_json::json!({"max_components_per_surface": 0})).is_err()
        );
        assert!(WorkerConfig::from_json(&serde_json::json!({"repair_attempts": 4})).is_err());
        assert!(WorkerConfig::from_json(&serde_json::json!({
            "max_data_bytes": 8,
            "max_surface_bytes": 4
        }))
        .is_err());
    }

    #[test]
    fn json_round_trips_and_schema_carries_example() {
        let cfg = WorkerConfig {
            composer_model: Some("model-a".into()),
            ..WorkerConfig::default()
        };
        assert_eq!(WorkerConfig::from_json(&cfg.to_json()).unwrap(), cfg);
        assert_eq!(
            WorkerConfig::json_schema()["example"],
            WorkerConfig::default().to_json()
        );
    }
}
