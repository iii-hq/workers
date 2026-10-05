//! `router::on_secret_changed` — the iii function bound to the `secrets`
//! worker's `secrets::changed` trigger type. The payload is advisory and
//! never carries a value: the handler re-reads the named reference through
//! `secrets::resolve`, and when its outcome changed (rotated, deleted,
//! access granted or revoked) the cache's change listener queues a
//! `Refresh::Credential` for every provider whose slice references it.
//!
//! Engine-backed coverage: tests/integration.rs (secret reference lifecycle).
use std::sync::Arc;

use futures::future::BoxFuture;
use iii_sdk::errors::Error;

use super::{parse_ref, valid_name, ChangeListener, Refetch, SecretCache};
use crate::config::state::{snapshot, ConfigCell};
use crate::registry::refresh::{Refresh, RefreshQueue};
use crate::registry::resolve::referenced_secrets;
use crate::types::router::{RouterAck, SecretChangedEvent};

/// The secret an event is about, from `name` or else from `ref`.
fn event_secret(event: &SecretChangedEvent) -> Option<String> {
    if let Some(name) = event.name.as_deref().filter(|name| valid_name(name)) {
        return Some(name.to_string());
    }
    match parse_ref(event.reference.as_deref()?)? {
        Ok(name) => Some(name.to_string()),
        Err(_) => None,
    }
}

pub fn make_on_secret_changed(
    config: ConfigCell,
    secrets: Arc<SecretCache>,
) -> impl Fn(SecretChangedEvent) -> BoxFuture<'static, Result<RouterAck, Error>> + Send + Sync + 'static
{
    move |event: SecretChangedEvent| {
        let (config, secrets) = (config.clone(), secrets.clone());
        Box::pin(async move {
            let Some(name) = event_secret(&event) else {
                return Ok(RouterAck { ok: false });
            };
            // Secrets no slice references are none of this router's business.
            if referenced_secrets(&snapshot(&config)).contains_key(&name) {
                secrets.refresh([name], Refetch::Invalidate).await;
            }
            Ok(RouterAck { ok: true })
        })
    }
}

/// The [`SecretCache`] change listener: queue a credential refresh for every
/// provider whose slice references a changed secret.
pub fn refresh_referencing_providers(config: ConfigCell, refresh: RefreshQueue) -> ChangeListener {
    Arc::new(move |names: Vec<String>| {
        let referenced = referenced_secrets(&snapshot(&config));
        refresh.schedule(
            names
                .iter()
                .filter_map(|name| referenced.get(name))
                .flatten()
                .map(|provider| (provider.clone(), Refresh::Credential)),
        );
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::state::new_config_cell;
    use crate::registry::refresh::testing::recording_queue;
    use crate::secrets::testing::FakeSecrets;
    use crate::secrets::SecretError;
    use serde_json::json;
    use std::time::Duration;

    /// Past the debounce (the paused clock auto-advances while idle).
    async fn flush() {
        tokio::time::sleep(Duration::from_secs(3)).await;
    }

    #[test]
    fn the_secret_comes_from_name_or_ref() {
        let event = |name: Option<&str>, reference: Option<&str>| SecretChangedEvent {
            name: name.map(String::from),
            reference: reference.map(String::from),
            action: Some("rotated".into()),
        };
        assert_eq!(event_secret(&event(Some("A"), None)).as_deref(), Some("A"));
        assert_eq!(
            event_secret(&event(None, Some("secret://B"))).as_deref(),
            Some("B")
        );
        assert_eq!(event_secret(&event(Some("bad name"), None)), None);
        assert_eq!(event_secret(&event(None, None)), None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_changed_event_evicts_the_value_and_refreshes_referencing_providers() {
        let config = new_config_cell(json!({ "providers": {
            "anthropic": { "api_key": "secret://SHARED" },
            "proxy": { "api_key": "secret://SHARED" },
            "openai": { "api_key": "sk-literal" },
        }}));
        let (queue, fired) = recording_queue(Duration::from_secs(2));
        let fake = FakeSecrets::default();
        fake.set("SHARED", Ok("v1"));
        let secrets = Arc::new(
            fake.cache()
                .with_listener(refresh_referencing_providers(config.clone(), queue)),
        );
        secrets.ensure(["SHARED".to_string()]).await;
        let handler = make_on_secret_changed(config, secrets.clone());

        // rotated
        fake.set("SHARED", Ok("v2"));
        let ack = handler(SecretChangedEvent {
            name: Some("SHARED".into()),
            reference: Some("secret://SHARED".into()),
            action: Some("rotated".into()),
        })
        .await
        .unwrap();
        assert!(ack.ok);
        assert_eq!(secrets.lookup("SHARED"), Some(Ok("v2".into())));
        flush().await;
        assert_eq!(
            *fired.lock().unwrap(),
            vec![
                "provider::anthropic::on_router_ready".to_string(),
                "provider::proxy::on_router_ready".to_string(),
            ]
        );

        // deleted: the cached value is gone, not served stale
        fired.lock().unwrap().clear();
        fake.set("SHARED", Err(SecretError::NotFound));
        handler(SecretChangedEvent {
            name: Some("SHARED".into()),
            reference: None,
            action: Some("deleted".into()),
        })
        .await
        .unwrap();
        assert_eq!(secrets.lookup("SHARED"), Some(Err(SecretError::NotFound)));
        flush().await;
        assert_eq!(fired.lock().unwrap().len(), 2);

        // an event that changes nothing nudges nobody
        fired.lock().unwrap().clear();
        handler(SecretChangedEvent {
            name: Some("SHARED".into()),
            reference: None,
            action: Some("access_changed".into()),
        })
        .await
        .unwrap();
        flush().await;
        assert!(fired.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn unreferenced_secrets_are_not_fetched() {
        let config = new_config_cell(json!({ "providers": { "openai": { "api_key": "sk" } } }));
        let fake = FakeSecrets::default();
        let handler = make_on_secret_changed(config, Arc::new(fake.cache()));
        let ack = handler(SecretChangedEvent {
            name: Some("SOMEONE_ELSES".into()),
            reference: None,
            action: Some("created".into()),
        })
        .await
        .unwrap();
        assert!(ack.ok);
        assert!(fake.calls().is_empty());
    }
}
