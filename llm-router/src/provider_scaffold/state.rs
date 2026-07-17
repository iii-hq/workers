//! Registration-token persistence in iii-state (engine `state::*` functions,
//! binary-worker.md § 7). The raw token lives under the provider's own scope
//! (its worker id); the router persists only its sha256 hash.
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

const TOKEN_KEY: &str = "registration_token";

pub async fn load_token(iii: &IIIClient, scope: &str) -> Option<String> {
    let value = iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({ "scope": scope, "key": TOKEN_KEY }),
            action: None,
            timeout_ms: None,
        })
        .await
        .ok()?;
    value.as_str().map(String::from)
}

pub async fn store_token(iii: &IIIClient, scope: &str, token: &str) -> Result<(), Error> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".into(),
        payload: json!({ "scope": scope, "key": TOKEN_KEY, "value": Value::from(token) }),
        action: None,
        timeout_ms: None,
    })
    .await?;
    Ok(())
}
