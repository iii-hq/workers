//! Owner side of the router's custom iii trigger types (README § Trigger
//! delivery): keep the subscriber set, dispatch at-least-once, isolate
//! failures. The engine replays existing registrations on (re)registration of
//! the type, which rebuilds the set after a restart. Dispatch invokes each
//! subscriber's iii function via `Bus::trigger`.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::bus::{Bus, TriggerTypeCallbacks};

const TYPES: [(&str, &str); 3] = [
    (
        "router::models::changed",
        "Fires when a provider reconciles its catalog slice.",
    ),
    (
        "router::provider::changed",
        "Fires when the provider registry changes (declare / availability flip).",
    ),
    (
        "router::ready",
        "Fires once when the router finishes booting; providers re-declare on it.",
    ),
];

/// trigger type -> [(trigger_id, function_id)]
type SubscriberMap = HashMap<String, Vec<(String, String)>>;

#[derive(Clone)]
pub struct TriggerEmitter {
    bus: Arc<dyn Bus>,
    subscribers: Arc<Mutex<SubscriberMap>>,
}

impl TriggerEmitter {
    /// `bus_dyn` is the emit path; `registrar` is the same bus for type
    /// registration (split so callers can pass Arc<FakeBus> without coercion).
    pub fn install<B: Bus + ?Sized>(bus_dyn: Arc<dyn Bus>, registrar: &B) -> Self {
        let emitter = TriggerEmitter {
            bus: bus_dyn,
            subscribers: Arc::new(Mutex::new(HashMap::new())),
        };
        for (type_id, description) in TYPES {
            let subs = emitter.subscribers.clone();
            let subs_un = emitter.subscribers.clone();
            registrar.register_trigger_type(
                type_id,
                description,
                TriggerTypeCallbacks {
                    on_register: Arc::new(move |b| {
                        let mut map = subs.lock().unwrap();
                        let list = map.entry(type_id.to_string()).or_default();
                        if !list.iter().any(|(id, _)| id == &b.id) {
                            list.push((b.id.clone(), b.function_id.clone()));
                        }
                    }),
                    on_unregister: Arc::new(move |b| {
                        let mut map = subs_un.lock().unwrap();
                        if let Some(list) = map.get_mut(type_id) {
                            list.retain(|(id, _)| id != &b.id);
                        }
                    }),
                },
            );
        }
        emitter
    }

    pub fn subscriber_count(&self, type_id: &str) -> usize {
        self.subscribers
            .lock()
            .unwrap()
            .get(type_id)
            .map_or(0, Vec::len)
    }

    /// Fire-and-forget dispatch to every subscriber; failures are isolated.
    pub async fn emit(&self, type_id: &str, payload: Value) {
        let targets: Vec<String> = self
            .subscribers
            .lock()
            .unwrap()
            .get(type_id)
            .map(|l| l.iter().map(|(_, fid)| fid.clone()).collect())
            .unwrap_or_default();
        for fid in targets {
            let _ = self.bus.trigger(&fid, payload.clone(), None).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{handler, Bus};
    use crate::testkit::fake_bus::FakeBus;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn dispatches_to_subscribers_of_the_emitted_type_only_and_isolates_failures() {
        let bus = FakeBus::new();
        // one subscriber binds BEFORE install (boot-order replay), one after
        bus.register_trigger("router::models::changed", "early::sub", json!({}));
        let emitter = TriggerEmitter::install(bus.clone() as Arc<dyn Bus>, &*bus);
        let seen = Arc::new(Mutex::new(0u32));
        let s2 = seen.clone();
        bus.register_function(
            "early::sub",
            handler(move |_| {
                let s = s2.clone();
                async move {
                    *s.lock().unwrap() += 1;
                    Ok(serde_json::Value::Null)
                }
            }),
        );
        bus.register_function(
            "bad::sub",
            handler(|_| async move { Err(crate::bus::BusError::Transport("boom".into())) }),
        );
        bus.register_trigger("router::models::changed", "bad::sub", json!({}));
        bus.register_trigger("router::ready", "early::sub", json!({}));

        emitter
            .emit(
                "router::models::changed",
                json!({ "provider": "a", "count": 1 }),
            )
            .await;
        assert_eq!(*seen.lock().unwrap(), 1); // models::changed only, bad::sub isolated
        emitter.emit("router::ready", json!({})).await;
        assert_eq!(*seen.lock().unwrap(), 2);
    }
}
