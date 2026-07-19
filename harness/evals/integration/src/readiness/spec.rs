use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ReadinessSpec {
    /// Exact function ids that must be registered. Internal functions are
    /// visible because the probe passes `include_internal: true`.
    pub functions: Vec<String>,
    /// Trigger types that must be registered (e.g. `harness::turn-completed`).
    pub trigger_types: Vec<String>,
    /// Queue topics that must exist as (name, expected broker type).
    pub queue_topics: Vec<(String, String)>,
    /// `configuration::get` id → expected seeded value (canonical-JSON
    /// byte-compare).
    pub config_entries: Vec<(String, Value)>,
}

impl ReadinessSpec {
    /// The surface required before Arm — everything except the harness,
    /// which is spawned after Arm (see `stack::WORKER_START_ORDER`).
    pub fn pre_harness(config_entries: Vec<(String, Value)>) -> Self {
        let functions = [
            // Session durability.
            "session::messages",
            // Context manager is mandatory and fails closed when absent.
            "context::assemble",
            "context::count-tokens",
            // The scripted router owns the fixed router ids.
            "router::chat",
            "router::abort",
            "router::models::list",
            "router::models::get",
            "router::models::supports",
            "router::system_prompt::get",
            // Recorder's only public engine surface. Configuration,
            // reset, and snapshots stay inside the runner process.
            "integration-recorder::lifecycle",
            // Queue surface consumed by the probe itself.
            "engine::queue::list_topics",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        Self {
            functions,
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
            functions: vec!["harness::send".to_string(), "harness::status".to_string()],
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
