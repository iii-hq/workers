//! `router::on_secret_changed` — the target of the router's observer of the
//! `secrets` worker's `secrets::on-changed`, which captures the event (its
//! output) in the Observed envelope. The event is advisory and
//! never carries a value: the handler re-reads the named reference through
//! `secrets::resolve`, and when its outcome changed (rotated, deleted,
//! access granted or revoked) the cache's change listener queues a
//! `Refresh::Credential` for every provider whose slice references it.
//!
//! Engine-backed coverage: tests/integration.rs (secret reference lifecycle).
use std::sync::Arc;

use futures::future::BoxFuture;
use iii_sdk::errors::Error;

use super::{ChangeListener, Refetch, SecretCache, SecretRef, parse_ref, valid_name};
use crate::config::state::{ConfigCell, snapshot};
use crate::registry::refresh::{Refresh, RefreshQueue};
use crate::registry::resolve::referenced_secrets;
use crate::types::router::{RouterAck, SecretChangedEvent, SecretChangedObserved};

/// The reference an event is about, canonical: from `ref` (which names the
/// store), or else from `name` as a vault secret.
fn event_secret(event: &SecretChangedEvent) -> Option<String> {
    if let Some(reference) = event.reference.as_deref()
        && let Some(Ok(reference)) = parse_ref(reference)
    {
        return Some(reference.key());
    }
    event
        .name
        .as_deref()
        .filter(|name| valid_name(name))
        .map(|name| SecretRef::vault(name).key())
}

pub fn make_on_secret_changed(
    config: ConfigCell,
    secrets: Arc<SecretCache>,
) -> impl Fn(SecretChangedObserved) -> BoxFuture<'static, Result<RouterAck, Error>> + Send + Sync + 'static
{
    move |observed: SecretChangedObserved| {
        let (config, secrets) = (config.clone(), secrets.clone());
        Box::pin(async move {
            let Some(reference) = observed.output.as_ref().and_then(event_secret) else {
                return Ok(RouterAck { ok: false });
            };
            // Secrets no slice references are none of this router's business.
            if referenced_secrets(&snapshot(&config)).contains_key(&reference) {
                secrets.refresh([reference], Refetch::Invalidate).await;
            }
            Ok(RouterAck { ok: true })
        })
    }
}

/// The [`SecretCache`] change listener: queue a credential refresh for every
/// provider whose slice references a changed secret.
pub fn refresh_referencing_providers(config: ConfigCell, refresh: RefreshQueue) -> ChangeListener {
    Arc::new(move |references: Vec<String>| {
        let referenced = referenced_secrets(&snapshot(&config));
        refresh.schedule(
            references
                .iter()
                .filter_map(|reference| referenced.get(reference))
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
    use crate::secrets::SecretError;
    use crate::secrets::testing::FakeSecrets;
    use serde_json::json;
    use std::time::Duration;

    fn observed(event: SecretChangedEvent) -> SecretChangedObserved {
        SecretChangedObserved {
            output: Some(event),
        }
    }

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
        assert_eq!(
            event_secret(&event(Some("A"), None)).as_deref(),
            Some("secret://A")
        );
        assert_eq!(
            event_secret(&event(None, Some("secret://B"))).as_deref(),
            Some("secret://B")
        );
        // The reference names the store; the bare name alone means the vault.
        assert_eq!(
            event_secret(&event(Some("C"), Some("env://C"))).as_deref(),
            Some("env://C")
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
        fake.set("secret://SHARED", Ok("v1"));
        let secrets = Arc::new(
            fake.cache()
                .with_listener(refresh_referencing_providers(config.clone(), queue)),
        );
        secrets.ensure(["secret://SHARED".to_string()]).await;
        let handler = make_on_secret_changed(config, secrets.clone());

        // rotated
        fake.set("secret://SHARED", Ok("v2"));
        let ack = handler(observed(SecretChangedEvent {
            name: Some("SHARED".into()),
            reference: Some("secret://SHARED".into()),
            action: Some("rotated".into()),
        }))
        .await
        .unwrap();
        assert!(ack.ok);
        assert_eq!(secrets.lookup("secret://SHARED"), Some(Ok("v2".into())));
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
        fake.set("secret://SHARED", Err(SecretError::NotFound));
        handler(observed(SecretChangedEvent {
            name: Some("SHARED".into()),
            reference: None,
            action: Some("deleted".into()),
        }))
        .await
        .unwrap();
        assert_eq!(
            secrets.lookup("secret://SHARED"),
            Some(Err(SecretError::NotFound))
        );
        flush().await;
        assert_eq!(fired.lock().unwrap().len(), 2);

        // an event that changes nothing nudges nobody
        fired.lock().unwrap().clear();
        handler(observed(SecretChangedEvent {
            name: Some("SHARED".into()),
            reference: None,
            action: Some("access_changed".into()),
        }))
        .await
        .unwrap();
        flush().await;
        assert!(fired.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_env_event_refreshes_only_the_env_reference() {
        let config = new_config_cell(json!({ "providers": {
            "openai": { "api_key": "env://OPENAI_API_KEY" },
            "other": { "api_key": "secret://OPENAI_API_KEY" },
        }}));
        let fake = FakeSecrets::default();
        fake.set("env://OPENAI_API_KEY", Ok("from-dotenv"));
        let secrets = Arc::new(fake.cache());
        let handler = make_on_secret_changed(config, secrets.clone());
        handler(observed(SecretChangedEvent {
            name: Some("OPENAI_API_KEY".into()),
            reference: Some("env://OPENAI_API_KEY".into()),
            action: Some("rotated".into()),
        }))
        .await
        .unwrap();
        assert_eq!(fake.calls(), vec!["env://OPENAI_API_KEY".to_string()]);
        assert_eq!(
            secrets.lookup("env://OPENAI_API_KEY"),
            Some(Ok("from-dotenv".into()))
        );
    }

    #[tokio::test]
    async fn unreferenced_secrets_are_not_fetched() {
        let config = new_config_cell(json!({ "providers": { "openai": { "api_key": "sk" } } }));
        let fake = FakeSecrets::default();
        let handler = make_on_secret_changed(config, Arc::new(fake.cache()));
        let ack = handler(observed(SecretChangedEvent {
            name: Some("SOMEONE_ELSES".into()),
            reference: None,
            action: Some("created".into()),
        }))
        .await
        .unwrap();
        assert!(ack.ok);
        assert!(fake.calls().is_empty());
    }

    /// The observer captures the event as output: the target reads it from
    /// the Observed envelope.
    #[tokio::test]
    async fn an_observed_envelope_refreshes_the_secret_named_in_its_output() {
        let config = new_config_cell(json!({ "providers": {
            "anthropic": { "api_key": "secret://SHARED" },
        }}));
        let fake = FakeSecrets::default();
        fake.set("secret://SHARED", Ok("v2"));
        let handler = make_on_secret_changed(config, Arc::new(fake.cache()));
        let envelope = json!({
            "observer_id": "obs-1",
            "activation_id": "act-1",
            "function_id": "secrets::on-changed",
            "invocation_id": "inv-1",
            "status": "success",
            "started_at_ms": 1,
            "duration_ms": 0.5,
            "output": { "name": "SHARED", "action": "rotated", "fingerprint": "f" },
        });
        let ack = handler(serde_json::from_value(envelope).unwrap())
            .await
            .unwrap();
        assert!(ack.ok);
        assert_eq!(fake.calls(), vec!["secret://SHARED".to_string()]);
    }
}
