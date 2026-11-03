//! Pub/sub backends. iii delivers every `publish` call to the observers of
//! `publish` on this worker's engine; a backend only carries events to the
//! pubsub workers of other engines. A [`Relay`] hands an event that arrived
//! from another engine to this engine's observers, so backends stay
//! unit-testable without a live engine.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::config::PubSubConfig;

pub mod local;
pub mod redis;

pub use local::LocalAdapter;
pub use redis::RedisAdapter;

const DEFAULT_REDIS_URL: &str = "redis://localhost:6379";

/// Delivers an event another engine's pubsub worker published to the
/// observers of `publish` on this engine.
#[async_trait]
pub trait Relay: Send + Sync + 'static {
    /// Whether anything on this engine observes `publish` (one atomic load).
    fn is_observed(&self) -> bool;

    /// Calls `publish` on this engine with `{ topic, data }`, marked as
    /// relayed from `origin` so it is not carried to other engines again.
    async fn deliver(&self, topic: &str, data: Value, origin: &str);
}

/// A pub/sub backend.
#[async_trait]
pub trait PubSubAdapter: Send + Sync + 'static {
    /// Carries one event published on this engine to the other engines.
    async fn publish(&self, topic: &str, data: Value);

    /// Stops receiving events from other engines.
    async fn shutdown(&self);
}

/// Build the backend named in the config. Unknown names error (parity with
/// the builtin's adapter registry: "PubSub adapter factory '<name>' not found").
pub async fn build_adapter(
    config: &PubSubConfig,
    relay: Arc<dyn Relay>,
) -> anyhow::Result<Arc<dyn PubSubAdapter>> {
    match config.effective_adapter_name() {
        "local" => Ok(Arc::new(LocalAdapter)),
        "redis" => {
            let url = config
                .adapter
                .as_ref()
                .and_then(|a| a.config.as_ref())
                .and_then(|c| c.get("redis_url"))
                .and_then(|v| v.as_str())
                .unwrap_or(DEFAULT_REDIS_URL)
                .to_string();
            Ok(Arc::new(RedisAdapter::connect(&url, relay).await?))
        }
        other => {
            anyhow::bail!(
                "PubSub adapter factory '{other}' not found (expected 'local' or 'redis')"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PubSubConfig;

    struct NoopRelay;

    #[async_trait]
    impl Relay for NoopRelay {
        fn is_observed(&self) -> bool {
            false
        }
        async fn deliver(&self, _topic: &str, _data: serde_json::Value, _origin: &str) {}
    }

    #[tokio::test]
    async fn build_adapter_defaults_to_local() {
        let cfg = PubSubConfig::default();
        let adapter = build_adapter(&cfg, std::sync::Arc::new(NoopRelay)).await;
        assert!(adapter.is_ok());
    }

    #[tokio::test]
    async fn build_adapter_rejects_unknown_name() {
        let cfg: PubSubConfig = serde_yaml::from_str("{adapter: {name: kafka}}").unwrap();
        // `Arc<dyn PubSubAdapter>` isn't Debug, so avoid `unwrap_err`.
        let err = match build_adapter(&cfg, std::sync::Arc::new(NoopRelay)).await {
            Ok(_) => panic!("unknown adapter name must error"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("kafka"));
    }
}
