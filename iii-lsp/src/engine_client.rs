use dashmap::{DashMap, DashSet};
use iii_sdk::{
    register_worker, FunctionInfo, FunctionsAvailableGuard, IIIConnectionState, InitOptions,
    TriggerInfo, TriggerTypeInfo, WorkerInfo, WorkerMetadata, III,
};
use std::sync::{Arc, Mutex};

pub struct EngineClient {
    iii: III,
    pub functions: DashMap<String, FunctionInfo>,
    pub trigger_types: DashMap<String, TriggerTypeInfo>,
    pub workers: DashMap<String, WorkerInfo>,
    /// Known values extracted from trigger configs
    pub known_stream_names: DashSet<String>,
    pub known_topics: DashSet<String>,
    pub known_api_paths: DashSet<String>,
    pub known_scopes: DashSet<String>,
    guard: Mutex<Option<FunctionsAvailableGuard>>,
}

impl EngineClient {
    pub fn new(url: &str) -> Arc<Self> {
        let iii = register_worker(
            url,
            InitOptions {
                metadata: Some(WorkerMetadata {
                    runtime: "rust".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    name: "iii-lsp".to_string(),
                    os: std::env::consts::OS.to_string(),
                    pid: Some(std::process::id()),
                    telemetry: None,
                }),
                ..Default::default()
            },
        );

        Arc::new(Self {
            iii,
            functions: DashMap::new(),
            trigger_types: DashMap::new(),
            workers: DashMap::new(),
            known_stream_names: DashSet::new(),
            known_topics: DashSet::new(),
            known_api_paths: DashSet::new(),
            known_scopes: DashSet::new(),
            guard: Mutex::new(None),
        })
    }

    pub async fn start(self: &Arc<Self>) {
        self.seed_cache().await;

        let client = Arc::clone(self);
        let guard = self.iii.on_functions_available(move |functions| {
            client.functions.clear();
            for func in &functions {
                client
                    .functions
                    .insert(func.function_id.clone(), func.clone());
            }

            let client = Arc::clone(&client);
            tokio::task::spawn(async move {
                client.reseed_secondary_caches().await;
            });
        });

        *self.guard.lock().unwrap() = Some(guard);
    }

    async fn seed_cache(&self) {
        if let Ok(functions) = self.iii.list_functions().await {
            for func in functions {
                self.functions.insert(func.function_id.clone(), func);
            }
        }
        self.reseed_secondary_caches().await;
    }

    async fn reseed_secondary_caches(&self) {
        if let Ok(trigger_types) = self.iii.list_trigger_types(false).await {
            self.trigger_types.clear();
            for tt in trigger_types {
                self.trigger_types.insert(tt.id.clone(), tt);
            }
        }

        if let Ok(workers) = self.iii.list_workers().await {
            self.workers.clear();
            for w in workers {
                self.workers.insert(w.id.clone(), w);
            }
        }

        // Extract known names from trigger configs
        if let Ok(triggers) = self.iii.list_triggers(false).await {
            self.extract_known_values(&triggers);
        }
    }

    /// Parse trigger configs to extract known stream names, topics, api paths, etc.
    fn extract_known_values(&self, triggers: &[TriggerInfo]) {
        self.known_stream_names.clear();
        self.known_topics.clear();
        self.known_api_paths.clear();
        self.known_scopes.clear();

        for trigger in triggers {
            match trigger.trigger_type.as_str() {
                "stream" | "stream:join" | "stream:leave" => {
                    if let Some(name) = trigger.config.get("stream_name").and_then(|v| v.as_str()) {
                        self.known_stream_names.insert(name.to_string());
                    }
                }
                "queue" | "subscribe" => {
                    if let Some(topic) = trigger.config.get("topic").and_then(|v| v.as_str()) {
                        self.known_topics.insert(topic.to_string());
                    }
                }
                "http" => {
                    if let Some(path) = trigger.config.get("api_path").and_then(|v| v.as_str()) {
                        self.known_api_paths.insert(path.to_string());
                    }
                }
                "state" => {
                    if let Some(scope) = trigger.config.get("scope").and_then(|v| v.as_str()) {
                        self.known_scopes.insert(scope.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    /// Get known values for a field name (stream_name, topic, api_path, scope, etc.)
    pub fn get_known_values(&self, field_name: &str) -> Vec<String> {
        match field_name {
            "stream_name" => self.known_stream_names.iter().map(|v| v.clone()).collect(),
            "topic" => self.known_topics.iter().map(|v| v.clone()).collect(),
            "api_path" => self.known_api_paths.iter().map(|v| v.clone()).collect(),
            "scope" => self.known_scopes.iter().map(|v| v.clone()).collect(),
            "queue" => self.known_topics.iter().map(|v| v.clone()).collect(), // queues use topics
            _ => Vec::new(),
        }
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

    pub fn get_trigger_type(&self, id: &str) -> Option<TriggerTypeInfo> {
        let entry = self.trigger_types.get(id)?;
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
        if let Ok(mut guard) = self.guard.lock() {
            *guard = None;
        }
        self.iii.shutdown_async().await;
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
        };
        functions.insert(func.function_id.clone(), func);

        assert_eq!(functions.len(), 1);
        let entry = functions.get("test::hello").unwrap();
        assert_eq!(entry.value().description.as_deref(), Some("A test function"));
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
}
