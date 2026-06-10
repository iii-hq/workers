//! (Re)register + read the llm-router configuration entry through the
//! engine's `configuration::register/get/set` iii functions. Never passes
//! initial_value, so operator-stored values survive every re-register.
use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{json, Value};

use super::schema::compose_entry_schema;
use crate::bus::{Bus, BusError};

pub const ENTRY_ID: &str = "llm-router";

/// Serializes every llm-router entry mutation (register's schema re-compose,
/// update_credential's read-merge-write) — spec § "Serialized merges".
pub type EntryWriteLock = std::sync::Arc<tokio::sync::Mutex<()>>;

pub async fn register_entry(
    bus: &Arc<dyn Bus>,
    provider_schemas: &BTreeMap<String, Value>,
) -> Result<(), BusError> {
    bus.trigger(
        "configuration::register",
        json!({
            "id": ENTRY_ID,
            "name": "LLM Router",
            "description": "Provider credentials, routing heuristics, and stream budgets for llm-router.",
            "schema": compose_entry_schema(provider_schemas),
        }),
        None,
    )
    .await?;
    Ok(())
}

/// Null before the entry exists.
pub async fn read_entry_value(bus: &Arc<dyn Bus>) -> Value {
    match bus
        .trigger("configuration::get", json!({ "id": ENTRY_ID }), None)
        .await
    {
        Ok(v) => v.get("value").cloned().unwrap_or(Value::Null),
        Err(_) => Value::Null, // NOT_FOUND before first registration
    }
}

pub async fn write_entry_value(bus: &Arc<dyn Bus>, value: Value) -> Result<(), BusError> {
    bus.trigger(
        "configuration::set",
        json!({ "id": ENTRY_ID, "value": value }),
        None,
    )
    .await?;
    Ok(())
}
