//! `router::on_config_changed` — the iii function bound to the engine's
//! `configuration` trigger (paste-a-key flow, spec § Triggers bound).
//! Fingerprint → diff → debounce → `provider::<id>::refresh_models`
//! (fire-and-forget iii call) + settings refresh.
//! Fingerprints are in-memory: a router restart re-fingerprints from zero,
//! which at worst causes one redundant discovery pass.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, RwLock};

use serde_json::{json, Value};

use super::fingerprint::{changed_slices, fingerprint_slices};
use crate::bus::{handler, Bus, Handler};
use crate::settings::{parse_settings, provider_slices, RouterSettings};

/// Async lookup: the registry's records sit behind a tokio mutex, so a sync
/// closure would have to block — async keeps the handler deadlock-free.
pub type ListingLookup =
    Arc<dyn Fn(&str) -> futures::future::BoxFuture<'static, bool> + Send + Sync>;

pub fn make_on_config_changed(
    bus: Arc<dyn Bus>,
    supports_model_listing: ListingLookup,
    settings: Arc<RwLock<RouterSettings>>,
    debounce_ms: u64,
) -> Handler {
    let last_fingerprints: Arc<Mutex<BTreeMap<String, String>>> = Arc::default();
    let pending: Arc<Mutex<BTreeSet<String>>> = Arc::default();
    let flush_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>> = Arc::default();

    handler(move |raw: Value| {
        let bus = bus.clone();
        let supports = supports_model_listing.clone();
        let settings = settings.clone();
        let last_fingerprints = last_fingerprints.clone();
        let pending = pending.clone();
        let flush_task = flush_task.clone();
        async move {
            if raw.get("id").and_then(Value::as_str) != Some("llm-router") {
                return Ok(Value::Null);
            }
            let new_value = raw.get("new_value").cloned().unwrap_or(Value::Null);

            *settings.write().unwrap() = parse_settings(&new_value);

            let slices = provider_slices(&new_value);
            let next = fingerprint_slices(&serde_json::to_value(&slices).unwrap_or(Value::Null));
            let changed = {
                let mut last = last_fingerprints.lock().unwrap();
                let changed = changed_slices(&last, &next);
                *last = next;
                changed
            };
            let mut any_pending = false;
            for id in changed {
                if supports(&id).await {
                    pending.lock().unwrap().insert(id);
                    any_pending = true;
                }
            }
            if !any_pending && pending.lock().unwrap().is_empty() {
                return Ok(Value::Null);
            }

            // debounce: replace any armed flush with a fresh one
            let mut task = flush_task.lock().unwrap();
            if let Some(t) = task.take() {
                t.abort();
            }
            let (bus, pending) = (bus.clone(), pending.clone());
            *task = Some(tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(debounce_ms)).await;
                let ids: Vec<String> = std::mem::take(&mut *pending.lock().unwrap())
                    .into_iter()
                    .collect();
                for id in ids {
                    // fire-and-forget; discovery reports back via models::reconcile
                    let _ = bus
                        .trigger(&format!("provider::{id}::refresh_models"), json!({}), None)
                        .await;
                }
            }));
            Ok(Value::Null)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::RouterSettings;
    use crate::testkit::fake_bus::FakeBus;
    use serde_json::json;
    use std::sync::{Arc, RwLock};

    fn event(value: serde_json::Value) -> serde_json::Value {
        json!({ "message_type": "configuration", "event_type": "configuration:updated", "id": "llm-router", "new_value": value })
    }

    fn refresh_calls(bus: &FakeBus) -> usize {
        bus.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(f, _)| f.contains("refresh_models"))
            .count()
    }

    #[tokio::test(start_paused = true)]
    async fn kicks_refresh_for_changed_listing_capable_slices_debounced_and_collapsed() {
        let bus = FakeBus::new();
        bus.register_function(
            "provider::anthropic::refresh_models",
            crate::bus::handler(|_| async move { Ok(json!({})) }),
        );
        let settings = Arc::new(RwLock::new(RouterSettings::default()));
        let lookup: ListingLookup = Arc::new(|id: &str| {
            let yes = id == "anthropic";
            Box::pin(async move { yes })
        });
        let on_changed = make_on_config_changed(bus.clone(), lookup, settings.clone(), 2000);

        on_changed(event(
            json!({ "providers": { "anthropic": { "api_key": "a" } } }),
        ))
        .await
        .unwrap();
        // let the spawned flush task register its sleep timer before advancing
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(500)).await;
        assert_eq!(refresh_calls(&bus), 0); // still debouncing
                                            // a second rapid edit collapses into the same flush
        on_changed(event(
            json!({ "providers": { "anthropic": { "api_key": "b" } } }),
        ))
        .await
        .unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(2100)).await;
        tokio::task::yield_now().await;
        assert_eq!(refresh_calls(&bus), 1);

        // unchanged value → no slice changed → nothing fires; non-listing providers never fire
        on_changed(event(
            json!({ "providers": { "anthropic": { "api_key": "b" }, "lmstudio": { "max_tokens": 1 } } }),
        ))
        .await
        .unwrap();
        tokio::time::advance(std::time::Duration::from_millis(2100)).await;
        tokio::task::yield_now().await;
        assert_eq!(refresh_calls(&bus), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn refreshes_settings_immediately_and_missing_provider_functions_never_panic() {
        let bus = FakeBus::new();
        let settings = Arc::new(RwLock::new(RouterSettings::default()));
        let always: ListingLookup = Arc::new(|_: &str| Box::pin(async { true }));
        let on_changed = make_on_config_changed(bus, always, settings.clone(), 2000);
        on_changed(event(
            json!({ "settings": { "idle_timeout_ms": 5000 }, "providers": { "ghost": { "api_key": "x" } } }),
        ))
        .await
        .unwrap();
        assert_eq!(settings.read().unwrap().idle_timeout_ms, 5000);
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(2100)).await;
        tokio::task::yield_now().await; // ghost provider missing → error swallowed
    }
}
