//! Registration-token persistence in iii-state (engine `state::*` functions,
//! binary-worker.md § 7). The raw token lives here, under the provider's own
//! scope; the router persists only its sha256 hash.
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

pub const STATE_SCOPE: &str = "provider-openrouter";
const TOKEN_KEY: &str = "registration_token";

pub async fn load_token(iii: &IIIClient) -> Option<String> {
    let value = iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({ "scope": STATE_SCOPE, "key": TOKEN_KEY }),
            action: None,
            timeout_ms: None,
        })
        .await
        .ok()?;
    value.as_str().map(String::from)
}

pub async fn store_token(iii: &IIIClient, token: &str) -> Result<(), Error> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".into(),
        payload: json!({ "scope": STATE_SCOPE, "key": TOKEN_KEY, "value": Value::from(token) }),
        action: None,
        timeout_ms: None,
    })
    .await?;
    Ok(())
}
