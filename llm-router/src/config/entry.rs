//! (Re)register + read the llm-router configuration entry through the
//! engine's `configuration::register/get/set` iii functions. Never passes
//! initial_value, so operator-stored values survive every re-register.
use std::collections::BTreeMap;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

use super::schema::compose_entry_schema;

pub const ENTRY_ID: &str = "llm-router";

/// Serializes every llm-router entry mutation (register's schema re-compose,
/// update_credential's read-merge-write) — spec § "Serialized merges".
pub type EntryWriteLock = std::sync::Arc<tokio::sync::Mutex<()>>;

pub async fn register_entry(
    iii: &IIIClient,
    provider_schemas: &BTreeMap<String, Value>,
) -> Result<(), Error> {
    iii.trigger(TriggerRequest {
        function_id: "configuration::register".into(),
        payload: json!({
            "id": ENTRY_ID,
            "name": "LLM Router",
            "description": "Provider credentials, routing heuristics, and stream budgets for llm-router.",
            "schema": compose_entry_schema(provider_schemas),
        }),
        action: None,
        timeout_ms: None,
    })
    .await?;
    Ok(())
}

/// Null before the entry exists.
pub async fn read_entry_value(iii: &IIIClient) -> Value {
    let res: Result<Value, _> = iii
        .trigger(TriggerRequest {
            function_id: "configuration::get".into(),
            payload: json!({ "id": ENTRY_ID }),
            action: None,
            timeout_ms: None,
        })
        .await;
    match res {
        Ok(v) => v.get("value").cloned().unwrap_or(Value::Null),
        Err(_) => Value::Null, // NOT_FOUND before first registration
    }
}

pub async fn write_entry_value(iii: &IIIClient, value: Value) -> Result<(), Error> {
    iii.trigger(TriggerRequest {
        function_id: "configuration::set".into(),
        payload: json!({ "id": ENTRY_ID, "value": value }),
        action: None,
        timeout_ms: None,
    })
    .await?;
    Ok(())
}
