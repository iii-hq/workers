use iii_sdk::{IIIError, TriggerRequest};
use serde_json::Value;

#[async_trait::async_trait]
pub trait IIITrigger: Send + Sync + 'static {
    async fn trigger(&self, request: TriggerRequest) -> Result<Value, IIIError>;
}

#[async_trait::async_trait]
impl IIITrigger for iii_sdk::III {
    async fn trigger(&self, request: TriggerRequest) -> Result<Value, IIIError> {
        iii_sdk::III::trigger(self, request).await
    }
}
