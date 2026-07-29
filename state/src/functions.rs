//! The six `state::*` service functions, SDK-registered. Ids, inputs, outputs,
//! and descriptions are byte-parity with the builtin's `#[function]` macros
//! (state.rs:569-761). Error codes become message prefixes (SDK handlers carry
//! a message, not a coded body) — documented in the README parity table.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
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

/// Binding authority is control-plane state, not agent state. The harness
/// reaches it through the internal functions below; public `state::*` calls
/// must never read, mutate, list, or fan it out to state triggers.
pub const BINDING_SCOPE: &str = "harness_binding";
pub const BINDING_OWNER_SCOPE: &str = "harness_binding_owner";
pub const HARNESS_GET_ID: &str = "harness::state::get";
pub const HARNESS_LIST_ID: &str = "harness::state::list";
pub const HARNESS_CAS_ID: &str = "harness::state::compare-and-set";

pub fn is_reserved_scope(scope: &str) -> bool {
    matches!(scope, BINDING_SCOPE | BINDING_OWNER_SCOPE)
}

fn reject_reserved_scope(scope: &str) -> Result<(), Error> {
    if is_reserved_scope(scope) {
        return Err(Error::Handler(format!(
            "RESERVED_SCOPE: `{scope}` is private harness bookkeeping"
        )));
    }
    Ok(())
}

fn require_reserved_scope(scope: &str) -> Result<(), Error> {
    if !is_reserved_scope(scope) {
        return Err(Error::Handler(format!(
            "INVALID_SCOPE: `{scope}` is not harness binding state"
        )));
    }
    Ok(())
}

/// Everything a function handler needs; one Arc cloned per registration.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CompareAndSetInput {
    pub scope: String,
    pub key: String,
    /// The value the caller believes is there. Omit to mean "expect absent" —
    /// the set-if-absent form.
    #[serde(default)]
    pub expected: Option<Value>,
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CompareAndSetResult {
    pub swapped: bool,
    /// What is actually stored, when the swap did not happen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<Value>,
}

/// The condition-contract envelope. `event` is the fired event; the rest of
/// the envelope (binding, context) is accepted and ignored so the same
/// function works as a condition and as a direct call.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct BarrierInput {
    #[serde(default)]
    pub event: Value,
    #[serde(default)]
    pub condition_config: Option<crate::barrier::BarrierConfig>,
}

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
        if is_reserved_scope(&event.scope) {
            return;
        }
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
    register_harness_functions(iii, &ctx);

    // state::set — max_value_bytes LIVE guard before the adapter write.
    {
        let ctx = ctx.clone();
        iii.register_function(
            "state::set",
            RegisterFunction::new_async(move |input: StateSetInput| {
                let ctx = ctx.clone();
                async move {
                    reject_reserved_scope(&input.scope)?;
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

    // state::compare-and-set — the primitive a claim needs. `get` then `set`
    // from outside cannot express "swap only if nobody moved it": two callers
    // reading the same value both write, and both believe they won.
    {
        let ctx = ctx.clone();
        iii.register_function(
            "state::compare-and-set",
            RegisterFunction::new_async(move |input: CompareAndSetInput| {
                let ctx = ctx.clone();
                async move {
                    reject_reserved_scope(&input.scope)?;
                    let swapped = ctx
                        .adapter
                        .compare_and_set(
                            &input.scope,
                            &input.key,
                            input.expected.as_ref(),
                            input.value.clone(),
                        )
                        .await
                        .map_err(|e| Error::Handler(format!("CAS_ERROR: {e}")))?;
                    match swapped {
                        None => {
                            // Same event a plain set emits: a watcher must not
                            // miss a write just because it went through CAS.
                            let et = if input.expected.is_none() {
                                StateEventType::Created
                            } else {
                                StateEventType::Updated
                            };
                            ctx.emit(event(
                                et,
                                input.scope,
                                input.key,
                                input.expected,
                                input.value,
                            ))
                            .await;
                            Ok(CompareAndSetResult {
                                swapped: true,
                                current: None,
                            })
                        }
                        // The caller retries against `current` rather than
                        // paying another round trip to find out what changed.
                        Some(current) => Ok(CompareAndSetResult {
                            swapped: false,
                            current: Some(current),
                        }),
                    }
                }
            })
            .description(
                "Atomically set a value only if it currently equals `expected` (omit `expected` \
                 to mean 'only if absent'). Returns { swapped, current } — on a miss, `current` \
                 is what is actually there, so a caller can recompute and retry. The primitive \
                 for counters, claims and any read-modify-write that two callers might race.",
            ),
        );
    }

    // state::barrier — fan-in as a condition. Registered beside the plain
    // state verbs because that is what it is: a function over one state key.
    {
        let ctx = ctx.clone();
        iii.register_function(
            "state::barrier",
            RegisterFunction::new_async(move |input: BarrierInput| {
                let ctx = ctx.clone();
                async move {
                    let cfg = input.condition_config.ok_or_else(|| {
                        Error::Handler(
                            "BARRIER_CONFIG: state::barrier needs `condition_config` \
                             { id, expect, key_from?, carry? } — as a binding it goes in \
                             `conditions: [{ function_id, config }]`"
                                .to_string(),
                        )
                    })?;
                    let decision = ctx
                        .adapter
                        .barrier_arrive(
                            crate::barrier::BARRIER_SCOPE,
                            &cfg.id.clone(),
                            &cfg,
                            &input.event,
                        )
                        .await
                        .map_err(|e| Error::Handler(format!("BARRIER_ERROR: {e}")))?;
                    Ok(decision)
                }
            })
            .description(
                "Fan-in gate: record one arrival against a barrier and answer the typed \
                 condition decision — `skip` until the expected set has arrived, then `allow` \
                 EXACTLY ONCE carrying every arrival's payload. Use as a binding condition so a \
                 coordinator wakes once when N producers finish instead of once per producer.",
            ),
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
                    reject_reserved_scope(&input.scope)?;
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
                    reject_reserved_scope(&input.scope)?;
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
                    reject_reserved_scope(&input.scope)?;
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
                    reject_reserved_scope(&input.scope)?;
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
                    reject_reserved_scope(&input.scope)?;
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
                        .filter(|scope| !is_reserved_scope(scope))
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

/// Private persistence primitives used only by the harness worker. They skip
/// trigger fan-out by design: a binding claim must not recursively fire a
/// catch-all state subscription.
fn register_harness_functions(iii: &Arc<IIIClient>, ctx: &Arc<StateCtx>) {
    let internal = serde_json::json!({ "internal": true, "trace_hidden": true });

    {
        let ctx = ctx.clone();
        iii.register_function(
            HARNESS_GET_ID,
            RegisterFunction::new_async(move |input: StateGetInput| {
                let ctx = ctx.clone();
                async move {
                    require_reserved_scope(&input.scope)?;
                    ctx.adapter
                        .get(&input.scope, &input.key)
                        .await
                        .map_err(|e| Error::Handler(format!("GET_ERROR: {e}")))
                }
            })
            .description("Internal: read harness bookkeeping state")
            .metadata(internal.clone())
            .response_format(json_value_or_null_schema(
                "HarnessStateGetResponse",
                "The raw private harness value, or null if absent.",
            )),
        );
    }

    {
        let ctx = ctx.clone();
        iii.register_function(
            HARNESS_LIST_ID,
            RegisterFunction::new_async(move |input: StateGetGroupInput| {
                let ctx = ctx.clone();
                async move {
                    require_reserved_scope(&input.scope)?;
                    let values = ctx
                        .adapter
                        .list(&input.scope)
                        .await
                        .map_err(|e| Error::Handler(format!("LIST_ERROR: {e}")))?;
                    Ok(serde_json::to_value(values).ok())
                }
            })
            .description("Internal: list harness bookkeeping state")
            .metadata(internal.clone())
            .response_format(json_value_or_null_schema(
                "HarnessStateListResponse",
                "Private harness values in the given scope.",
            )),
        );
    }

    {
        let ctx = ctx.clone();
        iii.register_function(
            HARNESS_CAS_ID,
            RegisterFunction::new_async(move |input: CompareAndSetInput| {
                let ctx = ctx.clone();
                async move {
                    require_reserved_scope(&input.scope)?;
                    let swapped = ctx
                        .adapter
                        .compare_and_set(
                            &input.scope,
                            &input.key,
                            input.expected.as_ref(),
                            input.value,
                        )
                        .await
                        .map_err(|e| Error::Handler(format!("CAS_ERROR: {e}")))?;
                    Ok(match swapped {
                        None => CompareAndSetResult {
                            swapped: true,
                            current: None,
                        },
                        Some(current) => CompareAndSetResult {
                            swapped: false,
                            current: Some(current),
                        },
                    })
                }
            })
            .description("Internal: atomically update harness bookkeeping state")
            .metadata(internal),
        );
    }
}

#[cfg(test)]
mod reserved_scope_tests {
    use super::*;

    #[test]
    fn harness_scopes_are_reserved() {
        assert!(is_reserved_scope("harness_binding"));
        assert!(is_reserved_scope("harness_binding_owner"));
        assert!(!is_reserved_scope("harness_turn"));
        assert!(!is_reserved_scope("agent_state"));
        assert!(reject_reserved_scope("harness_binding").is_err());
        assert!(reject_reserved_scope("agent_state").is_ok());
        assert!(require_reserved_scope("harness_binding").is_ok());
        assert!(require_reserved_scope("agent_state").is_err());
    }
}
