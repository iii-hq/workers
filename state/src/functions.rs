//! The six `state::*` service functions, SDK-registered. Ids, inputs, outputs,
//! and descriptions are byte-parity with the builtin's `#[function]` macros
//! (state.rs:569-761). Error codes become message prefixes (SDK handlers carry
//! a message, not a coded body) — documented in the README parity table.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::Value;
use tokio::sync::RwLock;

use crate::adapters::StateAdapter;
use crate::config::StateConfig;
use crate::events::{Invoker, fan_out};
use crate::structs::{
    StateDeleteInput, StateEventData, StateEventType, StateGetGroupInput, StateGetInput,
    StateListGroupsInput, StateListGroupsResult, StateListKeysResult, StateSetInput,
    StateUpdateInput,
};
use crate::trigger::TriggerTable;

pub type ConfigCell = Arc<RwLock<Arc<StateConfig>>>;

/// Everything a function handler needs; one Arc cloned per registration.
pub struct StateCtx {
    pub adapter: Arc<dyn StateAdapter>,
    pub triggers: TriggerTable,
    pub config: ConfigCell,
    pub invoker: Arc<dyn Invoker>,
}

impl StateCtx {
    async fn snapshot(&self) -> Arc<StateConfig> {
        self.config.read().await.clone()
    }

    async fn emit(&self, event: StateEventData) {
        let enabled = self.snapshot().await.triggers_enabled.unwrap_or(true);
        fan_out(self.invoker.clone(), &self.triggers, enabled, event).await;
    }
}

/// Typed "JSON value or null" response schema override.
///
/// `state::get`, `state::delete`, and `state::list` all return
/// `Option<serde_json::Value>` on the wire (the raw stored value/group, or
/// `null`) — that shape is intentionally polymorphic, so it can't be a
/// concrete `#[derive(JsonSchema)]` struct. Left to `schemars`, `Option<Value>`
/// auto-extracts to the permissive `{"title": "Nullable_AnyValue"}` schema
/// (no `type`/`properties`/... keyword), which the registry publish gate
/// (`collect_worker_interface.py --assert-typed-schemas`) rejects as
/// "unknown". This override documents the exact same permissive shape with an
/// explicit `type` keyword instead — it only annotates the registered schema
/// and never changes what the handler returns.
fn json_value_or_null_schema(title: &str, description: &str) -> Value {
    serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": title,
        "description": description,
        "type": ["string", "number", "boolean", "object", "array", "null"],
    })
}

fn event(
    event_type: StateEventType,
    scope: String,
    key: String,
    old_value: Option<Value>,
    new_value: Value,
) -> StateEventData {
    StateEventData {
        message_type: "state".to_string(),
        event_type,
        scope,
        key,
        old_value,
        new_value,
    }
}

pub fn register_functions(iii: &Arc<IIIClient>, ctx: Arc<StateCtx>) {
    // state::set — max_value_bytes LIVE guard before the adapter write.
    {
        let ctx = ctx.clone();
        iii.register_function(
            "state::set",
            RegisterFunction::new_async(move |input: StateSetInput| {
                let ctx = ctx.clone();
                async move {
                    if let Some(limit) = ctx.snapshot().await.max_value_bytes {
                        let size = serde_json::to_vec(&input.value)
                            .map(|b| b.len())
                            .unwrap_or(0);
                        if size > limit {
                            return Err(Error::Handler(format!(
                                "VALUE_TOO_LARGE: value of {size} bytes exceeds the configured \
                                 max_value_bytes limit of {limit}"
                            )));
                        }
                    }
                    let result = ctx
                        .adapter
                        .set(&input.scope, &input.key, input.value.clone())
                        .await
                        .map_err(|e| {
                            Error::Handler(format!("SET_ERROR: Failed to set value: {e}"))
                        })?;
                    let et = if result.old_value.is_none() {
                        StateEventType::Created
                    } else {
                        StateEventType::Updated
                    };
                    ctx.emit(event(
                        et,
                        input.scope,
                        input.key,
                        result.old_value.clone(),
                        result.new_value.clone(),
                    ))
                    .await;
                    Ok(result)
                }
            })
            .description("Set a value in state"),
        );
    }

    // state::get
    {
        let ctx = ctx.clone();
        iii.register_function(
            "state::get",
            RegisterFunction::new_async(move |input: StateGetInput| {
                let ctx = ctx.clone();
                async move {
                    ctx.adapter
                        .get(&input.scope, &input.key)
                        .await
                        .map_err(|e| Error::Handler(format!("GET_ERROR: Failed to get value: {e}")))
                }
            })
            .description("Get a value from state")
            .response_format(json_value_or_null_schema(
                "StateGetResponse",
                "The raw value stored at scope/key, or null if absent.",
            )),
        );
    }

    // state::delete — read-before-delete (returns the deleted value), GET_ERROR
    // if the lookup fails; the deleted event fires even when nothing existed.
    {
        let ctx = ctx.clone();
        iii.register_function(
            "state::delete",
            RegisterFunction::new_async(move |input: StateDeleteInput| {
                let ctx = ctx.clone();
                async move {
                    let old = ctx
                        .adapter
                        .get(&input.scope, &input.key)
                        .await
                        .map_err(|e| {
                            Error::Handler(format!(
                                "GET_ERROR: Failed to get value before delete: {e}"
                            ))
                        })?;
                    ctx.adapter
                        .delete(&input.scope, &input.key)
                        .await
                        .map_err(|e| {
                            Error::Handler(format!("DELETE_ERROR: Failed to delete value: {e}"))
                        })?;
                    ctx.emit(event(
                        StateEventType::Deleted,
                        input.scope,
                        input.key,
                        old.clone(),
                        Value::Null,
                    ))
                    .await;
                    Ok(old)
                }
            })
            .description("Delete a value from state")
            .response_format(json_value_or_null_schema(
                "StateDeleteResponse",
                "The value that was deleted (read before delete), or null if it did not exist.",
            )),
        );
    }

    // state::update
    {
        let ctx = ctx.clone();
        iii.register_function(
            "state::update",
            RegisterFunction::new_async(move |input: StateUpdateInput| {
                let ctx = ctx.clone();
                async move {
                    let result = ctx
                        .adapter
                        .update(&input.scope, &input.key, input.ops)
                        .await
                        .map_err(|e| {
                            Error::Handler(format!("UPDATE_ERROR: Failed to update value: {e}"))
                        })?;
                    let et = if result.old_value.is_none() {
                        StateEventType::Created
                    } else {
                        StateEventType::Updated
                    };
                    ctx.emit(event(
                        et,
                        input.scope,
                        input.key,
                        result.old_value.clone(),
                        result.new_value.clone(),
                    ))
                    .await;
                    Ok(result)
                }
            })
            .description("Update a value in state"),
        );
    }

    // state::list — always Some(array) (builtin serializes the Vec).
    {
        let ctx = ctx.clone();
        iii.register_function(
            "state::list",
            RegisterFunction::new_async(move |input: StateGetGroupInput| {
                let ctx = ctx.clone();
                async move {
                    let values = ctx.adapter.list(&input.scope).await.map_err(|e| {
                        Error::Handler(format!("LIST_ERROR: Failed to list values: {e}"))
                    })?;
                    Ok(serde_json::to_value(values).ok())
                }
            })
            .description("Get a group from state")
            .response_format(json_value_or_null_schema(
                "StateListResponse",
                "The values in the given scope, as a JSON array (or null on the \
                 rare serialization failure).",
            )),
        );
    }

    // state::list_keys — keys within a scope. Added alongside the console
    // state UI: state::list returns values only, which cannot drive per-item
    // navigation (no builtin counterpart; additive surface).
    {
        let ctx = ctx.clone();
        iii.register_function(
            "state::list_keys",
            RegisterFunction::new_async(move |input: StateGetGroupInput| {
                let ctx = ctx.clone();
                async move {
                    let keys = ctx.adapter.list_keys(&input.scope).await.map_err(|e| {
                        Error::Handler(format!("LIST_KEYS_ERROR: Failed to list keys: {e}"))
                    })?;
                    Ok(StateListKeysResult { keys })
                }
            })
            .description("List the keys stored in a scope"),
        );
    }

    // state::list_groups — dedup + sort (builtin parity).
    {
        let ctx = ctx.clone();
        iii.register_function(
            "state::list_groups",
            RegisterFunction::new_async(move |_input: StateListGroupsInput| {
                let ctx = ctx.clone();
                async move {
                    let groups = ctx.adapter.list_groups().await.map_err(|e| {
                        Error::Handler(format!("LIST_GROUPS_ERROR: Failed to list groups: {e}"))
                    })?;
                    let mut normalized: Vec<String> = groups
                        .into_iter()
                        .collect::<std::collections::HashSet<_>>()
                        .into_iter()
                        .collect();
                    normalized.sort();
                    Ok(StateListGroupsResult { groups: normalized })
                }
            })
            .description("List all state groups"),
        );
    }
}
