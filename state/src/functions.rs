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

/// Control-plane state is not agent state. A worker CLAIMS its private
/// namespace at runtime (`state::claim-namespace`): the claim reserves its
/// scopes — public `state::*` calls must never read, mutate, list, or fan
/// them out to state triggers — and registers internal accessors under the
/// claimant's own function-id prefix (`<prefix>::state::{get, list,
/// compare-and-set}`).
///
/// This worker knows nothing about WHO claims, and needs no configuration:
/// a claim is authorized by the engine-stamped `_caller_worker_id` (the
/// engine OVERWRITES it on every dispatch, so it cannot be forged), and a
/// worker may only claim the namespace matching its own worker NAME. That is
/// exactly the function-id namespace it already owns — only the worker named
/// `harness` can own `harness::state::*`.
///
/// Claims persist in [`CLAIMS_SCOPE`] and are re-registered at boot, so a
/// state restart restores the accessors without the tenant lifting a finger.
/// Scopes are first-claimant-wins; re-claiming your own namespace is a no-op.
pub const CLAIMS_SCOPE: &str = "__state_private_namespaces";
pub const CLAIM_NAMESPACE_ID: &str = "state::claim-namespace";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PrivateNamespace {
    /// Function-id prefix for the accessors, e.g. `harness` →
    /// `harness::state::get`. Must equal the claimant's worker name.
    pub functions_prefix: String,
    /// The reserved storage scopes this namespace owns.
    pub scopes: Vec<String>,
}

/// The live claim registry: read on every public `state::*` call (guards) and
/// written by `state::claim-namespace`.
#[derive(Debug, Default)]
pub struct PrivateNamespaces {
    inner: std::sync::RwLock<Claims>,
}

#[derive(Debug, Default)]
struct Claims {
    namespaces: Vec<PrivateNamespace>,
    reserved: std::collections::HashSet<String>,
}

/// What a claim attempt did.
pub enum ClaimOutcome {
    /// Newly reserved these scopes. `namespace_is_new` distinguishes the
    /// FIRST claim for a prefix (register its accessors) from growing an
    /// existing one (accessors read their scopes live, so re-registering
    /// would fail with "function id already registered").
    Claimed {
        fresh: Vec<String>,
        namespace_is_new: bool,
    },
    /// Everything asked for is already owned by this prefix — no-op.
    AlreadyOwned,
    /// A scope belongs to a different namespace; nothing was changed.
    Conflict(String),
}

impl PrivateNamespaces {
    pub fn is_reserved(&self, scope: &str) -> bool {
        self.read().reserved.contains(scope)
    }

    /// The scopes a namespace owns — the hard bound on its own accessors.
    pub fn owned_scopes(&self, functions_prefix: &str) -> std::collections::HashSet<String> {
        self.read()
            .namespaces
            .iter()
            .filter(|n| n.functions_prefix == functions_prefix)
            .flat_map(|n| n.scopes.iter().cloned())
            .collect()
    }

    pub fn snapshot(&self) -> Vec<PrivateNamespace> {
        self.read().namespaces.clone()
    }

    /// Reserve `scopes` for `functions_prefix`. First-claimant-wins: a scope
    /// held by another namespace fails the whole claim, so a partial state is
    /// never observable.
    pub fn claim(&self, functions_prefix: &str, scopes: &[String]) -> ClaimOutcome {
        let mut claims = self.write();
        let mut fresh = Vec::new();
        for scope in scopes {
            if claims.reserved.contains(scope) {
                let owner = claims
                    .namespaces
                    .iter()
                    .find(|n| n.scopes.iter().any(|s| s == scope))
                    .map(|n| n.functions_prefix.as_str())
                    .unwrap_or_default();
                if owner != functions_prefix {
                    return ClaimOutcome::Conflict(format!(
                        "scope `{scope}` is already claimed by namespace `{owner}`"
                    ));
                }
                continue; // already ours
            }
            fresh.push(scope.clone());
        }
        if fresh.is_empty() {
            return ClaimOutcome::AlreadyOwned;
        }
        for scope in &fresh {
            claims.reserved.insert(scope.clone());
        }
        let namespace_is_new = match claims
            .namespaces
            .iter_mut()
            .find(|n| n.functions_prefix == functions_prefix)
        {
            Some(existing) => {
                existing.scopes.extend(fresh.iter().cloned());
                false
            }
            None => {
                claims.namespaces.push(PrivateNamespace {
                    functions_prefix: functions_prefix.to_string(),
                    scopes: fresh.clone(),
                });
                true
            }
        };
        ClaimOutcome::Claimed {
            fresh,
            namespace_is_new,
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Claims> {
        self.inner.read().unwrap_or_else(|p| p.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Claims> {
        self.inner.write().unwrap_or_else(|p| p.into_inner())
    }
}

/// A namespace prefix must be a usable function-id segment.
pub fn valid_prefix(functions_prefix: &str) -> bool {
    !functions_prefix.is_empty()
        && functions_prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
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
    // The claims ledger itself is never publicly reachable.
    if scope == CLAIMS_SCOPE || private.is_reserved(scope) {
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
        if event.scope == CLAIMS_SCOPE || self.private.is_reserved(&event.scope) {
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
    register_claim_namespace(iii, &ctx);

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
                        .filter(|scope| {
                            scope.as_str() != CLAIMS_SCOPE && !ctx.private.is_reserved(scope)
                        })
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

/// Input for [`CLAIM_NAMESPACE_ID`]. `_caller_worker_id` is engine-stamped
/// (overwritten on every dispatch, so a caller cannot forge it); the handler
/// resolves it to the caller's worker NAME and requires it to equal
/// `functions_prefix`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ClaimNamespaceInput {
    /// Must equal the calling worker's name.
    pub functions_prefix: String,
    /// Storage scopes to reserve for this namespace.
    pub scopes: Vec<String>,
    #[serde(rename = "_caller_worker_id", default)]
    pub caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ClaimNamespaceResult {
    /// False when the namespace already owned everything asked for.
    pub claimed: bool,
    /// Every scope this namespace owns after the call.
    pub scopes: Vec<String>,
    /// The accessor ids now serving this namespace.
    pub functions: Vec<String>,
}

/// Resolve the engine-stamped caller worker id to its registered NAME.
/// Engine builtins register with their name as id, so an exact id match is
/// accepted too.
async fn caller_worker_name(iii: &IIIClient, caller_worker_id: &str) -> Result<String, Error> {
    let workers = iii
        .trigger(iii_sdk::protocol::TriggerRequest {
            function_id: "engine::workers::list".to_string(),
            payload: serde_json::json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .map_err(|e| Error::Handler(format!("CALLER_LOOKUP_ERROR: {e}")))?;
    workers
        .get("workers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|worker| worker.get("id").and_then(Value::as_str) == Some(caller_worker_id))
        .and_then(|worker| worker.get("name").and_then(Value::as_str))
        .map(str::to_string)
        // An id the engine no longer lists cannot be authorized as a name.
        .ok_or_else(|| {
            Error::Handler(format!(
                "UNKNOWN_CALLER: worker id `{caller_worker_id}` is not registered"
            ))
        })
}

/// `state::claim-namespace` — a worker reserves its own private namespace.
/// Public by necessity (tenants call it) but authorized by identity: you may
/// only claim the namespace named after your own worker.
fn register_claim_namespace(iii: &Arc<IIIClient>, ctx: &Arc<StateCtx>) {
    let ctx = ctx.clone();
    let client = iii.clone();
    iii.register_function(
        CLAIM_NAMESPACE_ID,
        RegisterFunction::new_async(move |input: ClaimNamespaceInput| {
            let ctx = ctx.clone();
            let client = client.clone();
            async move {
                if !valid_prefix(&input.functions_prefix) {
                    return Err(Error::Handler(
                        "INVALID_PREFIX: functions_prefix must be non-empty [A-Za-z0-9_-]"
                            .to_string(),
                    ));
                }
                if input.scopes.iter().any(String::is_empty) {
                    return Err(Error::Handler(
                        "INVALID_SCOPE: scopes must be non-empty".to_string(),
                    ));
                }
                if input.scopes.iter().any(|s| s == CLAIMS_SCOPE) {
                    return Err(Error::Handler(format!(
                        "RESERVED_SCOPE: `{CLAIMS_SCOPE}` is the claims ledger"
                    )));
                }
                let caller_worker_id = input.caller_worker_id.clone().ok_or_else(|| {
                    Error::Handler(
                        "UNKNOWN_CALLER: the engine did not stamp _caller_worker_id".to_string(),
                    )
                })?;
                let caller = caller_worker_name(&client, &caller_worker_id).await?;
                if caller != input.functions_prefix {
                    return Err(Error::Handler(format!(
                        "FORBIDDEN: worker `{caller}` may only claim namespace `{caller}`, \
                         not `{}`",
                        input.functions_prefix
                    )));
                }

                match ctx.private.claim(&input.functions_prefix, &input.scopes) {
                    ClaimOutcome::Conflict(reason) => {
                        Err(Error::Handler(format!("SCOPE_CONFLICT: {reason}")))
                    }
                    outcome => {
                        let (claimed, namespace_is_new) = match outcome {
                            ClaimOutcome::Claimed {
                                namespace_is_new, ..
                            } => (true, namespace_is_new),
                            _ => (false, false),
                        };
                        let scopes: Vec<String> = {
                            let mut owned: Vec<String> = ctx
                                .private
                                .owned_scopes(&input.functions_prefix)
                                .into_iter()
                                .collect();
                            owned.sort();
                            owned
                        };
                        if claimed {
                            // Persist BEFORE registering: a crash between the
                            // two costs a restart's re-registration, never a
                            // silently unreserved scope.
                            persist_claim(&ctx, &input.functions_prefix, &scopes).await?;
                            // Only the FIRST claim registers accessors — they
                            // read their owned scopes live, so a grown claim
                            // needs nothing (and re-registering would fail).
                            if namespace_is_new {
                                register_private_namespace_functions(
                                    &client,
                                    &ctx,
                                    &PrivateNamespace {
                                        functions_prefix: input.functions_prefix.clone(),
                                        scopes: scopes.clone(),
                                    },
                                );
                            }
                            tracing::info!(
                                prefix = %input.functions_prefix,
                                ?scopes,
                                "private namespace claimed"
                            );
                        }
                        let (get_id, list_id, cas_id) = internal_ids(&input.functions_prefix);
                        Ok(ClaimNamespaceResult {
                            claimed,
                            scopes,
                            functions: vec![get_id, list_id, cas_id],
                        })
                    }
                }
            }
        })
        .description(
            "Reserve a PRIVATE state namespace for the calling worker: its scopes become \
             invisible to public state::* and to trigger fan-out, and internal accessors \
             `<functions_prefix>::state::{get, list, compare-and-set}` are registered. A worker \
             may only claim the namespace matching its own worker name (engine-stamped caller \
             identity). Idempotent; claims survive a state restart.",
        ),
    );
}

async fn persist_claim(
    ctx: &Arc<StateCtx>,
    functions_prefix: &str,
    scopes: &[String],
) -> Result<(), Error> {
    ctx.adapter
        .set(
            CLAIMS_SCOPE,
            functions_prefix,
            serde_json::json!({ "functions_prefix": functions_prefix, "scopes": scopes }),
        )
        .await
        .map(|_| ())
        .map_err(|e| Error::Handler(format!("CLAIM_PERSIST_ERROR: {e}")))
}

/// Re-arm claims persisted by earlier runs. Called at boot, before any tenant
/// can ask: a state restart restores the accessors on its own.
pub async fn restore_persisted_claims(iii: &Arc<IIIClient>, ctx: &Arc<StateCtx>) {
    let stored = match ctx.adapter.list(CLAIMS_SCOPE).await {
        Ok(values) => values,
        Err(error) => {
            tracing::warn!(%error, "could not read persisted private-namespace claims");
            return;
        }
    };
    for value in stored {
        let Ok(namespace) = serde_json::from_value::<PrivateNamespace>(value.clone()) else {
            tracing::error!(?value, "skipping unreadable private-namespace claim");
            continue;
        };
        match ctx
            .private
            .claim(&namespace.functions_prefix, &namespace.scopes)
        {
            ClaimOutcome::Conflict(reason) => {
                tracing::error!(
                    prefix = %namespace.functions_prefix,
                    %reason,
                    "persisted claim conflicts; skipped"
                );
            }
            _ => {
                register_private_namespace_functions(iii, ctx, &namespace);
                tracing::info!(
                    prefix = %namespace.functions_prefix,
                    scopes = ?namespace.scopes,
                    "private namespace restored"
                );
            }
        }
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
    namespace: &PrivateNamespace,
) {
    let internal = serde_json::json!({ "internal": true, "trace_hidden": true });
    let prefix = namespace.functions_prefix.clone();
    let (get_id, list_id, cas_id) = internal_ids(&prefix);

    {
        let ctx = ctx.clone();
        let prefix = prefix.clone();
        iii.register_function(
            &get_id,
            RegisterFunction::new_async(move |input: StateGetInput| {
                let ctx = ctx.clone();
                let prefix = prefix.clone();
                async move {
                    require_owned_scope(&ctx.private.owned_scopes(&prefix), &prefix, &input.scope)?;
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
        let prefix = prefix.clone();
        iii.register_function(
            &list_id,
            RegisterFunction::new_async(move |input: StateGetGroupInput| {
                let ctx = ctx.clone();
                let prefix = prefix.clone();
                async move {
                    require_owned_scope(&ctx.private.owned_scopes(&prefix), &prefix, &input.scope)?;
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
                let prefix = prefix.clone();
                async move {
                    require_owned_scope(&ctx.private.owned_scopes(&prefix), &prefix, &input.scope)?;
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
mod private_namespace_tests {
    use super::*;

    fn scopes(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn claiming_reserves_scopes_and_is_idempotent() {
        // Neutral tenant names on purpose: this worker knows no tenant.
        let private = PrivateNamespaces::default();
        assert!(!private.is_reserved("acme_ledger"));

        assert!(matches!(
            private.claim("acme", &scopes(&["acme_ledger", "acme_owner"])),
            ClaimOutcome::Claimed { fresh, namespace_is_new: true } if fresh.len() == 2
        ));
        assert!(private.is_reserved("acme_ledger"));
        assert!(reject_reserved_scope(&private, "acme_ledger").is_err());
        assert!(reject_reserved_scope(&private, "agent_state").is_ok());

        // Re-claiming the same namespace is a no-op — the harness re-claims
        // on every boot and after any state restart.
        assert!(matches!(
            private.claim("acme", &scopes(&["acme_ledger", "acme_owner"])),
            ClaimOutcome::AlreadyOwned
        ));
        // Growing an existing claim adds only the new scope.
        assert!(matches!(
            private.claim("acme", &scopes(&["acme_ledger", "acme_extra"])),
            // Grown, not new: the accessors already exist.
            ClaimOutcome::Claimed { fresh, namespace_is_new: false }
                if fresh == vec!["acme_extra".to_string()]
        ));
    }

    #[test]
    fn a_scope_belongs_to_its_first_claimant() {
        let private = PrivateNamespaces::default();
        private.claim("first", &scopes(&["shared"]));
        match private.claim("second", &scopes(&["second_own", "shared"])) {
            ClaimOutcome::Conflict(reason) => assert!(reason.contains("`first`"), "{reason}"),
            _ => panic!("a foreign scope must fail the claim"),
        }
        // All-or-nothing: the conflicting claim reserved nothing at all.
        assert!(!private.is_reserved("second_own"));
    }

    #[test]
    fn accessors_are_hard_scoped_to_their_own_namespace() {
        let private = PrivateNamespaces::default();
        private.claim("acme", &scopes(&["acme_ledger"]));
        private.claim("wf", &scopes(&["wf_runs"]));

        let acme = private.owned_scopes("acme");
        assert!(require_owned_scope(&acme, "acme", "acme_ledger").is_ok());
        // Another tenant's reserved scope is NOT reachable through this
        // namespace's accessors — reserved-but-foreign is still denied.
        assert!(require_owned_scope(&acme, "acme", "wf_runs").is_err());
        assert!(require_owned_scope(&acme, "acme", "agent_state").is_err());
    }

    #[test]
    fn the_claims_ledger_is_never_publicly_reachable() {
        let private = PrivateNamespaces::default();
        assert!(reject_reserved_scope(&private, CLAIMS_SCOPE).is_err());
    }

    #[test]
    fn prefixes_must_be_usable_function_id_segments() {
        assert!(valid_prefix("harness"));
        assert!(valid_prefix("my-worker_2"));
        assert!(!valid_prefix(""));
        assert!(!valid_prefix("bad prefix"));
        assert!(!valid_prefix("bad::prefix"));
    }

    #[test]
    fn internal_ids_follow_the_prefix() {
        let (get, list, cas) = internal_ids("harness");
        assert_eq!(get, "harness::state::get");
        assert_eq!(list, "harness::state::list");
        assert_eq!(cas, "harness::state::compare-and-set");
    }

    #[test]
    fn persisted_claims_round_trip() {
        // What restore_persisted_claims() reads back at boot.
        let stored = serde_json::json!({
            "functions_prefix": "harness",
            "scopes": ["harness_binding", "harness_binding_owner"]
        });
        let namespace: PrivateNamespace = serde_json::from_value(stored).unwrap();
        assert_eq!(namespace.functions_prefix, "harness");
        let private = PrivateNamespaces::default();
        assert!(matches!(
            private.claim(&namespace.functions_prefix, &namespace.scopes),
            ClaimOutcome::Claimed { .. }
        ));
        assert_eq!(private.snapshot(), vec![namespace]);
    }
}
