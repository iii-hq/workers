//! `directory::engine::*` — read-side enrichment over engine
//! introspection.
//!
//! Eight functions, all in the `<entity>::{list,info}` shape:
//!
//!   * `directory::engine::functions::list`            — list functions, filterable by search/prefix/worker
//!   * `directory::engine::functions::info`            — single function with schemas, registered triggers, how-to skill
//!   * `directory::engine::triggers::list`             — list trigger TYPES (templates), filterable
//!   * `directory::engine::triggers::info`             — single trigger type with schemas and instance count
//!   * `directory::engine::registered-triggers::list`  — list registered trigger INSTANCES, filterable
//!   * `directory::engine::registered-triggers::info`  — composite: instance + type + function
//!   * `directory::engine::workers::list`              — list connected workers, filterable
//!   * `directory::engine::workers::info`              — worker envelope + its functions + trigger types + registered triggers
//!
//! All handlers are thin wrappers around `III::trigger` calls to the
//! engine introspection endpoints (`engine::functions::list`,
//! `engine::workers::list`, `engine::trigger-types::list`,
//! `engine::triggers::list`) plus filesystem-backed how-to skill discovery
//! via [`crate::how_to`].
//!
//! Worker-name attribution: the SDK returns no `worker` field on
//! `FunctionInfo` / `TriggerTypeInfo` / `TriggerInfo`; we cross-reference
//! `WorkerInfo.functions[]` (canonical for functions and registered
//! triggers) and fall back to the first `::` segment of the id (only
//! signal available for trigger types).
//!
//! Parity with `directory::registry::*`: the `workers::list` and
//! `workers::info` shapes share their core fields (`name`,
//! `description`, `version`) and a top-level `worker` envelope so
//! callers learn one shape and switch between checking the running
//! engine vs the public registry without re-learning the API.

use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, TriggerRequest, III};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::SkillsConfig;
use crate::how_to::{self, RelatedSkillRef};

/// Function information returned by `engine::functions::list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SdkFunctionInfo {
    pub function_id: String,
    pub description: Option<String>,
    pub request_format: Option<Value>,
    pub response_format: Option<Value>,
    pub metadata: Option<Value>,
}

/// Trigger information returned by `engine::triggers::list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SdkTriggerInfo {
    pub id: String,
    pub trigger_type: String,
    pub function_id: String,
    pub config: Value,
    pub metadata: Option<Value>,
}

/// Trigger type information returned by `engine::trigger-types::list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TriggerTypeInfo {
    pub id: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_request_format: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_request_format: Option<Value>,
}

/// Worker information returned by `engine::workers::list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WorkerInfo {
    pub id: String,
    pub name: Option<String>,
    pub runtime: Option<String>,
    pub version: Option<String>,
    pub os: Option<String>,
    pub ip_address: Option<String>,
    pub status: String,
    pub connected_at_ms: u64,
    pub function_count: usize,
    pub functions: Vec<String>,
    pub active_invocations: usize,
    #[serde(default)]
    pub isolation: Option<String>,
}

// ---------- shared input/output shapes ----------

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct FunctionListInput {
    /// Case-insensitive substring match against `function_id` and `description`.
    #[serde(default)]
    pub search: Option<String>,
    /// Exact prefix match on `function_id` (e.g. `"mem::"`).
    #[serde(default)]
    pub prefix: Option<String>,
    /// Exact worker-name match (the worker that registered the function).
    #[serde(default)]
    pub worker: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FunctionListEntry {
    pub function_id: String,
    /// Worker that registered it (resolved via `WorkerInfo.functions[]`),
    /// or the first `::` segment of `function_id` as fallback.
    pub worker_name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FunctionListOutput {
    pub functions: Vec<FunctionListEntry>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct FunctionInfoInput {
    pub function_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RegisteredTriggerSummary {
    pub id: String,
    pub trigger_type: String,
    pub config: Value,
}

/// Primary how-to skill that documents this function. Kept tiny so
/// `function-info` stays cheap to render; deeper related skills come
/// back via [`FunctionInfoOutput::related_skills`] as title-only refs
/// that callers can pull on demand through `directory::skills::get`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HowGuide {
    pub title: String,
    pub skill_id: String,
    pub body: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FunctionInfoOutput {
    pub function_id: String,
    pub worker_name: Option<String>,
    pub description: Option<String>,
    pub request_schema: Option<Value>,
    pub response_schema: Option<Value>,
    pub metadata: Option<Value>,
    pub registered_triggers: Vec<RegisteredTriggerSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub how_guide: Option<HowGuide>,
    /// Other skills (any `type`) that mention this function via either
    /// the literal `function_id` or the `iii://fn/<dotted/path>` URI.
    /// Body content is omitted; fetch on demand via `directory::skills::get`.
    pub related_skills: Vec<RelatedSkillRef>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct TriggerListInput {
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub worker: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TriggerListEntry {
    pub id: String,
    pub worker_name: Option<String>,
    pub description: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TriggerListOutput {
    pub triggers: Vec<TriggerListEntry>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct TriggerInfoInput {
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TriggerInfoOutput {
    pub id: String,
    pub worker_name: Option<String>,
    pub description: String,
    /// SDK 0.11.3 surfaces a single `trigger_request_format` that doubles
    /// as the per-instance configuration shape; expose it explicitly so
    /// callers don't have to know the alias.
    pub configuration_schema: Option<Value>,
    pub return_schema: Option<Value>,
    pub instance_count: usize,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct RegisteredTriggerListInput {
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub trigger_type: Option<String>,
    #[serde(default)]
    pub function_id: Option<String>,
    #[serde(default)]
    pub worker: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RegisteredTriggerListEntry {
    pub id: String,
    pub trigger_type: String,
    pub function_id: String,
    pub worker_name: Option<String>,
    /// Truncated (~80 chars) JSON preview of `config` so listings stay
    /// scannable. Use `directory::registered-trigger-info` for the full
    /// payload.
    pub config_summary: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RegisteredTriggerListOutput {
    pub registered_triggers: Vec<RegisteredTriggerListEntry>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct RegisteredTriggerInfoInput {
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RegisteredTriggerInfoOutput {
    pub id: String,
    pub trigger_type: String,
    pub function_id: String,
    pub worker_name: Option<String>,
    pub config: Value,
    pub metadata: Option<Value>,
    /// Full trigger-type detail for `trigger_type`. `None` if the type
    /// has been unregistered between calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger: Option<TriggerInfoOutput>,
    /// Full function detail for `function_id`. `None` if the function
    /// has been unregistered between calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<FunctionInfoOutput>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct WorkerListInput {
    /// Case-insensitive substring match against `name`.
    #[serde(default)]
    pub search: Option<String>,
    /// Exact runtime match (e.g. `"rust"`, `"node"`).
    #[serde(default)]
    pub runtime: Option<String>,
    /// Exact status match (e.g. `"connected"`).
    #[serde(default)]
    pub status: Option<String>,
}

/// Shared worker envelope used by both `directory::worker-list` rows
/// and the `worker` field of `directory::worker-info`. Field names line
/// up with `registry::Worker` (see [`crate::functions::registry::Worker`])
/// so callers learn one shape across local + registry surfaces.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Worker {
    /// Worker name as registered with the engine.
    pub name: Option<String>,
    /// Engine-side workers carry no description; field present for
    /// shape parity with `registry::Worker.description`. Always `None`.
    pub description: Option<String>,
    /// Worker version string from the worker's published manifest.
    pub version: Option<String>,
    /// Engine-assigned connection id (directory-specific).
    pub id: String,
    pub runtime: Option<String>,
    pub os: Option<String>,
    /// Connection state (e.g. `"connected"`, `"disconnected"`).
    pub status: String,
    pub function_count: usize,
    pub connected_at_ms: u64,
    pub active_invocations: usize,
    pub isolation: Option<String>,
    pub ip_address: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WorkerListOutput {
    pub workers: Vec<Worker>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct WorkerInfoInput {
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WorkerFunctionEntry {
    pub function_id: String,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WorkerTriggerTypeEntry {
    pub id: String,
    pub description: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WorkerRegisteredTriggerEntry {
    pub id: String,
    pub trigger_type: String,
    pub function_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WorkerInfoOutput {
    /// Same shape as `worker-list` rows (and `registry::worker-info.worker`).
    pub worker: Worker,
    pub functions: Vec<WorkerFunctionEntry>,
    pub trigger_types: Vec<WorkerTriggerTypeEntry>,
    pub registered_triggers: Vec<WorkerRegisteredTriggerEntry>,
}

// ---------- registration ----------

pub fn register(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    register_function_list(iii);
    register_function_info(iii, cfg);
    register_trigger_list(iii);
    register_trigger_info(iii);
    register_registered_trigger_list(iii);
    register_registered_trigger_info(iii, cfg);
    register_worker_list(iii);
    register_worker_info(iii);
}

fn register_function_list(iii: &Arc<III>) {
    let iii_inner = iii.clone();
    iii.register_function(
        RegisterFunction::new_async(
            "directory::engine::functions::list",
            move |req: FunctionListInput| {
                let iii = iii_inner.clone();
                async move { function_list(&iii, req).await.map_err(IIIError::Handler) }
            },
        )
        .description(
            "List every function registered with the engine. Filter by free-text \
             search, namespace prefix, and/or worker name.",
        ),
    );
}

fn register_function_info(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async(
            "directory::engine::functions::info",
            move |req: FunctionInfoInput| {
                let iii = iii_inner.clone();
                let cfg = cfg_inner.clone();
                async move {
                    function_info(&iii, &cfg, req)
                        .await
                        .map_err(IIIError::Handler)
                }
            },
        )
        .description(
            "Full detail for one function: schemas, owning worker, registered \
             triggers that target it, and any matching how-to skill from skills_folder.",
        ),
    );
}

fn register_trigger_list(iii: &Arc<III>) {
    let iii_inner = iii.clone();
    iii.register_function(
        RegisterFunction::new_async(
            "directory::engine::triggers::list",
            move |req: TriggerListInput| {
                let iii = iii_inner.clone();
                async move { trigger_list(&iii, req).await.map_err(IIIError::Handler) }
            },
        )
        .description(
            "List every trigger TYPE registered with the engine. Filter by \
             search, prefix, worker. (For registered trigger instances, use \
             directory::engine::registered-triggers::list.)",
        ),
    );
}

fn register_trigger_info(iii: &Arc<III>) {
    let iii_inner = iii.clone();
    iii.register_function(
        RegisterFunction::new_async(
            "directory::engine::triggers::info",
            move |req: TriggerInfoInput| {
                let iii = iii_inner.clone();
                async move { trigger_info(&iii, req).await.map_err(IIIError::Handler) }
            },
        )
        .description(
            "Full detail for one trigger type: configuration schema, return \
             schema, owning worker, and current instance count.",
        ),
    );
}

fn register_registered_trigger_list(iii: &Arc<III>) {
    let iii_inner = iii.clone();
    iii.register_function(
        RegisterFunction::new_async(
            "directory::engine::registered-triggers::list",
            move |req: RegisteredTriggerListInput| {
                let iii = iii_inner.clone();
                async move {
                    registered_trigger_list(&iii, req)
                        .await
                        .map_err(IIIError::Handler)
                }
            },
        )
        .description(
            "List registered trigger instances (the link rows between \
             trigger types and target functions). Filter by trigger_type, \
             function_id, worker, or free-text search.",
        ),
    );
}

fn register_registered_trigger_info(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async(
            "directory::engine::registered-triggers::info",
            move |req: RegisteredTriggerInfoInput| {
                let iii = iii_inner.clone();
                let cfg = cfg_inner.clone();
                async move {
                    registered_trigger_info(&iii, &cfg, req)
                        .await
                        .map_err(IIIError::Handler)
                }
            },
        )
        .description(
            "Full denormalized detail for one registered trigger: \
             instance config + trigger-type detail + function detail.",
        ),
    );
}

fn register_worker_list(iii: &Arc<III>) {
    let iii_inner = iii.clone();
    iii.register_function(
        RegisterFunction::new_async(
            "directory::engine::workers::list",
            move |req: WorkerListInput| {
                let iii = iii_inner.clone();
                async move { worker_list(&iii, req).await.map_err(IIIError::Handler) }
            },
        )
        .description(
            "List every worker currently connected to the engine. Filter by \
             name substring, runtime, or status. Same row shape as \
             directory::registry::workers::list so callers learn one envelope.",
        ),
    );
}

fn register_worker_info(iii: &Arc<III>) {
    let iii_inner = iii.clone();
    iii.register_function(
        RegisterFunction::new_async(
            "directory::engine::workers::info",
            move |req: WorkerInfoInput| {
                let iii = iii_inner.clone();
                async move { worker_info(&iii, req).await.map_err(IIIError::Handler) }
            },
        )
        .description(
            "Worker envelope plus the lists of functions, trigger types, and \
             registered triggers it owns. The `worker` field has the same \
             shape as directory::registry::workers::info so callers can \
             switch between local + registry surfaces with the same parser.",
        ),
    );
}

// ---------- core handlers ----------

pub async fn function_list(
    iii: &III,
    input: FunctionListInput,
) -> Result<FunctionListOutput, String> {
    let (functions, workers) = fetch_functions_and_workers(iii).await?;
    let owner_map = build_function_owner_map(&workers);

    let search = input.search.as_deref().map(str::to_lowercase);
    let prefix = input.prefix.as_deref();
    let worker = input.worker.as_deref();

    let mut entries: Vec<FunctionListEntry> = functions
        .into_iter()
        .filter_map(|f| {
            let worker_name = owner_map
                .get(&f.function_id)
                .cloned()
                .or_else(|| id_worker_namespace(&f.function_id));
            if let Some(needle) = &search {
                let hay_id = f.function_id.to_lowercase();
                let hay_desc = f.description.as_deref().unwrap_or_default().to_lowercase();
                if !hay_id.contains(needle) && !hay_desc.contains(needle) {
                    return None;
                }
            }
            if let Some(p) = prefix {
                if !f.function_id.starts_with(p) {
                    return None;
                }
            }
            if let Some(w) = worker {
                if worker_name.as_deref() != Some(w) {
                    return None;
                }
            }
            Some(FunctionListEntry {
                function_id: f.function_id,
                worker_name,
                description: f.description,
            })
        })
        .collect();
    entries.sort_by(|a, b| a.function_id.cmp(&b.function_id));
    Ok(FunctionListOutput { functions: entries })
}

pub async fn function_info(
    iii: &III,
    cfg: &SkillsConfig,
    input: FunctionInfoInput,
) -> Result<FunctionInfoOutput, String> {
    let function_id = input.function_id.trim().to_string();
    if function_id.is_empty() {
        return Err("function_id must be non-empty".into());
    }
    let (functions, workers) = fetch_functions_and_workers(iii).await?;
    let triggers = engine_list_triggers(iii, true)
        .await
        .map_err(|e| format!("engine::triggers::list: {e}"))?;
    function_info_core(&functions, &workers, &triggers, cfg, &function_id)
}

pub async fn trigger_list(iii: &III, input: TriggerListInput) -> Result<TriggerListOutput, String> {
    let trigger_types = engine_list_trigger_types(iii, true)
        .await
        .map_err(|e| format!("engine::trigger-types::list: {e}"))?;

    let search = input.search.as_deref().map(str::to_lowercase);
    let prefix = input.prefix.as_deref();
    let worker = input.worker.as_deref();

    let mut entries: Vec<TriggerListEntry> = trigger_types
        .into_iter()
        .filter_map(|t| {
            if let Some(needle) = &search {
                let hay = format!("{} {}", t.id, t.description).to_lowercase();
                if !hay.contains(needle) {
                    return None;
                }
            }
            if let Some(p) = prefix {
                if !t.id.starts_with(p) {
                    return None;
                }
            }
            let worker_name = id_worker_namespace(&t.id);
            if let Some(w) = worker {
                if worker_name.as_deref() != Some(w) {
                    return None;
                }
            }
            Some(TriggerListEntry {
                id: t.id,
                worker_name,
                description: t.description,
            })
        })
        .collect();
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(TriggerListOutput { triggers: entries })
}

pub async fn trigger_info(iii: &III, input: TriggerInfoInput) -> Result<TriggerInfoOutput, String> {
    let id = input.id.trim().to_string();
    if id.is_empty() {
        return Err("id must be non-empty".into());
    }
    let trigger_types = engine_list_trigger_types(iii, true)
        .await
        .map_err(|e| format!("engine::trigger-types::list: {e}"))?;
    let triggers = engine_list_triggers(iii, true)
        .await
        .map_err(|e| format!("engine::triggers::list: {e}"))?;
    trigger_info_core(&trigger_types, &triggers, &id)
}

pub async fn registered_trigger_list(
    iii: &III,
    input: RegisteredTriggerListInput,
) -> Result<RegisteredTriggerListOutput, String> {
    let triggers = engine_list_triggers(iii, true)
        .await
        .map_err(|e| format!("engine::triggers::list: {e}"))?;
    let workers = engine_list_workers(iii)
        .await
        .map_err(|e| format!("engine::workers::list: {e}"))?;
    let owner_map = build_function_owner_map(&workers);

    let search = input.search.as_deref().map(str::to_lowercase);
    let trigger_type_filter = input.trigger_type.as_deref();
    let function_id_filter = input.function_id.as_deref();
    let worker_filter = input.worker.as_deref();

    let mut entries: Vec<RegisteredTriggerListEntry> = triggers
        .into_iter()
        .filter_map(|t| {
            let worker_name = owner_map
                .get(&t.function_id)
                .cloned()
                .or_else(|| id_worker_namespace(&t.function_id));
            if let Some(tt) = trigger_type_filter {
                if t.trigger_type != tt {
                    return None;
                }
            }
            if let Some(fid) = function_id_filter {
                if t.function_id != fid {
                    return None;
                }
            }
            if let Some(w) = worker_filter {
                if worker_name.as_deref() != Some(w) {
                    return None;
                }
            }
            if let Some(needle) = &search {
                let hay = format!("{} {} {}", t.id, t.trigger_type, t.function_id).to_lowercase();
                if !hay.contains(needle) {
                    return None;
                }
            }
            let config_summary = summarize_config(&t.config);
            Some(RegisteredTriggerListEntry {
                id: t.id,
                trigger_type: t.trigger_type,
                function_id: t.function_id,
                worker_name,
                config_summary,
            })
        })
        .collect();
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(RegisteredTriggerListOutput {
        registered_triggers: entries,
    })
}

pub async fn registered_trigger_info(
    iii: &III,
    cfg: &SkillsConfig,
    input: RegisteredTriggerInfoInput,
) -> Result<RegisteredTriggerInfoOutput, String> {
    let id = input.id.trim().to_string();
    if id.is_empty() {
        return Err("id must be non-empty".into());
    }
    let triggers = engine_list_triggers(iii, true)
        .await
        .map_err(|e| format!("engine::triggers::list: {e}"))?;
    let trigger_types = engine_list_trigger_types(iii, true)
        .await
        .map_err(|e| format!("engine::trigger-types::list: {e}"))?;
    let (functions, workers) = fetch_functions_and_workers(iii).await?;
    let owner_map = build_function_owner_map(&workers);

    let trigger = triggers
        .iter()
        .find(|t| t.id == id)
        .cloned()
        .ok_or_else(|| format!("registered trigger not found: {id}"))?;

    let worker_name = owner_map
        .get(&trigger.function_id)
        .cloned()
        .or_else(|| id_worker_namespace(&trigger.function_id));

    let trigger_detail = trigger_info_core(&trigger_types, &triggers, &trigger.trigger_type).ok();
    let function_detail =
        function_info_core(&functions, &workers, &triggers, cfg, &trigger.function_id).ok();

    Ok(RegisteredTriggerInfoOutput {
        id: trigger.id,
        trigger_type: trigger.trigger_type,
        function_id: trigger.function_id,
        worker_name,
        config: trigger.config,
        metadata: trigger.metadata,
        trigger: trigger_detail,
        function: function_detail,
    })
}

pub async fn worker_list(iii: &III, input: WorkerListInput) -> Result<WorkerListOutput, String> {
    let workers = engine_list_workers(iii)
        .await
        .map_err(|e| format!("engine::workers::list: {e}"))?;

    let search = input.search.as_deref().map(str::to_lowercase);
    let runtime = input.runtime.as_deref();
    let status = input.status.as_deref();

    let mut entries: Vec<Worker> = workers
        .into_iter()
        .filter(|w| {
            if let Some(needle) = &search {
                let hay = w.name.as_deref().unwrap_or("").to_lowercase();
                if !hay.contains(needle) {
                    return false;
                }
            }
            if let Some(r) = runtime {
                if w.runtime.as_deref() != Some(r) {
                    return false;
                }
            }
            if let Some(s) = status {
                if w.status != s {
                    return false;
                }
            }
            true
        })
        .map(worker_envelope_from_sdk)
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(WorkerListOutput { workers: entries })
}

pub async fn worker_info(iii: &III, input: WorkerInfoInput) -> Result<WorkerInfoOutput, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("name must be non-empty".into());
    }

    let workers = engine_list_workers(iii)
        .await
        .map_err(|e| format!("engine::workers::list: {e}"))?;
    let worker = workers
        .iter()
        .find(|w| w.name.as_deref() == Some(name.as_str()))
        .cloned()
        .ok_or_else(|| format!("worker not found: {name}"))?;

    let functions = engine_list_functions(iii)
        .await
        .map_err(|e| format!("engine::functions::list: {e}"))?;
    let trigger_types = engine_list_trigger_types(iii, true)
        .await
        .map_err(|e| format!("engine::trigger-types::list: {e}"))?;
    let triggers = engine_list_triggers(iii, true)
        .await
        .map_err(|e| format!("engine::triggers::list: {e}"))?;

    let owned_fns: std::collections::HashSet<String> = worker.functions.iter().cloned().collect();
    let function_entries: Vec<WorkerFunctionEntry> = worker
        .functions
        .iter()
        .map(|fid| {
            let description = functions
                .iter()
                .find(|f| &f.function_id == fid)
                .and_then(|f| f.description.clone());
            WorkerFunctionEntry {
                function_id: fid.clone(),
                description,
            }
        })
        .collect();

    let prefix = format!("{name}::");
    let trigger_type_entries: Vec<WorkerTriggerTypeEntry> = trigger_types
        .into_iter()
        .filter(|t| {
            t.id.starts_with(&prefix) || id_worker_namespace(&t.id).as_deref() == Some(&name)
        })
        .map(|t| WorkerTriggerTypeEntry {
            id: t.id,
            description: t.description,
        })
        .collect();

    let registered_trigger_entries: Vec<WorkerRegisteredTriggerEntry> = triggers
        .into_iter()
        .filter(|t| owned_fns.contains(&t.function_id))
        .map(|t| WorkerRegisteredTriggerEntry {
            id: t.id,
            trigger_type: t.trigger_type,
            function_id: t.function_id,
        })
        .collect();

    Ok(WorkerInfoOutput {
        worker: worker_envelope_from_sdk(worker),
        functions: function_entries,
        trigger_types: trigger_type_entries,
        registered_triggers: registered_trigger_entries,
    })
}

// ---------- pure helpers (unit-testable without the engine) ----------

/// Project an SDK `WorkerInfo` into the directory `Worker` envelope.
/// `description` is always `None` since the engine carries no
/// description for connected workers — the field exists for shape
/// parity with `registry::Worker`.
pub(crate) fn worker_envelope_from_sdk(w: WorkerInfo) -> Worker {
    Worker {
        name: w.name,
        description: None,
        version: w.version,
        id: w.id,
        runtime: w.runtime,
        os: w.os,
        status: w.status,
        function_count: w.function_count,
        connected_at_ms: w.connected_at_ms,
        active_invocations: w.active_invocations,
        isolation: w.isolation,
        ip_address: w.ip_address,
    }
}

/// Build a `function_id → worker_name` map from `WorkerInfo.functions[]`.
/// This is the canonical attribution; the namespace-segment fallback is
/// used only for unknown ids.
pub(crate) fn build_function_owner_map(
    workers: &[WorkerInfo],
) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for w in workers {
        let Some(name) = &w.name else { continue };
        for fid in &w.functions {
            map.insert(fid.clone(), name.clone());
        }
    }
    map
}

/// First `::` segment, used as a fallback worker-name attribution for
/// trigger-type ids (no `WorkerInfo.trigger_types[]` field exists in
/// SDK 0.11.3).
pub fn id_worker_namespace(id: &str) -> Option<String> {
    match id.split_once("::") {
        Some((ns, _)) if !ns.is_empty() => Some(ns.to_string()),
        _ => None,
    }
}

/// Compact preview of a `config` JSON value so list rows stay scannable.
/// Single-line, char-truncated to 80 visible chars.
pub fn summarize_config(config: &Value) -> String {
    let raw = serde_json::to_string(config).unwrap_or_else(|_| "{}".to_string());
    let single_line: String = raw
        .chars()
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect();
    truncate_chars(&single_line, 80)
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        Some((byte_end, _)) => format!("{}...", &s[..byte_end]),
        None => s.to_string(),
    }
}

/// Internal: assemble `FunctionInfoOutput` from already-fetched lists.
/// The composite `registered-trigger-info` calls this so the bus isn't
/// hit twice for the same data.
pub(crate) fn function_info_core(
    functions: &[SdkFunctionInfo],
    workers: &[WorkerInfo],
    triggers: &[SdkTriggerInfo],
    cfg: &SkillsConfig,
    function_id: &str,
) -> Result<FunctionInfoOutput, String> {
    let f = functions
        .iter()
        .find(|f| f.function_id == function_id)
        .ok_or_else(|| format!("function not found: {function_id}"))?;
    let owner_map = build_function_owner_map(workers);
    let worker_name = owner_map
        .get(function_id)
        .cloned()
        .or_else(|| id_worker_namespace(function_id));

    let registered: Vec<RegisteredTriggerSummary> = triggers
        .iter()
        .filter(|t| t.function_id == function_id)
        .map(|t| RegisteredTriggerSummary {
            id: t.id.clone(),
            trigger_type: t.trigger_type.clone(),
            config: t.config.clone(),
        })
        .collect();

    let how_guide =
        how_to::find_for_function(&cfg.resolved_skills_folder(), function_id).map(|h| HowGuide {
            title: how_to::resolve_title(h.frontmatter.title.as_deref(), &h.body, &h.skill_id),
            skill_id: h.skill_id,
            body: h.body,
        });

    let related_skills = how_to::find_related_for_function(
        &cfg.resolved_skills_folder(),
        function_id,
        how_guide.as_ref().map(|h| h.skill_id.as_str()),
    );

    Ok(FunctionInfoOutput {
        function_id: f.function_id.clone(),
        worker_name,
        description: f.description.clone(),
        request_schema: f.request_format.clone(),
        response_schema: f.response_format.clone(),
        metadata: f.metadata.clone(),
        registered_triggers: registered,
        how_guide,
        related_skills,
    })
}

/// Internal: assemble `TriggerInfoOutput` from already-fetched lists.
pub(crate) fn trigger_info_core(
    trigger_types: &[TriggerTypeInfo],
    triggers: &[SdkTriggerInfo],
    id: &str,
) -> Result<TriggerInfoOutput, String> {
    let t = trigger_types
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| format!("trigger type not found: {id}"))?;
    let instance_count = triggers.iter().filter(|x| x.trigger_type == id).count();
    Ok(TriggerInfoOutput {
        id: t.id.clone(),
        worker_name: id_worker_namespace(&t.id),
        description: t.description.clone(),
        configuration_schema: t.trigger_request_format.clone(),
        return_schema: t.call_request_format.clone(),
        instance_count,
    })
}

async fn engine_list_functions(iii: &III) -> Result<Vec<SdkFunctionInfo>, IIIError> {
    let result = iii
        .trigger(TriggerRequest {
            function_id: "engine::functions::list".into(),
            payload: serde_json::json!({}),
            action: None,
            timeout_ms: None,
        })
        .await?;
    Ok(result
        .get("functions")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default())
}

async fn engine_list_workers(iii: &III) -> Result<Vec<WorkerInfo>, IIIError> {
    let result = iii
        .trigger(TriggerRequest {
            function_id: "engine::workers::list".into(),
            payload: serde_json::json!({}),
            action: None,
            timeout_ms: None,
        })
        .await?;
    Ok(result
        .get("workers")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default())
}

async fn engine_list_triggers(
    iii: &III,
    include_internal: bool,
) -> Result<Vec<SdkTriggerInfo>, IIIError> {
    let result = iii
        .trigger(TriggerRequest {
            function_id: "engine::triggers::list".into(),
            payload: serde_json::json!({ "include_internal": include_internal }),
            action: None,
            timeout_ms: None,
        })
        .await?;
    Ok(result
        .get("triggers")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default())
}

async fn engine_list_trigger_types(
    iii: &III,
    include_internal: bool,
) -> Result<Vec<TriggerTypeInfo>, IIIError> {
    let result = iii
        .trigger(TriggerRequest {
            function_id: "engine::trigger-types::list".into(),
            payload: serde_json::json!({ "include_internal": include_internal }),
            action: None,
            timeout_ms: None,
        })
        .await?;
    Ok(result
        .get("trigger_types")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default())
}

async fn fetch_functions_and_workers(
    iii: &III,
) -> Result<(Vec<SdkFunctionInfo>, Vec<WorkerInfo>), String> {
    let functions = engine_list_functions(iii)
        .await
        .map_err(|e| format!("engine::functions::list: {e}"))?;
    let workers = engine_list_workers(iii)
        .await
        .map_err(|e| format!("engine::workers::list: {e}"))?;
    Ok((functions, workers))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn worker(name: &str, functions: &[&str]) -> WorkerInfo {
        WorkerInfo {
            id: format!("w-{name}"),
            name: Some(name.to_string()),
            runtime: Some("rust".into()),
            version: Some("0.0.0".into()),
            os: Some("linux".into()),
            ip_address: None,
            status: "connected".into(),
            connected_at_ms: 0,
            function_count: functions.len(),
            functions: functions.iter().map(|s| s.to_string()).collect(),
            active_invocations: 0,
            isolation: None,
        }
    }

    fn function(function_id: &str, description: Option<&str>) -> SdkFunctionInfo {
        SdkFunctionInfo {
            function_id: function_id.into(),
            description: description.map(String::from),
            request_format: Some(json!({"type": "object"})),
            response_format: Some(json!({"type": "object"})),
            metadata: None,
        }
    }

    fn trigger_type(id: &str, description: &str) -> TriggerTypeInfo {
        TriggerTypeInfo {
            id: id.into(),
            description: description.into(),
            trigger_request_format: Some(json!({"type": "object"})),
            call_request_format: Some(json!({"type": "object"})),
        }
    }

    fn registered_trigger(id: &str, trigger_type: &str, function_id: &str) -> SdkTriggerInfo {
        SdkTriggerInfo {
            id: id.into(),
            trigger_type: trigger_type.into(),
            function_id: function_id.into(),
            config: json!({"interval_ms": 1000}),
            metadata: None,
        }
    }

    /// Build a `SkillsConfig` whose `skills_folder` points at the supplied
    /// (empty) tempdir so the how-to / related-skill scans don't pick up
    /// the real `iii-directory/skills/` tree when tests run with the
    /// crate's CWD.
    fn isolated_cfg(tmp: &std::path::Path) -> SkillsConfig {
        SkillsConfig {
            skills_folder: tmp.to_string_lossy().into_owned(),
            ..SkillsConfig::default()
        }
    }

    #[test]
    fn id_worker_namespace_picks_first_segment() {
        assert_eq!(id_worker_namespace("mem::observe"), Some("mem".to_string()));
        assert_eq!(id_worker_namespace("flat"), None);
    }

    #[test]
    fn build_owner_map_uses_worker_functions() {
        let workers = vec![
            worker("memory", &["mem::observe", "mem::recall"]),
            worker("router", &["router::send"]),
        ];
        let map = build_function_owner_map(&workers);
        assert_eq!(map.get("mem::observe"), Some(&"memory".to_string()));
        assert_eq!(map.get("router::send"), Some(&"router".to_string()));
        assert!(!map.contains_key("missing::fn"));
    }

    #[test]
    fn summarize_config_truncates_long_payloads() {
        let big = json!({ "k": "x".repeat(200) });
        let s = summarize_config(&big);
        assert!(s.ends_with("..."));
        assert!(s.chars().count() <= 80 + 3);
    }

    #[test]
    fn summarize_config_handles_empty_object() {
        assert_eq!(summarize_config(&json!({})), "{}");
    }

    #[test]
    fn function_info_core_includes_registered_triggers() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = isolated_cfg(tmp.path());
        let functions = vec![function("mem::observe", Some("Observe events."))];
        let workers = vec![worker("agentmemory", &["mem::observe"])];
        let triggers = vec![
            registered_trigger("trg-1", "mem::on-change", "mem::observe"),
            registered_trigger("trg-2", "other::tick", "other::fn"),
        ];
        let details =
            function_info_core(&functions, &workers, &triggers, &cfg, "mem::observe").unwrap();
        assert_eq!(details.function_id, "mem::observe");
        assert_eq!(details.worker_name.as_deref(), Some("agentmemory"));
        assert_eq!(details.registered_triggers.len(), 1);
        assert_eq!(details.registered_triggers[0].id, "trg-1");
        // No how-to fixtures so the guide stays None.
        assert!(details.how_guide.is_none());
        assert!(details.related_skills.is_empty());
    }

    #[test]
    fn function_info_core_falls_back_to_namespace_when_no_owner() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = isolated_cfg(tmp.path());
        let functions = vec![function("orphan::fn", None)];
        let workers: Vec<WorkerInfo> = vec![]; // worker disconnected
        let triggers: Vec<SdkTriggerInfo> = vec![];
        let details =
            function_info_core(&functions, &workers, &triggers, &cfg, "orphan::fn").unwrap();
        assert_eq!(details.worker_name.as_deref(), Some("orphan"));
    }

    #[test]
    fn function_info_core_errors_on_unknown_id() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = isolated_cfg(tmp.path());
        let err = function_info_core(&[], &[], &[], &cfg, "missing::fn").unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[test]
    fn trigger_info_core_counts_instances() {
        let trigger_types = vec![trigger_type("mem::on-change", "Fires on memory change.")];
        let triggers = vec![
            registered_trigger("t1", "mem::on-change", "subA"),
            registered_trigger("t2", "mem::on-change", "subB"),
            registered_trigger("t3", "other", "x"),
        ];
        let det = trigger_info_core(&trigger_types, &triggers, "mem::on-change").unwrap();
        assert_eq!(det.instance_count, 2);
        assert_eq!(det.worker_name.as_deref(), Some("mem"));
        assert_eq!(det.id, "mem::on-change");
        assert!(det.configuration_schema.is_some());
        assert!(det.return_schema.is_some());
    }

    #[test]
    fn trigger_info_core_errors_on_unknown() {
        let err = trigger_info_core(&[], &[], "missing").unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[test]
    fn worker_envelope_drops_description_and_keeps_runtime_metadata() {
        let w = worker("agentmemory", &["mem::observe"]);
        let env = worker_envelope_from_sdk(w);
        assert_eq!(env.name.as_deref(), Some("agentmemory"));
        assert!(
            env.description.is_none(),
            "directory carries no description"
        );
        assert_eq!(env.runtime.as_deref(), Some("rust"));
        assert_eq!(env.status, "connected");
        assert_eq!(env.function_count, 1);
    }
}
