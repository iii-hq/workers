//! Minimal provider of a worker-owned trigger type that an A2UI live binding
//! can target. Run it next to an engine (no iii-stream needed):
//!
//! ```bash
//! cargo run --example owned_trigger_provider -- ws://127.0.0.1:49134
//! ```
//!
//! Then bind a surface to it with `a2ui::binding::set`:
//!
//! ```json
//! {
//!   "surface_id": "clicks",
//!   "binding": {
//!     "id": "clicks-count",
//!     "trigger_type": "demo::counter-changed",
//!     "config": { "counter": "clicks" },
//!     "target_path": "/count",
//!     "query": {
//!       "function_id": "demo::counter::get",
//!       "payload": { "counter": "clicks" },
//!       "result_path": "/value"
//!     }
//!   }
//! }
//! ```
//!
//! `demo::counter::bump { "counter": "clicks" }` commits a new value and then
//! notifies matching bindings with `{ counter, revision }`; the A2UI page
//! re-reads through the read-only `demo::counter::get` query.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{
    register_worker, Error, IIIClient, InitOptions, RegisterFunction, RegisterTriggerType,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const TRIGGER_TYPE: &str = "demo::counter-changed";
/// Bound fan-out: bindings beyond this are rejected at registration.
const MAX_BINDINGS: usize = 64;

/// Binding configuration for `demo::counter-changed`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CounterConfig {
    /// Counter whose changes are delivered.
    counter: String,
}

/// Request naming one counter.
#[derive(Debug, Deserialize, JsonSchema)]
struct CounterRequest {
    /// Counter name.
    counter: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct Empty {}

#[derive(Default)]
struct Provider {
    /// counter -> (value, revision); the source of truth the query reads.
    counters: Mutex<HashMap<String, (u64, u64)>>,
    /// trigger id -> binding.
    bindings: Mutex<HashMap<String, TriggerConfig>>,
}

struct Handler(Arc<Provider>);

#[async_trait]
impl TriggerHandler for Handler {
    async fn register_trigger(&self, binding: TriggerConfig) -> Result<(), Error> {
        // Validate the filter: throwing rejects the binding.
        let config: CounterConfig = serde_json::from_value(binding.config.clone())
            .map_err(|error| Error::Handler(format!("invalid {TRIGGER_TYPE} config: {error}")))?;
        if config.counter.trim().is_empty() || config.counter.len() > 64 {
            return Err(Error::Handler("counter must be 1-64 bytes".into()));
        }
        let mut bindings = self.0.bindings.lock().unwrap();
        if bindings.len() >= MAX_BINDINGS && !bindings.contains_key(&binding.id) {
            return Err(Error::Handler(format!("too many {TRIGGER_TYPE} bindings")));
        }
        bindings.insert(binding.id.clone(), binding);
        Ok(())
    }

    async fn unregister_trigger(&self, binding: TriggerConfig) -> Result<(), Error> {
        self.0.bindings.lock().unwrap().remove(&binding.id);
        Ok(())
    }
}

impl Provider {
    fn read(&self, counter: &str) -> Value {
        let (value, revision) = self
            .counters
            .lock()
            .unwrap()
            .get(counter)
            .copied()
            .unwrap_or_default();
        json!({ "counter": counter, "value": value, "revision": revision })
    }

    /// Deliver after commit, sequentially with a timeout per binding: no task
    /// per event and no queue for slow consumers.
    async fn notify(&self, iii: &IIIClient, counter: &str, revision: u64) -> usize {
        let targets: Vec<TriggerConfig> = self
            .bindings
            .lock()
            .unwrap()
            .values()
            .filter(|binding| {
                binding.config.get("counter").and_then(Value::as_str) == Some(counter)
            })
            .cloned()
            .collect();
        let mut delivered = 0;
        for binding in targets {
            let request = TriggerRequest {
                function_id: binding.function_id.clone(),
                payload: json!({ "counter": counter, "revision": revision }),
                action: None,
                timeout_ms: Some(5_000),
            };
            // Preserve the consumer's namespace and metadata.
            let mut request = request.metadata(binding.metadata.clone().unwrap_or(Value::Null));
            if let Some(namespace) = binding.namespace.clone() {
                request = request.namespace(namespace);
            }
            match iii.trigger(request).await {
                Ok(_) => delivered += 1,
                Err(error) => {
                    eprintln!("{TRIGGER_TYPE} delivery to {} failed: {error}", binding.id)
                }
            }
        }
        delivered
    }
}

#[tokio::main]
async fn main() {
    let url = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("III_URL").ok())
        .unwrap_or_else(|| "ws://127.0.0.1:49134".into());
    let iii = Arc::new(register_worker(
        &url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                name: "a2ui-demo-provider".into(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    let provider = Arc::new(Provider::default());

    iii.register_trigger_type(
        RegisterTriggerType::new(
            TRIGGER_TYPE,
            "Demo counter changed: { counter, revision } after each committed bump.",
            Handler(provider.clone()),
        )
        .trigger_request_format::<CounterConfig>(),
    );

    let get = provider.clone();
    iii.register_function(
        "demo::counter::get",
        RegisterFunction::new_async(move |request: CounterRequest| {
            let provider = get.clone();
            async move { Ok::<Value, Error>(provider.read(&request.counter)) }
        })
        .description("Read one demo counter: { counter, value, revision }.")
        .metadata(json!({ "read_only": true })),
    );

    let bump = provider.clone();
    let bump_iii = iii.clone();
    iii.register_function(
        "demo::counter::bump",
        RegisterFunction::new_async(move |request: CounterRequest| {
            let provider = bump.clone();
            let iii = bump_iii.clone();
            async move {
                let revision = {
                    let mut counters = provider.counters.lock().unwrap();
                    let entry = counters.entry(request.counter.clone()).or_default();
                    entry.0 += 1;
                    entry.1 += 1;
                    entry.1
                };
                let delivered = provider.notify(&iii, &request.counter, revision).await;
                let mut current = provider.read(&request.counter);
                current["delivered"] = json!(delivered);
                Ok::<Value, Error>(current)
            }
        })
        .description(
            "Increment a demo counter, then notify matching demo::counter-changed bindings.",
        ),
    );

    let count = provider.clone();
    iii.register_function(
        "demo::bindings::count",
        RegisterFunction::new_async(move |_: Empty| {
            let provider = count.clone();
            async move {
                Ok::<Value, Error>(json!({ "count": provider.bindings.lock().unwrap().len() }))
            }
        })
        .description("Number of active demo::counter-changed bindings.")
        .metadata(json!({ "read_only": true })),
    );

    println!("a2ui-demo-provider ready on {url}");
    let _ = tokio::signal::ctrl_c().await;
    iii.shutdown_async().await;
}
