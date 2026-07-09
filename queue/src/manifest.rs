//! Worker manifest generation.

use serde::Serialize;

use crate::config::QueueConfig;
use crate::functions::{
    DISCARD_MESSAGE_FN_ID, DLQ_MESSAGES_FN_ID, DLQ_TOPICS_FN_ID, ENQUEUE_FUNCTION_QUEUE_FN_ID,
    ENSURE_FUNCTION_QUEUE_FN_ID, LIST_TOPICS_FN_ID, PUBLISH_FN_ID, REDRIVE_FN_ID,
    REDRIVE_MESSAGE_FN_ID, TOPIC_STATS_FN_ID,
};
use crate::TRIGGER_TYPE;

#[derive(Debug, Serialize)]
pub struct ModuleManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub default_config: serde_json::Value,
    pub config_schema: serde_json::Value,
    pub trigger_types: Vec<String>,
    pub functions: Vec<String>,
    pub supported_targets: Vec<String>,
}

pub fn build_manifest() -> ModuleManifest {
    ModuleManifest {
        name: "queue".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: "Durable queue worker for iii.".to_string(),
        default_config: QueueConfig::default().to_json(),
        config_schema: QueueConfig::json_schema(),
        trigger_types: vec![TRIGGER_TYPE.to_string()],
        functions: vec![
            PUBLISH_FN_ID.to_string(),
            REDRIVE_FN_ID.to_string(),
            REDRIVE_MESSAGE_FN_ID.to_string(),
            DISCARD_MESSAGE_FN_ID.to_string(),
            LIST_TOPICS_FN_ID.to_string(),
            TOPIC_STATS_FN_ID.to_string(),
            DLQ_TOPICS_FN_ID.to_string(),
            DLQ_MESSAGES_FN_ID.to_string(),
            ENSURE_FUNCTION_QUEUE_FN_ID.to_string(),
            ENQUEUE_FUNCTION_QUEUE_FN_ID.to_string(),
        ],
        supported_targets: vec![option_env!("TARGET").unwrap_or("unknown").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_queue_identity_and_interfaces() {
        let value = serde_json::to_value(build_manifest()).unwrap();
        assert_eq!(value["name"], "queue");
        assert_eq!(value["trigger_types"][0], crate::TRIGGER_TYPE);
        assert!(value["functions"].as_array().unwrap().len() >= 8);
        assert!(value["config_schema"].is_object());
    }
}
