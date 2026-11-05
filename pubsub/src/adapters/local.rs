//! In-process backend (default): events stay on this engine. iii already
//! delivers each `publish` call to the observers of `publish` here, so there
//! is nothing else to carry.

use async_trait::async_trait;
use serde_json::Value;

use super::PubSubAdapter;

pub struct LocalAdapter;

#[async_trait]
impl PubSubAdapter for LocalAdapter {
    async fn publish(&self, topic: &str, _data: Value) {
        tracing::debug!(topic = %topic, "Event published on this engine");
    }

    async fn shutdown(&self) {}
}
