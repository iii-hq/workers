//! (Re)register, fetch, and write the llm-router configuration entry through
//! engine's `configuration::register/get/set` iii functions. Never passes
//! initial_value, so operator-stored values survive every re-register.
use std::collections::BTreeMap;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

use super::schema::compose_entry_schema;

pub const ENTRY_ID: &str = "llm-router";

/// Serializes entry mutations and authoritative reloads so an older reload
/// cannot overwrite the snapshot produced by a newer credential write.
pub type EntryWriteLock = std::sync::Arc<tokio::sync::Mutex<()>>;

pub async fn register_entry(
    iii: &IIIClient,
    provider_schemas: &BTreeMap<String, Value>,
) -> Result<(), Error> {
    migrate_provider_system_prompts(iii).await?;
    iii.trigger(
        TriggerRequest {
            function_id: "configuration::register".into(),
            payload: json!({
                "id": ENTRY_ID,
                "name": "LLM Router",
                "description": "Provider credentials, routing heuristics, and stream budgets for llm-router.",
                "schema": compose_entry_schema(provider_schemas),
            }),
            action: None,
            timeout_ms: None,
        }
        .namespace("default"),
    )
    .await?;
    Ok(())
}

/// Fetch the authoritative, env-expanded entry value.
///
/// Registration precedes every fetch during normal operation, so a missing
/// entry or transport failure is surfaced instead of replacing the last-good
/// in-memory snapshot with `null`.
pub async fn read_entry_value(iii: &IIIClient) -> Result<Value, Error> {
    read_entry_value_with_raw(iii, false).await
}

async fn read_entry_value_with_raw(iii: &IIIClient, raw: bool) -> Result<Value, Error> {
    let response: Value = iii
        .trigger(
            TriggerRequest {
                function_id: "configuration::get".into(),
                payload: json!({ "id": ENTRY_ID, "raw": raw }),
                action: None,
                timeout_ms: None,
            }
            .namespace("default"),
        )
        .await?;
    Ok(response.get("value").cloned().unwrap_or(Value::Null))
}

pub async fn write_entry_value(iii: &IIIClient, value: Value) -> Result<(), Error> {
    set_entry_value(iii, value).await?;
    Ok(())
}

async fn set_entry_value(iii: &IIIClient, value: Value) -> Result<Value, Error> {
    iii.trigger(
        TriggerRequest {
            function_id: "configuration::set".into(),
            payload: json!({ "id": ENTRY_ID, "value": value }),
            action: None,
            timeout_ms: None,
        }
        .namespace("default"),
    )
    .await
}

/// Remove retired provider-owned prompt overrides before the final strict
/// schema is registered. The raw read/write preserves environment templates.
async fn migrate_provider_system_prompts(iii: &IIIClient) -> Result<(), Error> {
    let value = match read_entry_value_with_raw(iii, true).await {
        Ok(value) => value,
        Err(error) if error.to_string().contains("NOT_FOUND") => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut migrated = value.clone();
    if !drop_provider_system_prompts(&mut migrated) {
        return Ok(());
    }
    iii.trigger(
        TriggerRequest {
            function_id: "configuration::register".into(),
            payload: json!({
                "id": ENTRY_ID,
                "name": "LLM Router",
                "description": "LLM Router configuration migration.",
                "schema": {
                    "type": ["object", "null"],
                    "additionalProperties": true
                }
            }),
            action: None,
            timeout_ms: None,
        }
        .namespace("default"),
    )
    .await?;

    let mut expected = value;
    loop {
        let response = set_entry_value(iii, migrated.clone()).await?;
        let overwritten = response.get("old_value").cloned().ok_or_else(|| {
            Error::Handler(
                "configuration::set response missing old_value during llm-router migration".into(),
            )
        })?;
        let Some(rebased) = reconcile_provider_prompt_write(&mut expected, &migrated, overwritten)
        else {
            return Ok(());
        };
        tracing::warn!(
            "llm-router configuration changed during migration; preserving the intervening update"
        );
        migrated = rebased;
    }
}

fn drop_provider_system_prompts(value: &mut Value) -> bool {
    let Some(providers) = value.get_mut("providers").and_then(Value::as_object_mut) else {
        return false;
    };
    let mut changed = false;
    for provider in providers.values_mut() {
        if let Some(settings) = provider.as_object_mut() {
            changed |= settings.remove("system_prompt").is_some();
        }
    }
    changed
}

fn reconcile_provider_prompt_write(
    expected: &mut Value,
    written: &Value,
    mut overwritten: Value,
) -> Option<Value> {
    // `old_value` is atomic with the set. A mismatch means this write replaced
    // a newer snapshot, so restore that snapshot with only the retired keys gone.
    if overwritten == *expected {
        return None;
    }
    *expected = written.clone();
    drop_provider_system_prompts(&mut overwritten);
    Some(overwritten)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_prompt_migration_preserves_an_intervening_update() {
        let mut expected =
            json!({ "providers": { "custom": { "api_key": "old", "system_prompt": "legacy" } } });
        let first_write = json!({ "providers": { "custom": { "api_key": "old" } } });

        let rebased = reconcile_provider_prompt_write(
            &mut expected,
            &first_write,
            json!({ "providers": { "custom": { "api_key": "new", "system_prompt": "legacy" } } }),
        )
        .expect("the intervening value needs a compensating write");

        assert_eq!(
            rebased,
            json!({ "providers": { "custom": { "api_key": "new" } } })
        );
        assert!(reconcile_provider_prompt_write(&mut expected, &rebased, first_write).is_none());
    }
}
