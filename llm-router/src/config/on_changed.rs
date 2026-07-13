//! `router::on_config_changed` — the iii function bound to the engine's
//! `configuration` trigger (paste-a-key flow, spec § Triggers bound).
//! Fingerprint → diff → debounce → `provider::<id>::refresh_models`
//! (fire-and-forget iii call) + whole-snapshot configuration refresh.
//!
//! Engine-backed coverage: tests/integration.rs (paste-a-key flow).
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

use super::entry::{read_entry_value, EntryWriteLock};
use super::fingerprint::{changed_slices, fingerprint_slices};
use super::state::{apply_config, snapshot, ConfigCell};
use crate::settings::provider_slices;
use crate::types::router::{ConfigChangedEvent, RouterAck};

/// Async lookup: the registry's records sit behind a tokio mutex, so a sync
/// closure would have to block — async keeps the handler deadlock-free.
pub type ListingLookup = Arc<dyn Fn(&str) -> BoxFuture<'static, bool> + Send + Sync>;

pub fn make_on_config_changed(
    iii: IIIClient,
    supports_model_listing: ListingLookup,
    config: ConfigCell,
    entry_lock: EntryWriteLock,
    debounce_ms: u64,
) -> impl Fn(ConfigChangedEvent) -> BoxFuture<'static, Result<RouterAck, Error>>
       + Send
       + Sync
       + Clone
       + 'static {
    let initial_fingerprints = {
        let current = snapshot(&config);
        let slices = provider_slices(current.value());
        fingerprint_slices(&serde_json::to_value(slices).unwrap_or(Value::Null))
    };
    let last_fingerprints = Arc::new(Mutex::new(initial_fingerprints));
    let pending: Arc<Mutex<BTreeSet<String>>> = Arc::default();
    let flush_generation: Arc<AtomicU64> = Arc::default();

    move |_event: ConfigChangedEvent| {
        let iii = iii.clone();
        let supports = supports_model_listing.clone();
        let config = config.clone();
        let entry_lock = entry_lock.clone();
        let last_fingerprints = last_fingerprints.clone();
        let pending = pending.clone();
        let flush_generation = flush_generation.clone();
        Box::pin(async move {
            // The trigger payload is advisory and this function is also
            // discoverable on the bus. Re-fetching prevents a direct caller
            // from injecting an arbitrary in-memory configuration. The read
            // retries with a short backoff: the trigger is not redelivered,
            // so dropping it would strand the router on the previous
            // snapshot until the NEXT configuration change.
            const READ_ATTEMPTS: u32 = 3;
            let mut changed = None;
            for attempt in 1..=READ_ATTEMPTS {
                {
                    let _guard = entry_lock.lock().await;
                    match read_entry_value(&iii).await {
                        Ok(new_value) => {
                            apply_config(&config, new_value.clone());
                            let slices = provider_slices(&new_value);
                            let next = fingerprint_slices(
                                &serde_json::to_value(slices).unwrap_or(Value::Null),
                            );
                            let mut last = last_fingerprints.lock().unwrap();
                            changed = Some(changed_slices(&last, &next));
                            *last = next;
                        }
                        Err(error) => {
                            tracing::error!(
                                %error,
                                attempt,
                                "config-change: failed to fetch authoritative configuration"
                            );
                        }
                    }
                }
                if changed.is_some() {
                    break;
                }
                if attempt < READ_ATTEMPTS {
                    tokio::time::sleep(std::time::Duration::from_millis(200 * u64::from(attempt)))
                        .await;
                }
            }
            let Some(changed) = changed else {
                tracing::error!(
                    "config-change: keeping previous snapshot; authoritative read failed after retries"
                );
                return Ok(RouterAck { ok: false });
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

            // Debounce: each event arms a fresh flush; a superseded task
            // notices via the generation check and exits BEFORE draining.
            // Never abort a task — one past its drain has sole custody of
            // the taken ids (their fingerprints already advanced), so a
            // mid-flush kill would silently drop refreshes.
            let generation = flush_generation.fetch_add(1, Ordering::SeqCst) + 1;
            let pending = pending.clone();
            let flush_generation = flush_generation.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(debounce_ms)).await;
                if flush_generation.load(Ordering::SeqCst) != generation {
                    return; // a newer event re-armed the flush; it owns the drain
                }
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
            });
            Ok(RouterAck { ok: true })
        })
    }
}
