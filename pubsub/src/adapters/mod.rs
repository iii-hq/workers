//! Pub/sub backends. Signature parity with the builtin trait
//! (engine/src/workers/pubsub/mod.rs:22-26); the engine handle becomes an
//! [`Invoker`] so adapters stay unit-testable without a live engine.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::config::PubSubConfig;

pub mod local;
pub mod redis;

pub use local::LocalAdapter;
pub use redis::RedisAdapter;

const DEFAULT_REDIS_URL: &str = "redis://localhost:6379";

/// Abstraction over `iii.trigger` (the builtin used `engine.call`). Fan-out
/// deliveries go through this; results are ignored by callers (fire-and-forget
/// parity with the builtin's `tokio::spawn(engine.call(..))`).
#[async_trait]
pub trait Invoker: Send + Sync + 'static {
    async fn call(&self, function_id: &str, payload: Value) -> Result<Option<Value>, String>;
}

/// Exact method parity with the builtin `PubSubAdapter`.
#[async_trait]
pub trait PubSubAdapter: Send + Sync + 'static {
    async fn publish(&self, topic: &str, data: Value);
    async fn subscribe(&self, topic: &str, id: &str, function_id: &str);
    async fn unsubscribe(&self, topic: &str, id: &str);
}

/// Build the backend named in the config. Unknown names error (parity with
/// the builtin's adapter registry: "PubSub adapter factory '<name>' not found").
pub async fn build_adapter(
    config: &PubSubConfig,
    invoker: Arc<dyn Invoker>,
) -> anyhow::Result<Arc<dyn PubSubAdapter>> {
    match config.effective_adapter_name() {
        "local" => Ok(Arc::new(LocalAdapter::new(invoker))),
        "redis" => {
            let url = config
                .adapter
                .as_ref()
                .and_then(|a| a.config.as_ref())
                .and_then(|c| c.get("redis_url"))
                .and_then(|v| v.as_str())
                .unwrap_or(DEFAULT_REDIS_URL)
                .to_string();
            Ok(Arc::new(RedisAdapter::connect(&url, invoker).await?))
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

    struct NoopInvoker;

    #[async_trait]
    impl Invoker for NoopInvoker {
        async fn call(
            &self,
            _function_id: &str,
            _payload: serde_json::Value,
        ) -> Result<Option<serde_json::Value>, String> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn build_adapter_defaults_to_local() {
        let cfg = PubSubConfig::default();
        let adapter = build_adapter(&cfg, std::sync::Arc::new(NoopInvoker)).await;
        assert!(adapter.is_ok());
    }

    #[tokio::test]
    async fn build_adapter_rejects_unknown_name() {
        let cfg: PubSubConfig = serde_yaml::from_str("{adapter: {name: kafka}}").unwrap();
        // `Arc<dyn PubSubAdapter>` isn't Debug, so avoid `unwrap_err`.
        let err = match build_adapter(&cfg, std::sync::Arc::new(NoopInvoker)).await {
            Ok(_) => panic!("unknown adapter name must error"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("kafka"));
    }
}
