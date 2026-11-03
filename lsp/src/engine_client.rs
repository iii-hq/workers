use dashmap::{DashMap, DashSet};
use iii_sdk::runtime::{IIIConnectionState, WorkerMetadata};
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::Value;
use std::sync::{Arc, Mutex};

use crate::engine_introspection::{self, FunctionInfo, ObserverInfo, WorkerInfo};

/// The sources the engine hosts: private functions it calls once per event.
/// `engine::functions::list` leaves the `engine::*` ones out and lists the
/// others only while the engine worker that hosts them runs, so the server
/// knows them all by id.
pub const ENGINE_SOURCES: &[(&str, &str)] = &[
    (
        "engine::on-functions-changed",
        "A function was registered or unregistered.",
    ),
    (
        "engine::on-workers-changed",
        "A worker connected or disconnected; filter on `input.event`.",
    ),
    (
        "engine::on-observers-changed",
        "An observer changed state: `{ observer_id, function_id, target, state, reason? }`.",
    ),
    (
        "observability::on-log",
        "A log record; filter on `input.severity_number`.",
    ),
    ("observability::on-span", "A finished span."),
    (
        "configuration::on-updated",
        "A configuration entry was registered, updated or deleted; filter on `input.id` and `input.event_type`.",
    ),
    (
        "stream::on-changed",
        "A stream item changed; filter on `input.stream_name`, `input.group_id`.",
    ),
    (
        "stream::on-join",
        "A client joined a stream; filter on `input.stream_name`, `input.group_id`.",
    ),
    (
        "stream::on-leave",
        "A client left a stream; filter on `input.stream_name`, `input.group_id`.",
    ),
];

/// The description of an engine-hosted source, when `id` names one.
pub fn engine_source(id: &str) -> Option<&'static str> {
    ENGINE_SOURCES
        .iter()
        .find(|(source, _)| *source == id)
        .map(|(_, description)| *description)
}

pub struct EngineClient {
    iii: IIIClient,
    pub functions: DashMap<String, FunctionInfo>,
    pub workers: DashMap<String, WorkerInfo>,
    /// Live observers, by id.
    pub observers: DashMap<String, ObserverInfo>,
    /// Known values read from observers' args and filters
    pub known_stream_names: DashSet<String>,
    pub known_topics: DashSet<String>,
    pub known_scopes: DashSet<String>,
    /// String values of observers' args, by observed function and arg name.
    pub known_args: DashMap<(String, String), DashSet<String>>,
    poll_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl EngineClient {
    pub fn new(url: &str) -> Arc<Self> {
        let iii = register_worker(
            url,
            InitOptions {
                metadata: Some(WorkerMetadata {
                    runtime: "rust".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    name: "lsp".to_string(),
                    os: std::env::consts::OS.to_string(),
                    pid: Some(std::process::id()),
                    telemetry: None,
                    ..Default::default()
                }),
                ..Default::default()
            },
        );

        Arc::new(Self {
            iii,
            functions: DashMap::new(),
            workers: DashMap::new(),
            observers: DashMap::new(),
            known_stream_names: DashSet::new(),
            known_topics: DashSet::new(),
            known_scopes: DashSet::new(),
            known_args: DashMap::new(),
            poll_task: Mutex::new(None),
        })
    }

    pub async fn start(self: &Arc<Self>) {
        self.seed_cache().await;

        let weak = Arc::downgrade(self);
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                let Some(client) = weak.upgrade() else {
                    break;
                };
                if !client.is_connected() {
                    continue;
                }
                if let Ok(functions) = engine_introspection::list_functions(&client.iii).await {
                    // A row carries its observer count, so an observer added
                    // or removed changes it too.
                    let changed = functions.len() != client.functions.len()
                        || functions.iter().any(|f| {
                            client
                                .functions
                                .get(&f.function_id)
                                .is_none_or(|existing| existing.value() != f)
                        });
                    if changed {
                        client.functions.clear();
                        for func in functions {
                            client.functions.insert(func.function_id.clone(), func);
                        }
                        client.reseed_secondary_caches().await;
                    }
                }
            }
        });

        *self.poll_task.lock().unwrap() = Some(handle);
    }

    async fn seed_cache(&self) {
        if let Ok(functions) = engine_introspection::list_functions(&self.iii).await {
            for func in functions {
                self.functions.insert(func.function_id.clone(), func);
            }
        }
        self.reseed_secondary_caches().await;
    }

    async fn reseed_secondary_caches(&self) {
        if let Ok(workers) = engine_introspection::list_workers(&self.iii).await {
            self.workers.clear();
            for w in workers {
                self.workers.insert(w.id.clone(), w);
            }
        }

        if let Ok(observers) = engine_introspection::list_observers(&self.iii).await {
            self.extract_known_values(&observers);
            self.observers.clear();
            for obs in observers {
                self.observers.insert(obs.id.clone(), obs);
            }
        }
    }

    /// Read known stream names, topics, scopes and arg values from the
    /// observers' args and filters.
    fn extract_known_values(&self, observers: &[ObserverInfo]) {
        self.known_stream_names.clear();
        self.known_topics.clear();
        self.known_scopes.clear();
        self.known_args.clear();

        for obs in observers {
            if let Some(args) = obs.args.as_object() {
                for (name, value) in args {
                    if let Some(value) = value.as_str() {
                        self.known_args
                            .entry((obs.function_id.clone(), name.clone()))
                            .or_default()
                            .insert(value.to_string());
                    }
                }
            }

            match obs.function_id.as_str() {
                "stream::on-changed" | "stream::on-join" | "stream::on-leave" => {
                    for name in filter_values(&obs.filters, "input.stream_name") {
                        self.known_stream_names.insert(name);
                    }
                }
                "queue::on-message" => {
                    if let Some(topic) = obs.args.get("topic").and_then(|v| v.as_str()) {
                        self.known_topics.insert(topic.to_string());
                    }
                }
                "publish" => {
                    for topic in filter_values(&obs.filters, "input.topic") {
                        self.known_topics.insert(topic);
                    }
                }
                "state::on-changed" => {
                    for scope in filter_values(&obs.filters, "input.scope") {
                        self.known_scopes.insert(scope);
                    }
                }
                _ => {}
            }
        }
    }

    /// Get known values for a field name (stream_name, topic, scope, queue)
    pub fn get_known_values(&self, field_name: &str) -> Vec<String> {
        match field_name {
            "stream_name" => self.known_stream_names.iter().map(|v| v.clone()).collect(),
            "topic" => self.known_topics.iter().map(|v| v.clone()).collect(),
            "scope" => self.known_scopes.iter().map(|v| v.clone()).collect(),
            "queue" => self.known_topics.iter().map(|v| v.clone()).collect(), // queues use topics
            _ => Vec::new(),
        }
    }

    /// The string values the observers of `function_id` pass as `arg`.
    pub fn get_arg_values(&self, function_id: &str, arg: &str) -> Vec<String> {
        self.known_args
            .get(&(function_id.to_string(), arg.to_string()))
            .map(|values| values.iter().map(|v| v.clone()).collect())
            .unwrap_or_default()
    }

    /// The targets of the observers of `function_id`, sorted.
    pub fn observer_targets(&self, function_id: &str) -> Vec<String> {
        let mut targets: Vec<String> = self
            .observers
            .iter()
            .filter(|entry| entry.value().function_id == function_id)
            .map(|entry| entry.value().target.function_id.clone())
            .collect();
        targets.sort();
        targets.dedup();
        targets
    }

    pub fn is_connected(&self) -> bool {
        matches!(
            self.iii.get_connection_state(),
            IIIConnectionState::Connected
        )
    }

    pub fn get_function(&self, id: &str) -> Option<FunctionInfo> {
        let entry = self.functions.get(id)?;
        Some(entry.value().clone())
    }

    pub fn find_worker_for_function(&self, function_id: &str) -> Option<WorkerInfo> {
        for entry in self.workers.iter() {
            let worker = entry.value();
            if worker.functions.contains(&function_id.to_string()) {
                return Some(worker.clone());
            }
        }
        None
    }

    pub async fn shutdown(&self) {
        if let Ok(mut guard) = self.poll_task.lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
        self.iii.shutdown_async().await;
    }
}

/// The string values a filter compares `path` with: the operands of its
/// `$eq` and the items of its `$in`, in the clauses of the filter and of its
/// `$and` and `$or` members (the engine's normal form is
/// `{ "$and": [ { path: { op: operand } }, … ] }`).
pub fn filter_values(filters: &Value, path: &str) -> Vec<String> {
    let mut values = Vec::new();
    collect_filter_values(filters, path, &mut values);
    values
}

fn collect_filter_values(filters: &Value, path: &str, values: &mut Vec<String>) {
    let Some(clauses) = filters.as_object() else {
        return;
    };
    for (key, operand) in clauses {
        match key.as_str() {
            "$and" | "$or" => {
                for member in operand.as_array().into_iter().flatten() {
                    collect_filter_values(member, path, values);
                }
            }
            _ if key == path => {
                if let Some(value) = operand.get("$eq").and_then(Value::as_str) {
                    values.push(value.to_string());
                }
                for item in operand
                    .get("$in")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(value) = item.as_str() {
                        values.push(value.to_string());
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_operations() {
        let functions: DashMap<String, FunctionInfo> = DashMap::new();
        let func = FunctionInfo {
            function_id: "test::hello".to_string(),
            description: Some("A test function".to_string()),
            request_format: None,
            response_format: None,
            metadata: None,
            ..Default::default()
        };
        functions.insert(func.function_id.clone(), func);

        assert_eq!(functions.len(), 1);
        let entry = functions.get("test::hello").unwrap();
        assert_eq!(
            entry.value().description.as_deref(),
            Some("A test function")
        );
    }

    #[test]
    fn cache_update_replaces_entries() {
        let functions: DashMap<String, FunctionInfo> = DashMap::new();
        functions.insert(
            "old::func".to_string(),
            FunctionInfo {
                function_id: "old::func".to_string(),
                description: None,
                request_format: None,
                response_format: None,
                metadata: None,
                ..Default::default()
            },
        );

        functions.clear();
        functions.insert(
            "new::func".to_string(),
            FunctionInfo {
                function_id: "new::func".to_string(),
                description: Some("New".to_string()),
                request_format: None,
                response_format: None,
                metadata: None,
                ..Default::default()
            },
        );

        assert_eq!(functions.len(), 1);
        assert!(functions.get("old::func").is_none());
        assert!(functions.get("new::func").is_some());
    }

    #[test]
    fn cache_empty_returns_none() {
        let functions: DashMap<String, FunctionInfo> = DashMap::new();
        assert!(functions.get("nonexistent").is_none());
    }

    #[test]
    fn filter_values_reads_the_normal_form() {
        let filters = serde_json::json!({
            "$and": [
                { "input.event_type": { "$eq": "set" } },
                { "input.scope": { "$eq": "orders" } },
            ]
        });
        assert_eq!(filter_values(&filters, "input.scope"), vec!["orders"]);
        assert!(filter_values(&filters, "input.key").is_empty());
    }

    #[test]
    fn filter_values_reads_in_or_and_written_filters() {
        let filters = serde_json::json!({
            "input.topic": { "$in": ["orders", "invoices", 3] },
            "$or": [{ "input.topic": { "$eq": "refunds" } }],
        });
        let mut values = filter_values(&filters, "input.topic");
        values.sort();
        assert_eq!(values, vec!["invoices", "orders", "refunds"]);
        assert!(filter_values(&serde_json::json!({}), "input.topic").is_empty());
    }

    #[test]
    fn engine_sources_are_known_by_id() {
        assert!(engine_source("engine::on-functions-changed").is_some());
        assert!(engine_source("configuration::on-updated").is_some());
        assert!(engine_source("todos::create").is_none());
    }
}
