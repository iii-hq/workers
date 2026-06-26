//! `slack::auth::test` and `slack::config-status` (identity + health badge).

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::clients::slack;
use crate::deps::{Deps, Identity};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct AuthTestReq {}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ConfigStatusReq {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ConfigStatus {
    /// True when the configured bot_token authenticates against Slack.
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterprise_id: Option<String>,
    /// True once the harness bridge lands and is enabled (always false in M1).
    pub bridge_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

async fn config_status(deps: &Deps) -> Result<ConfigStatus, Error> {
    match slack::auth_test(deps).await {
        Ok(id) => {
            *deps.identity.write().await = Some(id.clone());
            let Identity {
                team,
                team_id,
                user_id,
                bot_id,
                url,
                enterprise_id,
            } = id;
            Ok(ConfigStatus {
                ok: true,
                team,
                team_id,
                bot_user_id: user_id,
                bot_id,
                url,
                enterprise_id,
                bridge_enabled: false,
                error: None,
            })
        }
        Err(e) => Ok(ConfigStatus {
            ok: false,
            team: None,
            team_id: None,
            bot_user_id: None,
            bot_id: None,
            url: None,
            enterprise_id: None,
            bridge_enabled: false,
            error: Some(e.to_string()),
        }),
    }
}

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    super::register(
        iii,
        deps,
        "slack::auth::test",
        "Check the bot token and return the bot/workspace identity (auth.test).",
        |d, _req: AuthTestReq| async move {
            let value = crate::clients::slack::call(&d, "auth.test", serde_json::json!({})).await?;
            Ok(crate::response::SlackResponse::from_value(value))
        },
    );
    super::register(
        iii,
        deps,
        "slack::config-status",
        "Validate the configured Slack credentials and report identity + health (for the console badge).",
        |d, _req: ConfigStatusReq| async move { config_status(&d).await },
    );
}
