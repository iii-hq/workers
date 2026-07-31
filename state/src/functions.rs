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

/// Control-plane state is not agent state. Workers claim PRIVATE namespaces
/// via `StateConfig::private_namespaces`; each claim reserves its scopes —
/// public `state::*` calls must never read, mutate, list, or fan them out to
/// state triggers — and registers internal accessors under the claim's own
/// function-id prefix (`<prefix>::state::{get, list, compare-and-set}`).
/// This worker knows nothing about WHO claims: the harness's binding
/// authority is just the first tenant, wired entirely from config.
#[derive(Debug, Default)]
pub struct PrivateNamespaces {
    namespaces: Vec<crate::config::PrivateNamespace>,
    reserved: std::collections::HashSet<String>,
}

impl PrivateNamespaces {
    /// Boot-time snapshot (restart-tier, like the adapter: the accessors
    /// register once at start). Invalid entries are skipped LOUDLY; a scope
    /// already claimed by an earlier namespace stays with the first claimant.
    pub fn new(configured: &[crate::config::PrivateNamespace]) -> Self {
        let mut namespaces: Vec<crate::config::PrivateNamespace> = Vec::new();
        let mut reserved = std::collections::HashSet::new();
        for entry in configured {
            let prefix = entry.functions_prefix.trim();
            if prefix.is_empty()
                || !prefix
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                tracing::error!(
                    functions_prefix = %entry.functions_prefix,
                    "private namespace skipped: functions_prefix must be non-empty [A-Za-z0-9_-]"
                );
                continue;
            }
            let scopes: Vec<String> = entry
                .scopes
                .iter()
                .filter(|scope| {
                    if scope.is_empty() {
                        tracing::error!(prefix, "private namespace scope skipped: empty");
                        return false;
                    }
                    if !reserved.insert((*scope).clone()) {
                        tracing::error!(
                            prefix,
                            scope = %scope,
                            "private namespace scope skipped: already claimed"
                        );
                        return false;
                    }
                    true
                })
                .cloned()
                .collect();
            if scopes.is_empty() {
                tracing::error!(prefix, "private namespace skipped: no usable scopes");
                continue;
            }
            namespaces.push(crate::config::PrivateNamespace {
                functions_prefix: prefix.to_string(),
                scopes,
            });
        }
        Self {
            namespaces,
            reserved,
        }
    }

    pub fn is_reserved(&self, scope: &str) -> bool {
        self.reserved.contains(scope)
    }

    fn iter(&self) -> impl Iterator<Item = &crate::config::PrivateNamespace> {
        self.namespaces.iter()
    }
}

/// The internal accessor ids a namespace claim registers.
pub fn internal_ids(functions_prefix: &str) -> (String, String, String) {
    (
        format!("{functions_prefix}::state::get"),
        format!("{functions_prefix}::state::list"),
        format!("{functions_prefix}::state::compare-and-set"),
    )
}

fn reject_reserved_scope(private: &PrivateNamespaces, scope: &str) -> Result<(), Error> {
    if private.is_reserved(scope) {
        return Err(Error::Handler(format!(
            "RESERVED_SCOPE: `{scope}` is private worker bookkeeping"
        )));
    }
    Ok(())
}

fn require_owned_scope(
    owned: &std::collections::HashSet<String>,
    prefix: &str,
    scope: &str,
) -> Result<(), Error> {
    if !owned.contains(scope) {
        return Err(Error::Handler(format!(
            "INVALID_SCOPE: `{scope}` is not private state of namespace `{prefix}`"
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
    /// Boot-time private-namespace snapshot (see [`PrivateNamespaces`]).
    pub private: Arc<PrivateNamespaces>,
}

impl StateCtx {
    async fn snapshot(&self) -> Arc<StateConfig> {
        self.config.read().await.clone()
    }

    async fn emit(&self, event: StateEventData) {
        if self.private.is_reserved(&event.scope) {
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
    for namespace in ctx.private.iter().cloned().collect::<Vec<_>>() {
        register_private_namespace_functions(iii, &ctx, &namespace);
    }

    // state::set — max_value_bytes LIVE guard before the adapter write.
    {
        let ctx = ctx.clone();
        iii.register_function(
            "state::set",
            RegisterFunction::new_async(move |input: StateSetInput| {
                let ctx = ctx.clone();
                async move {
                    reject_reserved_scope(&ctx.private, &input.scope)?;
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
                    reject_reserved_scope(&ctx.private, &input.scope)?;
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
                    reject_reserved_scope(&ctx.private, &input.scope)?;
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
                    reject_reserved_scope(&ctx.private, &input.scope)?;
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
                    reject_reserved_scope(&ctx.private, &input.scope)?;
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
                    reject_reserved_scope(&ctx.private, &input.scope)?;
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
                    reject_reserved_scope(&ctx.private, &input.scope)?;
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
                        .filter(|scope| !ctx.private.is_reserved(scope))
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

/// Private persistence primitives for the workers that claimed a namespace
/// in `StateConfig::private_namespaces`. They skip trigger fan-out by design:
/// a binding claim must not recursively fire a catch-all state subscription.
/// Each accessor is hard-scoped to ITS namespace's own scopes — one tenant
/// can never reach another tenant's bookkeeping.
fn register_private_namespace_functions(
    iii: &Arc<IIIClient>,
    ctx: &Arc<StateCtx>,
    namespace: &crate::config::PrivateNamespace,
) {
    let internal = serde_json::json!({ "internal": true, "trace_hidden": true });
    let prefix = namespace.functions_prefix.clone();
    let owned: Arc<std::collections::HashSet<String>> =
        Arc::new(namespace.scopes.iter().cloned().collect());
    let (get_id, list_id, cas_id) = internal_ids(&prefix);

    {
        let ctx = ctx.clone();
        let owned = owned.clone();
        let prefix = prefix.clone();
        iii.register_function(
            &get_id,
            RegisterFunction::new_async(move |input: StateGetInput| {
                let ctx = ctx.clone();
                let owned = owned.clone();
                let prefix = prefix.clone();
                async move {
                    require_owned_scope(&owned, &prefix, &input.scope)?;
                    ctx.adapter
                        .get(&input.scope, &input.key)
                        .await
                        .map_err(|e| Error::Handler(format!("GET_ERROR: {e}")))
                }
            })
            .description(format!(
                "Internal: read private `{}` bookkeeping state",
                namespace.functions_prefix
            ))
            .metadata(internal.clone())
            .response_format(json_value_or_null_schema(
                "PrivateStateGetResponse",
                "The raw private value, or null if absent.",
            )),
        );
    }

    {
        let ctx = ctx.clone();
        let owned = owned.clone();
        let prefix = prefix.clone();
        iii.register_function(
            &list_id,
            RegisterFunction::new_async(move |input: StateGetGroupInput| {
                let ctx = ctx.clone();
                let owned = owned.clone();
                let prefix = prefix.clone();
                async move {
                    require_owned_scope(&owned, &prefix, &input.scope)?;
                    let values = ctx
                        .adapter
                        .list(&input.scope)
                        .await
                        .map_err(|e| Error::Handler(format!("LIST_ERROR: {e}")))?;
                    Ok(serde_json::to_value(values).ok())
                }
            })
            .description(format!(
                "Internal: list private `{}` bookkeeping state",
                namespace.functions_prefix
            ))
            .metadata(internal.clone())
            .response_format(json_value_or_null_schema(
                "PrivateStateListResponse",
                "Private values in the given scope.",
            )),
        );
    }

    {
        let ctx = ctx.clone();
        iii.register_function(
            &cas_id,
            RegisterFunction::new_async(move |input: CompareAndSetInput| {
                let ctx = ctx.clone();
                let owned = owned.clone();
                let prefix = prefix.clone();
                async move {
                    require_owned_scope(&owned, &prefix, &input.scope)?;
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
            .description(format!(
                "Internal: atomically update private `{}` bookkeeping state",
                namespace.functions_prefix
            ))
            .metadata(internal),
        );
    }
}

#[cfg(test)]
mod reserved_scope_tests {
    use super::*;
    use crate::config::PrivateNamespace;

    fn ns(prefix: &str, scopes: &[&str]) -> PrivateNamespace {
        PrivateNamespace {
            functions_prefix: prefix.into(),
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn claimed_scopes_are_reserved_and_guarded() {
        // Neutral tenant names on purpose: nothing here knows about the
        // harness — its deployment config is just the first claimant.
        let private = PrivateNamespaces::new(&[
            ns("acme", &["acme_ledger", "acme_ledger_owner"]),
            ns("wf", &["wf_runs"]),
        ]);
        assert!(private.is_reserved("acme_ledger"));
        assert!(private.is_reserved("wf_runs"));
        assert!(!private.is_reserved("agent_state"));
        assert!(reject_reserved_scope(&private, "acme_ledger").is_err());
        assert!(reject_reserved_scope(&private, "agent_state").is_ok());
    }

    #[test]
    fn accessors_are_hard_scoped_to_their_own_namespace() {
        let owned: std::collections::HashSet<String> =
            ["acme_ledger".to_string()].into_iter().collect();
        assert!(require_owned_scope(&owned, "acme", "acme_ledger").is_ok());
        // Another tenant's reserved scope is NOT reachable through this
        // namespace's accessors — reserved-but-foreign is still denied.
        assert!(require_owned_scope(&owned, "acme", "wf_runs").is_err());
        assert!(require_owned_scope(&owned, "acme", "agent_state").is_err());
    }

    #[test]
    fn internal_ids_follow_the_prefix() {
        let (get, list, cas) = internal_ids("harness");
        assert_eq!(get, "harness::state::get");
        assert_eq!(list, "harness::state::list");
        assert_eq!(cas, "harness::state::compare-and-set");
    }

    #[test]
    fn invalid_and_duplicate_claims_are_skipped_loudly() {
        let private = PrivateNamespaces::new(&[
            ns("", &["a"]),           // empty prefix → skipped
            ns("bad prefix", &["b"]), // invalid chars → skipped
            ns("first", &["shared", "own"]),
            ns("second", &["shared"]), // duplicate scope → entry left empty → skipped
        ]);
        assert!(private.is_reserved("shared"));
        assert!(private.is_reserved("own"));
        assert!(!private.is_reserved("a"));
        assert!(!private.is_reserved("b"));
        // `shared` stayed with its first claimant.
        let owners: Vec<&str> = private
            .iter()
            .map(|n| n.functions_prefix.as_str())
            .collect();
        assert_eq!(owners, vec!["first"]);
    }
}
