use async_trait::async_trait;
use iii_sdk::{
    errors::Error,
    trigger::{TriggerConfig, TriggerHandler},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

use super::registry::TriggerRegistry;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NewMailBindingConfig {
    account: String,
    #[serde(default = "default_folder")]
    folder: String,
    #[serde(default = "default_timeout")]
    handler_timeout_ms: u64,
}
fn default_folder() -> String {
    "INBOX".into()
}
fn default_timeout() -> u64 {
    30_000
}

pub struct Handler {
    pub registry: Arc<TriggerRegistry>,
}

#[async_trait]
impl TriggerHandler for Handler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let cfg: NewMailBindingConfig =
            serde_json::from_value(config.config.clone()).map_err(|e| {
                Error::Handler(
                    json!({
                        "code":"CONFIG_ERROR",
                        "message":format!("email::new-mail config: {e}")
                    })
                    .to_string(),
                )
            })?;
        self.registry.register(
            cfg.account.clone(),
            cfg.folder.clone(),
            config.id.clone(),
            config.function_id.clone(),
            cfg.handler_timeout_ms,
        );
        tracing::info!(
            instance = %config.id,
            function = %config.function_id,
            account = %cfg.account,
            folder = %cfg.folder,
            "email::new-mail trigger registered"
        );
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.registry.unregister(&config.id);
        tracing::info!(instance = %config.id, "email::new-mail trigger unregistered");
        Ok(())
    }
}
