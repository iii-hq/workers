//! Worker configuration: queue transport plus durable named function queues.

use std::collections::BTreeMap;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapter::FunctionQueueConfig;

pub const DEFAULT_ADAPTER: &str = "builtin";
/// The engine's default `observers.observer_grace_ms`: a worker that
/// reattaches, or restarts under the same name, within it re-observes its
/// subscribers' ids, and their queues have kept receiving their topics.
pub const DEFAULT_SUBSCRIBER_RETENTION_MS: u64 = 30_000;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdapterEntry {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueueConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<AdapterEntry>,
    /// Durable function queues keyed by the name used by
    /// `CallAction::Enqueue`. Definitions are persisted through the
    /// configuration worker and restored when this worker restarts.
    #[serde(default)]
    pub queue_configs: BTreeMap<String, FunctionQueueConfig>,
    /// The namespace a function-queue job runs in. Read at boot.
    #[serde(default, skip_serializing_if = "RunJobsIn::is_enqueuer")]
    pub run_jobs_in: RunJobsIn,
    /// How long, in milliseconds, the queue of a `queue::on-message`
    /// subscriber whose observer is removed (disposed, or its worker
    /// disconnected) keeps receiving its topic. Its consumer stops at once;
    /// an observer with the same id (or group) within this time consumes
    /// everything the queue received. After it, the queue keeps only what it
    /// holds. `0` stops the queue receiving when the observer is removed.
    /// The default is the engine's default `observers.observer_grace_ms`;
    /// keep it at least the engine's setting, so a worker that re-observes
    /// within the grace period finds everything published while it was away.
    #[serde(default = "default_subscriber_retention_ms")]
    pub subscriber_retention_ms: u64,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            adapter: None,
            queue_configs: BTreeMap::new(),
            run_jobs_in: RunJobsIn::default(),
            subscriber_retention_ms: DEFAULT_SUBSCRIBER_RETENTION_MS,
        }
    }
}

fn default_subscriber_retention_ms() -> u64 {
    DEFAULT_SUBSCRIBER_RETENTION_MS
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunJobsIn {
    /// The namespace the job was enqueued in.
    #[default]
    Enqueuer,
    /// The namespace of the queue worker that takes the job, whichever namespace enqueued it. A
    /// job whose function is not registered there goes back to the broker, unspent, for another
    /// worker. For one app scaled out as several namespaces, each with its own queue worker.
    Worker,
}

impl RunJobsIn {
    fn is_enqueuer(&self) -> bool {
        *self == Self::Enqueuer
    }
}

impl QueueConfig {
    pub fn effective_adapter_name(&self) -> &str {
        self.adapter
            .as_ref()
            .map(|a| a.name.as_str())
            .unwrap_or(DEFAULT_ADAPTER)
    }

    /// Registry/install seed. Direct library construction keeps the lightweight
    /// in-memory default, while installed workers persist accepted jobs.
    pub fn packaged_default() -> Self {
        Self {
            adapter: Some(AdapterEntry {
                name: DEFAULT_ADAPTER.to_string(),
                config: Some(serde_json::json!({
                    "store_method": "file_based",
                    "file_path": iii_worker_paths::default_path("data/queue"),
                    "save_interval_ms": 5000
                })),
            }),
            queue_configs: BTreeMap::new(),
            run_jobs_in: RunJobsIn::Enqueuer,
            subscriber_retention_ms: DEFAULT_SUBSCRIBER_RETENTION_MS,
        }
    }

    /// [`Self::subscriber_retention_ms`] as a duration.
    pub fn subscriber_retention(&self) -> Duration {
        Duration::from_millis(self.subscriber_retention_ms)
    }

    pub fn normalized(&self) -> Self {
        self.clone()
    }

    pub fn validate(&self) -> Result<(), String> {
        for (name, config) in &self.queue_configs {
            config.validate(name)?;
        }
        // Only a shared broker can give a job back for another worker.
        if self.run_jobs_in == RunJobsIn::Worker && self.effective_adapter_name() != "rabbitmq" {
            return Err(format!(
                "run_jobs_in worker needs the rabbitmq adapter, not '{}'",
                self.effective_adapter_name()
            ));
        }
        Ok(())
    }

    pub fn json_schema() -> Value {
        serde_json::to_value(schemars::schema_for!(QueueConfig)).unwrap_or(Value::Null)
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn from_json(value: &Value) -> Result<Self, String> {
        let config: Self = serde_json::from_value(value.clone())
            .map_err(|e| format!("invalid queue configuration: {e}"))?;
        config.validate()?;
        Ok(config)
    }
}

/// The active adapter's `config` object, Null-safe: an absent
/// `adapter`/`adapter.config`, and an explicit JSON `null`, both come back as
/// `None` rather than `Some(Value::Null)` -- adapter constructors
/// (`RedisAdapter::from_config`, `RabbitMQAdapter::from_config`, ...) only
/// ever need to distinguish "no config" from "an actual object".
pub fn adapter_config_value(config: &QueueConfig) -> Option<Value> {
    config
        .adapter
        .as_ref()
        .and_then(|adapter| adapter.config.clone())
        .filter(|value| !value.is_null())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_builtin_adapter() {
        let c = QueueConfig::default();
        assert_eq!(c.effective_adapter_name(), "builtin");
    }

    #[test]
    fn packaged_default_is_file_backed() {
        let config = QueueConfig::packaged_default();
        assert_eq!(config.effective_adapter_name(), "builtin");
        assert_eq!(
            config.adapter.unwrap().config.unwrap()["store_method"],
            "file_based"
        );
    }

    #[test]
    fn deserializes_adapter_entry() {
        let c: QueueConfig =
            serde_yaml::from_str("{adapter: {name: file_based, config: {path: './data/queue'}}}")
                .unwrap();
        assert_eq!(c.effective_adapter_name(), "file_based");
        assert_eq!(c.adapter.unwrap().config.unwrap()["path"], "./data/queue");
    }

    #[test]
    fn rejects_unknown_fields() {
        let r: Result<QueueConfig, _> = serde_yaml::from_str("{adapter: {name: builtin}, wat: 1}");
        assert!(r.is_err(), "deny_unknown_fields must reject 'wat'");
    }

    #[test]
    fn json_roundtrip() {
        let c: QueueConfig = serde_yaml::from_str("{adapter: {name: file_based}}").unwrap();
        let back = QueueConfig::from_json(&c.to_json()).unwrap();
        assert_eq!(back.effective_adapter_name(), "file_based");
    }

    #[test]
    fn adapter_config_value_none_without_adapter() {
        assert_eq!(adapter_config_value(&QueueConfig::default()), None);
    }

    #[test]
    fn adapter_config_value_none_without_config() {
        let c = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "redis".to_string(),
                config: None,
            }),
            ..QueueConfig::default()
        };
        assert_eq!(adapter_config_value(&c), None);
    }

    #[test]
    fn adapter_config_value_none_for_explicit_null() {
        let c = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "redis".to_string(),
                config: Some(serde_json::Value::Null),
            }),
            ..QueueConfig::default()
        };
        assert_eq!(adapter_config_value(&c), None);
    }

    #[test]
    fn adapter_config_value_some_for_object() {
        let c = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "redis".to_string(),
                config: Some(serde_json::json!({"redis_url": "redis://x"})),
            }),
            ..QueueConfig::default()
        };
        assert_eq!(
            adapter_config_value(&c),
            Some(serde_json::json!({"redis_url": "redis://x"}))
        );
    }

    #[test]
    fn run_jobs_in_defaults_to_enqueuer_and_worker_needs_rabbitmq() {
        assert_eq!(QueueConfig::default().run_jobs_in, RunJobsIn::Enqueuer);
        assert!(QueueConfig::default()
            .to_json()
            .get("run_jobs_in")
            .is_none());
        let c: QueueConfig =
            serde_yaml::from_str("{adapter: {name: rabbitmq}, run_jobs_in: worker}").unwrap();
        assert_eq!(c.run_jobs_in, RunJobsIn::Worker);
        assert_eq!(QueueConfig::from_json(&c.to_json()).unwrap(), c);
        let builtin: QueueConfig = serde_yaml::from_str("{run_jobs_in: worker}").unwrap();
        assert!(
            builtin.validate().is_err(),
            "only a shared broker can give a job back"
        );
    }

    #[test]
    fn subscriber_retention_defaults_to_the_engine_grace_period() {
        let defaults: QueueConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(defaults, QueueConfig::default());
        assert_eq!(defaults.subscriber_retention_ms, 30_000);
        assert_eq!(defaults.subscriber_retention(), Duration::from_secs(30));
        assert_eq!(
            QueueConfig::packaged_default().subscriber_retention_ms,
            30_000
        );
        assert_eq!(
            QueueConfig::default().to_json()["subscriber_retention_ms"],
            30_000
        );

        let none: QueueConfig = serde_yaml::from_str("{subscriber_retention_ms: 0}").unwrap();
        none.validate().unwrap();
        assert_eq!(none.subscriber_retention(), Duration::ZERO);
        assert_eq!(QueueConfig::from_json(&none.to_json()).unwrap(), none);
        assert_eq!(none.to_json()["subscriber_retention_ms"], 0);

        for invalid in [
            serde_json::json!({ "subscriber_retention_ms": -1 }),
            serde_json::json!({ "subscriber_retention_ms": 1.5 }),
            serde_json::json!({ "subscriber_retention_ms": "30s" }),
            serde_json::json!({ "subscriber_retention_ms": null }),
        ] {
            assert!(QueueConfig::from_json(&invalid).is_err(), "{invalid}");
        }

        let schema = QueueConfig::json_schema();
        let field = &schema["properties"]["subscriber_retention_ms"];
        assert_eq!(field["default"], 30_000, "{schema}");
        assert_eq!(field["minimum"], 0.0, "{schema}");
    }

    #[test]
    fn named_queue_config_roundtrips() {
        let c: QueueConfig = serde_yaml::from_str(
            "queue_configs:\n  harness-turn:\n    type: fifo\n    message_group_field: session_id\n",
        )
        .unwrap();
        c.validate().unwrap();
        assert_eq!(
            c.queue_configs["harness-turn"]
                .message_group_field
                .as_deref(),
            Some("session_id")
        );
        assert_eq!(QueueConfig::from_json(&c.to_json()).unwrap(), c);
    }
}
