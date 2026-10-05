//! `router::on_config_changed` — the iii function bound to the engine's
//! `configuration` trigger (paste-a-key flow, spec § Triggers bound).
//! Fingerprint → diff → resolve the changed slices' `secret://` references →
//! debounce → `provider::<id>::refresh_models` (fire-and-forget iii call) +
//! whole-snapshot configuration refresh.
//!
//! Engine-backed coverage: tests/integration.rs (paste-a-key flow, with a
//! literal key and with a secret reference).
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use serde_json::Value;

use super::entry::{read_entry_value, EntryWriteLock};
use super::fingerprint::{changed_slices, fingerprint_slices};
use super::state::{apply_config, snapshot, ConfigCell};
use crate::registry::refresh::{Refresh, RefreshQueue};
use crate::registry::resolve::referenced_secrets;
use crate::secrets::{Refetch, SecretCache};
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
    secrets: Arc<SecretCache>,
    refresh: RefreshQueue,
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

    move |_event: ConfigChangedEvent| {
        let iii = iii.clone();
        let supports = supports_model_listing.clone();
        let config = config.clone();
        let entry_lock = entry_lock.clone();
        let last_fingerprints = last_fingerprints.clone();
        let secrets = secrets.clone();
        let refresh = refresh.clone();
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
            follow_up(changed, &config, &secrets, &refresh, &supports).await;
            Ok(RouterAck { ok: true })
        })
    }
}

/// After a new snapshot is installed: drop cached secrets no slice
/// references any more, re-read the references of the changed slices, and
/// only then queue discovery — so a pasted `secret://NAME` is resolvable by
/// the time the provider's `refresh_models` asks `router::provider::resolve`.
async fn follow_up(
    changed: Vec<String>,
    config: &ConfigCell,
    secrets: &SecretCache,
    refresh: &RefreshQueue,
    supports_model_listing: &ListingLookup,
) {
    let referenced = referenced_secrets(&snapshot(config));
    secrets.retain(&referenced.keys().cloned().collect());
    let touched: Vec<String> = referenced
        .into_iter()
        .filter(|(_, providers)| providers.iter().any(|id| changed.contains(id)))
        .map(|(name, _)| name)
        .collect();
    secrets.refresh(touched, Refetch::KeepLastGood).await;

    let mut due = BTreeSet::new();
    for id in changed {
        if supports_model_listing(&id).await {
            due.insert(id);
        }
    }
    refresh.schedule(due.into_iter().map(|id| (id, Refresh::Models)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::state::new_config_cell;
    use crate::registry::refresh::testing::recording_queue;
    use crate::secrets::testing::FakeSecrets;
    use serde_json::json;
    use std::time::Duration;

    fn lists_models() -> ListingLookup {
        Arc::new(|_id: &str| Box::pin(async { true }))
    }

    #[tokio::test(start_paused = true)]
    async fn changed_slices_resolve_their_references_before_discovery_is_queued() {
        let fake = FakeSecrets::default();
        fake.set("ANTHROPIC_API_KEY", Ok("sk-from-secrets"));
        fake.set("STALE", Ok("old"));
        let secrets = fake.cache();
        secrets.ensure(["STALE".to_string()]).await;
        let (queue, fired) = recording_queue(Duration::from_secs(2));
        let config = new_config_cell(json!({ "providers": {
            "anthropic": { "api_key": "secret://ANTHROPIC_API_KEY" },
            "openai": { "api_key": "secret://OPENAI_API_KEY" },
        }}));

        follow_up(
            vec!["anthropic".into()],
            &config,
            &secrets,
            &queue,
            &lists_models(),
        )
        .await;

        // Resolved already — before the debounced refresh_models fires.
        assert_eq!(
            secrets.lookup("ANTHROPIC_API_KEY"),
            Some(Ok("sk-from-secrets".into()))
        );
        assert!(fired.lock().unwrap().is_empty());
        // Unchanged slices are not re-read; unreferenced entries are dropped.
        assert!(!fake.calls().contains(&"OPENAI_API_KEY".to_string()));
        assert_eq!(secrets.lookup("STALE"), None);

        tokio::time::sleep(Duration::from_secs(3)).await;
        assert_eq!(
            *fired.lock().unwrap(),
            vec!["provider::anthropic::refresh_models".to_string()]
        );
    }
}
