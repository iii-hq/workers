use serde_json::Value;

use super::contracts::{harness_contracts, pre_harness_contracts};
use super::ExpectedFunctionContract;

#[derive(Debug, Clone)]
pub struct ReadinessSpec {
    /// Exact live function contracts that must be registered. Internal
    /// functions are visible because discovery passes
    /// `include_internal: true`.
    pub functions: Vec<ExpectedFunctionContract>,
    /// Trigger types that must be registered (e.g. `harness::turn-completed`).
    pub trigger_types: Vec<String>,
    /// Queue topics that must exist as (name, expected broker type).
    pub queue_topics: Vec<(String, String)>,
    /// `configuration::get` id → authoritative seeded subset. Every seeded
    /// value must match canonically; worker-installed defaults may add fields.
    pub config_entries: Vec<(String, Value)>,
}

impl ReadinessSpec {
    /// The surface required before Arm — everything except the harness,
    /// which is spawned after Arm (see `stack::WORKER_START_ORDER`).
    pub fn pre_harness(config_entries: Vec<(String, Value)>) -> Self {
        Self {
            functions: pre_harness_contracts(),
            trigger_types: Vec::new(),
            queue_topics: Vec::new(),
            config_entries,
        }
    }

    /// The harness surface, probed after `Stack::spawn_harness` and before
    /// Send: public functions, lifecycle trigger types, the provisioned
    /// `harness-turn` topic, and the harness's own seeded config entry.
    pub fn harness_surface(config_entries: Vec<(String, Value)>) -> Self {
        Self {
            functions: harness_contracts(),
            trigger_types: vec![
                "harness::turn-started".to_string(),
                "harness::turn-completed".to_string(),
            ],
            queue_topics: vec![("harness-turn".to_string(), "builtin".to_string())],
            config_entries,
        }
    }
}

#[derive(Debug)]
pub struct ReadinessReport {
    /// Empty when ready. Each entry names one missing/mismatched surface.
    pub missing: Vec<String>,
}
