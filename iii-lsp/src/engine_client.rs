use dashmap::DashMap;
use iii_sdk::{
    register_worker, FunctionInfo, FunctionsAvailableGuard, IIIConnectionState, InitOptions,
    TriggerTypeInfo, WorkerInfo, WorkerMetadata, III,
};
use std::sync::{Arc, Mutex};

pub struct EngineClient {
    iii: III,
    pub functions: DashMap<String, FunctionInfo>,
    pub trigger_types: DashMap<String, TriggerTypeInfo>,
    pub workers: DashMap<String, WorkerInfo>,
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
