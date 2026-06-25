//! `router::on_config_changed` — the iii function bound to the engine's
//! `configuration` trigger (paste-a-key flow, spec § Triggers bound).
//! Fingerprint → diff → debounce → `provider::<id>::refresh_models`
//! (fire-and-forget iii call) + settings refresh.
//! Fingerprints are in-memory: a router restart re-fingerprints from zero,
//! which at worst causes one redundant discovery pass.
//!
//! Engine-backed coverage: tests/integration.rs (paste-a-key flow).
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, RwLock};

use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

use super::fingerprint::{changed_slices, fingerprint_slices};
use crate::settings::{parse_settings, provider_slices, RouterSettings};
use crate::types::router::{ConfigChangedEvent, RouterAck};

/// Async lookup: the registry's records sit behind a tokio mutex, so a sync
/// closure would have to block — async keeps the handler deadlock-free.
pub type ListingLookup = Arc<dyn Fn(&str) -> BoxFuture<'static, bool> + Send + Sync>;

pub fn make_on_config_changed(
    iii: IIIClient,
    supports_model_listing: ListingLookup,
    settings: Arc<RwLock<RouterSettings>>,
    debounce_ms: u64,
) -> impl Fn(ConfigChangedEvent) -> BoxFuture<'static, Result<RouterAck, Error>> + Send + Sync + 'static
{
    let last_fingerprints: Arc<Mutex<BTreeMap<String, String>>> = Arc::default();
    let pending: Arc<Mutex<BTreeSet<String>>> = Arc::default();
    let flush_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>> = Arc::default();

    move |event: ConfigChangedEvent| {
        let iii = iii.clone();
        let supports = supports_model_listing.clone();
        let settings = settings.clone();
        let last_fingerprints = last_fingerprints.clone();
        let pending = pending.clone();
        let flush_task = flush_task.clone();
        Box::pin(async move {
            if event.id.as_deref() != Some("llm-router") {
                return Ok(RouterAck { ok: true });
            }
            let new_value = event.new_value;

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
                return Ok(RouterAck { ok: true });
            }

            // debounce: replace any armed flush with a fresh one
            let mut task = flush_task.lock().unwrap();
            if let Some(t) = task.take() {
                t.abort();
            }
            let pending = pending.clone();
            *task = Some(tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(debounce_ms)).await;
                let ids: Vec<String> = std::mem::take(&mut *pending.lock().unwrap())
                    .into_iter()
                    .collect();
                for id in ids {
                    // fire-and-forget; discovery reports back via models::reconcile
                    let _ = iii
                        .trigger(TriggerRequest {
                            function_id: format!("provider::{id}::refresh_models"),
                            payload: json!({}),
                            action: None,
                            timeout_ms: None,
                        })
                        .await;
                }
            }));
            Ok(RouterAck { ok: true })
        })
    }
}
