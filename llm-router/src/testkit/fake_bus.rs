//! In-memory `Bus` implementation: a function table plus emulations of the iii
//! engine builtins the router depends on (`state::get/set`,
//! `configuration::register/get/set`), engine-faithful trigger-type replay,
//! and a `subscribe`-topic emitter for tests.
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::bus::{Bus, BusError, Handler, TriggerBinding, TriggerTypeCallbacks};

#[derive(Debug, Clone)]
pub struct ConfigEntry {
    pub name: String,
    pub description: String,
    pub schema: Value,
    pub value: Value,
}

#[derive(Default)]
pub struct FakeBus {
    functions: Mutex<HashMap<String, Handler>>,
    unavailable: Mutex<HashSet<String>>,
    state: Mutex<HashMap<(String, String), Value>>,
    pub config_entries: Mutex<HashMap<String, ConfigEntry>>,
    trigger_types: Mutex<HashMap<String, Arc<TriggerTypeCallbacks>>>,
    bindings: Mutex<Vec<(String, TriggerBinding)>>, // (trigger_type, binding)
    pub calls: Mutex<Vec<(String, Value)>>,
    seq: AtomicU64,
}

impl FakeBus {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn set_unavailable(&self, function_id: &str) {
        self.unavailable
            .lock()
            .unwrap()
            .insert(function_id.to_string());
    }
    pub fn set_available(&self, function_id: &str) {
        self.unavailable.lock().unwrap().remove(function_id);
    }

    fn handler_for(&self, function_id: &str) -> Option<Handler> {
        if self.unavailable.lock().unwrap().contains(function_id) {
            return None;
        }
        self.functions.lock().unwrap().get(function_id).cloned()
    }

    /// Fire a `subscribe` topic (engine::workers-available etc.) to bound functions.
    pub async fn emit_topic(&self, topic: &str, payload: Value) {
        let targets: Vec<String> = self
            .bindings
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, b)| t == "subscribe" && b.config["topic"] == topic)
            .map(|(_, b)| b.function_id.clone())
            .collect();
        for fid in targets {
            if let Some(h) = self.handler_for(&fid) {
                let _ = h(payload.clone()).await;
            }
        }
    }

    async fn fire_config_updated(&self, id: &str, old_value: Value) {
        let (entry, targets) = {
            let entries = self.config_entries.lock().unwrap();
            let Some(entry) = entries.get(id).cloned() else {
                return;
            };
            let targets: Vec<String> = self
                .bindings
                .lock()
                .unwrap()
                .iter()
                .filter(|(t, b)| {
                    t == "configuration"
                        && b.config
                            .get("configuration_id")
                            .map(|v| v == id)
                            .unwrap_or(true)
                        && b.config
                            .get("event_types")
                            .and_then(|v| v.as_array())
                            .map(|a| a.iter().any(|e| e == "configuration:updated"))
                            .unwrap_or(true)
                })
                .map(|(_, b)| b.function_id.clone())
                .collect();
            (entry, targets)
        };
        let event = json!({
            "message_type": "configuration",
            "event_type": "configuration:updated",
            "id": id,
            "name": entry.name,
            "description": entry.description,
            "schema": entry.schema,
            "old_value": old_value,
            "new_value": entry.value,
        });
        for fid in targets {
            if let Some(h) = self.handler_for(&fid) {
                let _ = h(event.clone()).await;
            }
        }
    }

    /// Engine builtins. Ok(Some(v)) handled; Ok(None) = not a builtin.
    async fn builtin(&self, function_id: &str, p: &Value) -> Result<Option<Value>, BusError> {
        match function_id {
            "state::get" => {
                let key = (
                    p["scope"].as_str().unwrap_or("").into(),
                    p["key"].as_str().unwrap_or("").into(),
                );
                Ok(Some(
                    self.state
                        .lock()
                        .unwrap()
                        .get(&key)
                        .cloned()
                        .unwrap_or(Value::Null),
                ))
            }
            "state::set" => {
                let key = (
                    p["scope"].as_str().unwrap_or("").into(),
                    p["key"].as_str().unwrap_or("").into(),
                );
                self.state.lock().unwrap().insert(key, p["value"].clone());
                Ok(Some(Value::Null))
            }
            "configuration::register" => {
                let id = p["id"].as_str().unwrap_or("").to_string();
                let prior = self.config_entries.lock().unwrap().get(&id).cloned();
                let value = match p.get("initial_value") {
                    Some(v) => v.clone(),
                    None => prior.map(|e| e.value).unwrap_or(Value::Null),
                };
                self.config_entries.lock().unwrap().insert(
                    id.clone(),
                    ConfigEntry {
                        name: p["name"].as_str().unwrap_or("").into(),
                        description: p["description"].as_str().unwrap_or("").into(),
                        schema: p["schema"].clone(),
                        value,
                    },
                );
                Ok(Some(json!({ "id": id })))
            }
            "configuration::get" => {
                let id = p["id"].as_str().unwrap_or("");
                match self.config_entries.lock().unwrap().get(id) {
                    Some(e) => Ok(Some(json!({ "id": id, "value": e.value }))),
                    None => Err(BusError::Coded {
                        code: "NOT_FOUND".into(),
                        message: format!("configuration {id}"),
                    }),
                }
            }
            "configuration::set" => {
                let id = p["id"].as_str().unwrap_or("").to_string();
                let old = {
                    let mut entries = self.config_entries.lock().unwrap();
                    let Some(entry) = entries.get_mut(&id) else {
                        return Err(BusError::Coded {
                            code: "NOT_FOUND".into(),
                            message: format!("configuration {id}"),
                        });
                    };
                    std::mem::replace(&mut entry.value, p["value"].clone())
                };
                self.fire_config_updated(&id, old).await;
                Ok(Some(json!({ "ok": true })))
            }
            _ => Ok(None),
        }
    }
}

#[async_trait::async_trait]
impl Bus for FakeBus {
    async fn trigger(
        &self,
        function_id: &str,
        payload: Value,
        _timeout_ms: Option<u64>,
    ) -> Result<Value, BusError> {
        self.calls
            .lock()
            .unwrap()
            .push((function_id.to_string(), payload.clone()));
        if let Some(v) = self.builtin(function_id, &payload).await? {
            return Ok(v);
        }
        match self.handler_for(function_id) {
            Some(h) => h(payload).await,
            None => Err(BusError::FunctionNotFound(function_id.to_string())),
        }
    }

    fn register_function(&self, id: &str, handler: Handler) {
        self.functions
            .lock()
            .unwrap()
            .insert(id.to_string(), handler);
    }

    fn register_trigger(&self, trigger_type: &str, function_id: &str, config: Value) {
        let binding = TriggerBinding {
            id: format!("trig_{}", self.seq.fetch_add(1, Ordering::SeqCst)),
            function_id: function_id.to_string(),
            config,
        };
        if let Some(cb) = self.trigger_types.lock().unwrap().get(trigger_type) {
            (cb.on_register)(&binding);
        }
        self.bindings
            .lock()
            .unwrap()
            .push((trigger_type.to_string(), binding));
    }

    fn register_trigger_type(&self, id: &str, _description: &str, callbacks: TriggerTypeCallbacks) {
        let callbacks = Arc::new(callbacks);
        // engine replays already-bound triggers of this type to a (re)registering owner
        for (t, b) in self.bindings.lock().unwrap().iter() {
            if t == id {
                (callbacks.on_register)(b);
            }
        }
        self.trigger_types
            .lock()
            .unwrap()
            .insert(id.to_string(), callbacks);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{handler, Bus, BusError, TriggerTypeCallbacks};
    use serde_json::json;
    use std::sync::Mutex as StdMutex;

    #[tokio::test]
    async fn routes_trigger_to_registered_functions() {
        let bus = FakeBus::new();
        bus.register_function(
            "echo::run",
            handler(|v| async move { Ok(json!({ "got": v })) }),
        );
        let res = bus
            .trigger("echo::run", json!({ "a": 1 }), None)
            .await
            .unwrap();
        assert_eq!(res, json!({ "got": { "a": 1 } }));
    }

    #[tokio::test]
    async fn unknown_or_unavailable_functions_are_function_not_found() {
        let bus = FakeBus::new();
        assert!(matches!(
            bus.trigger("nope::x", json!({}), None).await,
            Err(BusError::FunctionNotFound(_))
        ));
        bus.register_function("p::stream", handler(|_| async move { Ok(json!({})) }));
        bus.set_unavailable("p::stream");
        assert!(matches!(
            bus.trigger("p::stream", json!({}), None).await,
            Err(BusError::FunctionNotFound(_))
        ));
    }

    #[tokio::test]
    async fn emulates_state_get_set_per_scope_and_key() {
        let bus = FakeBus::new();
        let got = bus
            .trigger("state::get", json!({ "scope": "s", "key": "k" }), None)
            .await
            .unwrap();
        assert_eq!(got, serde_json::Value::Null);
        bus.trigger(
            "state::set",
            json!({ "scope": "s", "key": "k", "value": { "x": 1 } }),
            None,
        )
        .await
        .unwrap();
        let got = bus
            .trigger("state::get", json!({ "scope": "s", "key": "k" }), None)
            .await
            .unwrap();
        assert_eq!(got, json!({ "x": 1 }));
    }

    #[tokio::test]
    async fn configuration_register_preserves_stored_value_on_reregister() {
        let bus = FakeBus::new();
        bus.trigger("configuration::register", json!({ "id": "llm-router", "name": "n", "description": "d", "schema": {}, "initial_value": { "a": 1 } }), None).await.unwrap();
        bus.trigger(
            "configuration::set",
            json!({ "id": "llm-router", "value": { "a": 2 } }),
            None,
        )
        .await
        .unwrap();
        bus.trigger(
            "configuration::register",
            json!({ "id": "llm-router", "name": "n", "description": "d", "schema": { "v": 2 } }),
            None,
        )
        .await
        .unwrap();
        let got = bus
            .trigger("configuration::get", json!({ "id": "llm-router" }), None)
            .await
            .unwrap();
        assert_eq!(got, json!({ "id": "llm-router", "value": { "a": 2 } }));
    }

    #[tokio::test]
    async fn configuration_set_fires_bound_triggers_with_id_filter() {
        let bus = FakeBus::new();
        let seen = std::sync::Arc::new(StdMutex::new(Vec::<serde_json::Value>::new()));
        let seen2 = seen.clone();
        bus.register_function(
            "r::on_config",
            handler(move |v| {
                let seen = seen2.clone();
                async move {
                    seen.lock().unwrap().push(v);
                    Ok(serde_json::Value::Null)
                }
            }),
        );
        bus.trigger(
            "configuration::register",
            json!({ "id": "llm-router", "name": "n", "description": "d", "schema": {} }),
            None,
        )
        .await
        .unwrap();
        bus.trigger(
            "configuration::register",
            json!({ "id": "other", "name": "n", "description": "d", "schema": {} }),
            None,
        )
        .await
        .unwrap();
        bus.register_trigger(
            "configuration",
            "r::on_config",
            json!({ "configuration_id": "llm-router", "event_types": ["configuration:updated"] }),
        );
        bus.trigger(
            "configuration::set",
            json!({ "id": "other", "value": 1 }),
            None,
        )
        .await
        .unwrap();
        bus.trigger(
            "configuration::set",
            json!({ "id": "llm-router", "value": { "b": 2 } }),
            None,
        )
        .await
        .unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0]["id"], "llm-router");
        assert_eq!(seen[0]["event_type"], "configuration:updated");
        assert_eq!(seen[0]["new_value"], json!({ "b": 2 }));
    }

    #[tokio::test]
    async fn custom_trigger_type_registrations_reach_the_owner_even_if_bound_first() {
        let bus = FakeBus::new();
        // subscriber binds BEFORE the type owner exists (boot-order recovery)
        bus.register_trigger("router::ready", "prov::on_ready", json!({}));
        let registered = std::sync::Arc::new(StdMutex::new(Vec::<String>::new()));
        let r2 = registered.clone();
        bus.register_trigger_type(
            "router::ready",
            "fires at boot",
            TriggerTypeCallbacks {
                on_register: std::sync::Arc::new(move |b| {
                    r2.lock().unwrap().push(b.function_id.clone())
                }),
                on_unregister: std::sync::Arc::new(|_| {}),
            },
        );
        assert_eq!(registered.lock().unwrap().as_slice(), ["prov::on_ready"]);
    }

    #[tokio::test]
    async fn emit_topic_reaches_subscribe_bindings() {
        let bus = FakeBus::new();
        let seen = std::sync::Arc::new(StdMutex::new(Vec::<serde_json::Value>::new()));
        let s2 = seen.clone();
        bus.register_function(
            "r::on_workers",
            handler(move |v| {
                let seen = s2.clone();
                async move {
                    seen.lock().unwrap().push(v);
                    Ok(serde_json::Value::Null)
                }
            }),
        );
        bus.register_trigger(
            "subscribe",
            "r::on_workers",
            json!({ "topic": "engine::workers-available" }),
        );
        bus.emit_topic(
            "engine::workers-available",
            json!({ "event": "worker_connected", "worker_id": "w1" }),
        )
        .await;
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}
