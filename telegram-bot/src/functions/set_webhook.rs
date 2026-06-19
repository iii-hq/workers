use std::sync::Arc;

use iii_sdk::{IIIError, III};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::clients::telegram;
use crate::config::UpdatesAdapter;
use crate::deps::Deps;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SetWebhookRequest {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SetWebhookResponse {
    pub ok: bool,
    pub url: String,
}

pub fn register(iii: &Arc<III>, deps: &Arc<Deps>) {
    super::register(
        iii,
        deps,
        super::SET_WEBHOOK_ID,
        "Register the Telegram webhook URL from configuration.",
        |d, _req: SetWebhookRequest| async move { handle(&d).await },
    );
}

async fn handle(deps: &Deps) -> Result<SetWebhookResponse, IIIError> {
    let cfg = deps.cfg().await;
    let UpdatesAdapter::Webhook(webhook) = &cfg.updates else {
        return Err(IIIError::Handler(
            "set-webhook requires updates adapter name: webhook".into(),
        ));
    };
    let Some(url) = webhook.endpoint_url() else {
        return Err(IIIError::Handler(
            "updates.webhook.config.base_url is not set in telegram-bot configuration".into(),
        ));
    };
    telegram::set_webhook(deps, &url, webhook.secret.as_deref()).await?;
    Ok(SetWebhookResponse { ok: true, url })
}
