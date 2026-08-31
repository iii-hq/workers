pub mod imap;
pub mod smtp;

use iii_sdk::{errors::Error, protocol::TriggerRequest, IIIClient};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub async fn fetch_vault_credential(
    iii: &IIIClient,
    account: &str,
    transport: &str,
) -> Result<Value, Error> {
    let cred = iii
        .trigger(TriggerRequest {
            function_id: "auth::get_token".to_string(),
            payload: json!({ "provider": format!("email::{account}") }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .map_err(|e| {
            Error::Handler(
                json!({
                    "code": "E606",
                    "message": format!("account `{account}` has no {transport}.username/password configured and auth::get_token failed for `email::{account}`: {e}")
                })
                .to_string(),
            )
        })?;
    if cred.is_null() {
        return Err(Error::Handler(
            json!({
                "code": "E607",
                "message": format!("account `{account}` has no {transport}.username/password configured and no credential is stored for `email::{account}`")
            })
            .to_string(),
        ));
    }
    Ok(cred)
}

/// Wire-identical mirror of `iii_sdk::channels::StreamChannelRef`. The SDK
/// type does not derive `JsonSchema`, which would block typed registration
/// of `email::search` and `email::attachment::get`. Converted via `From` at
/// the handler boundary so the rest of the worker keeps using the SDK type.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct StreamRef {
    pub channel_id: String,
    pub access_key: String,
    #[serde(default)]
    pub direction: StreamDirection,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum StreamDirection {
    Read,
    #[default]
    Write,
}

impl From<StreamRef> for iii_sdk::channels::StreamChannelRef {
    fn from(s: StreamRef) -> Self {
        iii_sdk::channels::StreamChannelRef {
            channel_id: s.channel_id,
            access_key: s.access_key,
            direction: match s.direction {
                StreamDirection::Read => iii_sdk::channels::ChannelDirection::Read,
                StreamDirection::Write => iii_sdk::channels::ChannelDirection::Write,
            },
        }
    }
}
