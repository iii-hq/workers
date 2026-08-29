//! Typed public and internal functions for the A2UI worker.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::composer::{ComposeInput, Composer};
use crate::configuration::{ConfigCell, OnConfigChangeEvent, OnConfigChangeResponse, CONFIG_FN_ID};
use crate::hook::{StampSessionEvent, StampSessionResponse, STAMP_SESSION_ID};
use crate::protocol::{
    apply_messages, enforce_state_limits, export_surface, now_ms, push_history, set_data_path,
    snapshot, validate_identifier, validate_live_binding, validate_renderable, ActionRecord,
    DeleteSurface, DeleteSurfaceMessage, LiveBinding, ServerMessage, SessionState, SurfaceExport,
    SurfaceRecord, SurfaceRevision, SurfaceStatus, SurfaceSummary, SurfaceTemplate, CATALOG_ID,
    PAGE_HASH, PROTOCOL_VERSION,
};
use crate::store::Store;

pub const GENERATE_ID: &str = "a2ui::generate";
pub const APPLY_ID: &str = "a2ui::surface::apply";
pub const GET_ID: &str = "a2ui::surface::get";
pub const LIST_ID: &str = "a2ui::surface::list";
pub const DELETE_ID: &str = "a2ui::surface::delete";
pub const PATCH_ID: &str = "a2ui::surface::patch";
pub const EXPORT_ID: &str = "a2ui::surface::export";
pub const ACTION_ID: &str = "a2ui::action";
pub const HISTORY_ID: &str = "a2ui::surface::history";
pub const UNDO_ID: &str = "a2ui::surface::undo";
pub const DUPLICATE_ID: &str = "a2ui::surface::duplicate";
pub const PIN_ID: &str = "a2ui::surface::pin";
pub const IMPORT_ID: &str = "a2ui::surface::import";
pub const EXPORT_CODE_ID: &str = "a2ui::surface::export-code";
pub const BINDING_SET_ID: &str = "a2ui::binding::set";
pub const BINDING_DELETE_ID: &str = "a2ui::binding::delete";
pub const BINDING_APPLY_ID: &str = "a2ui::binding::apply";
pub const TEMPLATE_SAVE_ID: &str = "a2ui::template::save";
pub const TEMPLATE_LIST_ID: &str = "a2ui::template::list";
pub const TEMPLATE_GET_ID: &str = "a2ui::template::get";
pub const TEMPLATE_APPLY_ID: &str = "a2ui::template::apply";
pub const TEMPLATE_DELETE_ID: &str = "a2ui::template::delete";

pub const GENERATE_DESC: &str = "Generate a safe A2UI v0.9.1 Console surface from a short description and optional data; returns a compact receipt while the UI renders in chat and on the A2UI page.";
pub const APPLY_DESC: &str = "Validate and atomically apply a batch of A2UI v0.9.1 protocol messages to the current Harness session's durable surface state.";
pub const GET_DESC: &str = "Read one durable A2UI surface, including its flat component graph and data model, from the current Harness session.";
pub const LIST_DESC: &str = "List compact summaries of the durable A2UI surfaces owned by the current Harness session, newest first.";
pub const DELETE_DESC: &str = "Delete one A2UI surface from the current Harness session and publish the change to subscribed Console pages.";
pub const PATCH_DESC: &str = "Update an existing A2UI surface from a plain-language instruction while preserving unspecified content and rejecting stale revisions.";
pub const EXPORT_DESC: &str = "Export one A2UI surface as a portable, replayable JSON package without Harness session metadata.";
pub const ACTION_DESC: &str = "Console-only: persist an A2UI component action and forward it as a structured message to the originating Harness session.";
const HISTORY_DESC: &str = "List the bounded, restorable revision snapshots for one A2UI surface.";
const UNDO_DESC: &str =
    "Restore an A2UI surface from a prior revision while creating a new monotonic revision.";
const DUPLICATE_DESC: &str =
    "Duplicate an A2UI surface inside the current Harness session under a new stable id.";
const PIN_DESC: &str =
    "Pin or unpin an A2UI surface so important interfaces remain first in the library.";
const IMPORT_DESC: &str =
    "Import a portable A2UI surface package into the current Harness session.";
const EXPORT_CODE_DESC: &str =
    "Export a generated interface as a runnable React app or a data-serving iii worker template.";
const BINDING_SET_DESC: &str =
    "Attach an allowlisted live state, stream, or shell event binding to a surface.";
const BINDING_DELETE_DESC: &str = "Remove a declarative live event binding from an A2UI surface.";
const BINDING_APPLY_DESC: &str =
    "Console-only: persist a value delivered by a declared A2UI live binding.";
const TEMPLATE_SAVE_DESC: &str =
    "Save an A2UI surface as a reusable template in the current Harness session.";
const TEMPLATE_LIST_DESC: &str =
    "List reusable A2UI templates stored for the current Harness session.";
const TEMPLATE_GET_DESC: &str = "Read one reusable A2UI template from the current Harness session.";
const TEMPLATE_APPLY_DESC: &str =
    "Create a new A2UI surface from a reusable template in the current Harness session.";
const TEMPLATE_DELETE_DESC: &str =
    "Delete one reusable A2UI template from the current Harness session.";

#[derive(Clone)]
pub struct Deps {
    pub iii: Arc<IIIClient>,
    pub config: ConfigCell,
    pub store: Arc<Store>,
    pub composer: Arc<Composer>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GenerateRequest {
    /// Plain-language description of the interface to compose.
    pub description: String,
    /// Optional application data. The composer should bind components to this
    /// data instead of repeating it in the component graph.
    #[serde(default)]
    pub data: Option<Value>,
    /// Stable id to use for the generated surface. Omit for a generated id.
    #[serde(default)]
    pub surface_id: Option<String>,
    /// Human-readable title shown in the Console sidebar.
    #[serde(default)]
    pub title: Option<String>,
    /// Replace a surface with the same id. Defaults to true.
    #[serde(default = "default_true")]
    pub replace: bool,
    /// Harness-stamped context; accepted from trusted direct Console calls but
    /// intentionally absent from the model-facing request schema.
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(default)]
    #[schemars(skip)]
    pub model: Option<String>,
    #[serde(default)]
    #[schemars(skip)]
    pub provider: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyRequest {
    /// Ordered A2UI v0.9.1 envelopes. One batch targets one surface and is
    /// persisted atomically after the complete batch validates.
    pub messages: Vec<ServerMessage>,
    /// Optional Console title used when this batch creates a surface.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

/// Select one surface in the current Harness session.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetRequest {
    /// Stable surface id returned by generate, apply, or list.
    pub surface_id: String,
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

/// List the surfaces belonging to the current Harness session.
/// List generated surfaces in the authoritative Harness session.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListRequest {
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

/// Remove one surface from the current Harness session.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteRequest {
    /// Stable surface id returned by generate, apply, or list.
    pub surface_id: String,
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchRequest {
    /// Stable id of the surface to update.
    pub surface_id: String,
    /// Plain-language change to make while preserving unspecified content.
    pub instruction: String,
    /// Optional data to merge or use while composing the replacement.
    #[serde(default)]
    pub data: Option<Value>,
    /// Optional replacement title.
    #[serde(default)]
    pub title: Option<String>,
    /// Reject the update unless the surface is still at this revision.
    #[serde(default)]
    pub expected_revision: Option<u64>,
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(default)]
    #[schemars(skip)]
    pub model: Option<String>,
    #[serde(default)]
    #[schemars(skip)]
    pub provider: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportRequest {
    /// Stable id of the surface to export.
    pub surface_id: String,
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryRequest {
    /// Surface whose restorable snapshots should be listed.
    pub surface_id: String,
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UndoRequest {
    /// Surface to restore.
    pub surface_id: String,
    #[serde(default)]
    pub to_revision: Option<u64>,
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DuplicateRequest {
    /// Existing surface to copy.
    pub surface_id: String,
    pub new_surface_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PinRequest {
    /// Surface whose library priority should change.
    pub surface_id: String,
    pub pinned: bool,
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportRequest {
    /// Portable A2UI JSON package created by surface export.
    pub package: SurfaceExport,
    #[serde(default)]
    pub surface_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CodeTarget {
    React,
    Worker,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportCodeRequest {
    /// Surface to turn into source files.
    pub surface_id: String,
    pub target: CodeTarget,
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CodeFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CodeExport {
    pub format: String,
    pub surface_id: String,
    pub files: Vec<CodeFile>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingSetRequest {
    /// Surface that should receive live values.
    pub surface_id: String,
    pub binding: LiveBinding,
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingDeleteRequest {
    /// Surface that owns the binding.
    pub surface_id: String,
    pub binding_id: String,
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingApplyRequest {
    /// Surface receiving the event value.
    pub surface_id: String,
    pub binding_id: String,
    pub value: Value,
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplateSelectRequest {
    /// Reusable template identifier.
    pub template_id: String,
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplateSaveRequest {
    /// Surface to capture as a reusable template.
    pub surface_id: String,
    pub template_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplateApplyRequest {
    /// Template to instantiate.
    pub template_id: String,
    pub surface_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

/// List reusable templates in the authoritative Harness session.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplateListRequest {
    /// Harness session is injected by the trusted pre-trigger hook.
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TemplateListResponse {
    pub templates: Vec<SurfaceTemplate>,
    pub count: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionRequest {
    /// Originating Harness session. Supplied by the trusted Console client.
    #[serde(default)]
    #[schemars(skip)]
    pub session_id: Option<String>,
    pub surface_id: String,
    pub source_component_id: String,
    /// Stable client-generated id reused when Harness delivery is retried.
    pub action_id: String,
    pub name: String,
    #[serde(default)]
    pub context: Value,
    /// Full client-side model when createSurface.sendDataModel requested it.
    #[serde(default)]
    pub data_model: Option<Value>,
    /// Reject a gesture rendered from stale surface state.
    pub expected_revision: u64,
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    #[schemars(skip)]
    pub(crate) _caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SurfaceReceipt {
    pub session_id: String,
    pub surface_id: String,
    pub title: String,
    pub status: SurfaceStatus,
    pub protocol_version: String,
    pub catalog_id: String,
    pub revision: u64,
    pub component_count: usize,
    pub page: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListResponse {
    pub session_id: String,
    pub surfaces: Vec<SurfaceSummary>,
    pub count: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ActionResponse {
    pub accepted: bool,
    pub forwarded: bool,
    pub session_id: String,
    pub surface_id: String,
    pub revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forward_error: Option<String>,
}

pub async fn generate(deps: &Deps, req: GenerateRequest) -> Result<SurfaceReceipt, String> {
    let session_id = require_session(req.session_id)?;
    let description = req.description.trim();
    if description.is_empty() {
        return Err("description must not be empty".into());
    }
    let cfg = deps.config.read().await.clone();
    if description.len() > cfg.max_description_bytes {
        return Err(format!(
            "description is {} bytes; maximum is {}",
            description.len(),
            cfg.max_description_bytes
        ));
    }
    validate_data_size(req.data.as_ref(), cfg.max_data_bytes)?;
    let surface_id = req.surface_id.unwrap_or_else(generated_surface_id);
    let title = req.title.unwrap_or_else(|| title_from(description));
    let messages = deps
        .composer
        .compose(
            ComposeInput {
                session_id: &session_id,
                surface_id: &surface_id,
                description,
                data: req.data.as_ref(),
                existing_surface: None,
                inherited_model: req.model.as_deref(),
                inherited_provider: req.provider.as_deref(),
            },
            &cfg,
        )
        .await?;

    let _guard = deps.store.mutation_guard(&session_id).await;
    let mut state = deps.store.load(&session_id).await?;
    let replaced = state.get(&surface_id).cloned();
    if replaced.is_some() {
        if !req.replace {
            return Err(format!("surface `{surface_id}` already exists"));
        }
        state
            .surfaces
            .retain(|surface| surface.surface_id != surface_id);
    }
    apply_messages(&mut state, &messages, Some(&title), &cfg)?;
    if let Some(previous) = replaced {
        let entry = snapshot(&previous, "regenerate");
        let surface = state.get_mut(&surface_id).expect("replacement retained");
        surface.created_at_ms = previous.created_at_ms;
        surface.revision = previous.revision + 1;
        surface.pinned = previous.pinned;
        surface.bindings = previous.bindings;
        surface.history = previous.history;
        push_history(surface, entry, cfg.max_history_per_surface);
    }
    let surface = state
        .get(&surface_id)
        .ok_or_else(|| "generated surface was not retained".to_string())?;
    validate_renderable(surface)?;
    let receipt = receipt(surface, SurfaceStatus::Active);
    save_state(deps, &mut state, &cfg).await?;
    Ok(receipt)
}

pub async fn apply(deps: &Deps, req: ApplyRequest) -> Result<SurfaceReceipt, String> {
    let session_id = require_session(req.session_id)?;
    let cfg = deps.config.read().await.clone();
    let _guard = deps.store.mutation_guard(&session_id).await;
    let mut state = deps.store.load(&session_id).await?;
    let before = req
        .messages
        .first()
        .and_then(|message| state.get(message.surface_id()))
        .cloned();
    let outcome = apply_messages(&mut state, &req.messages, req.title.as_deref(), &cfg)?;
    if let (Some(previous), SurfaceStatus::Active) = (before, &outcome.status) {
        if let Some(surface) = state.get_mut(&outcome.surface_id) {
            let entry = snapshot(&previous, "protocol apply");
            push_history(surface, entry, cfg.max_history_per_surface);
        }
    }
    let result = match outcome.status {
        SurfaceStatus::Active => {
            let surface = state
                .get(&outcome.surface_id)
                .ok_or_else(|| "applied surface was not retained".to_string())?;
            receipt(surface, SurfaceStatus::Active)
        }
        SurfaceStatus::Deleted => SurfaceReceipt {
            session_id: session_id.clone(),
            surface_id: outcome.surface_id,
            title: "deleted surface".into(),
            status: SurfaceStatus::Deleted,
            protocol_version: PROTOCOL_VERSION.into(),
            catalog_id: CATALOG_ID.into(),
            revision: outcome.revision,
            component_count: 0,
            page: PAGE_HASH.into(),
        },
    };
    save_state(deps, &mut state, &cfg).await?;
    Ok(result)
}

pub async fn get(deps: &Deps, req: GetRequest) -> Result<SurfaceRecord, String> {
    let session_id = require_session(req.session_id)?;
    deps.store
        .load(&session_id)
        .await?
        .get(&req.surface_id)
        .cloned()
        .ok_or_else(|| format!("surface `{}` was not found", req.surface_id))
}

pub async fn list(deps: &Deps, req: ListRequest) -> Result<ListResponse, String> {
    let session_id = require_session(req.session_id)?;
    let mut surfaces: Vec<SurfaceSummary> = deps
        .store
        .load(&session_id)
        .await?
        .surfaces
        .iter()
        .map(SurfaceSummary::from)
        .collect();
    surfaces.sort_by(|left, right| {
        right.pinned.cmp(&left.pinned).then_with(|| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| left.surface_id.cmp(&right.surface_id))
        })
    });
    Ok(ListResponse {
        session_id,
        count: surfaces.len(),
        surfaces,
    })
}

pub async fn delete(deps: &Deps, req: DeleteRequest) -> Result<SurfaceReceipt, String> {
    let session_id = require_session(req.session_id)?;
    apply(
        deps,
        ApplyRequest {
            messages: vec![ServerMessage::DeleteSurface(DeleteSurfaceMessage {
                version: PROTOCOL_VERSION.into(),
                delete_surface: DeleteSurface {
                    surface_id: req.surface_id,
                },
            })],
            title: None,
            session_id: Some(session_id),
            _caller_worker_id: None,
        },
    )
    .await
}

pub async fn patch(deps: &Deps, req: PatchRequest) -> Result<SurfaceReceipt, String> {
    let session_id = require_session(req.session_id)?;
    let instruction = req.instruction.trim();
    if instruction.is_empty() {
        return Err("instruction must not be empty".into());
    }
    let cfg = deps.config.read().await.clone();
    if instruction.len() > cfg.max_description_bytes {
        return Err(format!(
            "instruction is {} bytes; maximum is {}",
            instruction.len(),
            cfg.max_description_bytes
        ));
    }
    validate_data_size(req.data.as_ref(), cfg.max_data_bytes)?;
    let original = deps
        .store
        .load(&session_id)
        .await?
        .get(&req.surface_id)
        .cloned()
        .ok_or_else(|| format!("surface `{}` was not found", req.surface_id))?;
    if let Some(expected) = req.expected_revision {
        if original.revision != expected {
            return Err(format!(
                "surface `{}` is at revision {}, not expected revision {expected}",
                req.surface_id, original.revision
            ));
        }
    }
    let messages = deps
        .composer
        .compose(
            ComposeInput {
                session_id: &session_id,
                surface_id: &req.surface_id,
                description: instruction,
                data: req.data.as_ref(),
                existing_surface: Some(&original),
                inherited_model: req.model.as_deref(),
                inherited_provider: req.provider.as_deref(),
            },
            &cfg,
        )
        .await?;

    let _guard = deps.store.mutation_guard(&session_id).await;
    let mut state = deps.store.load(&session_id).await?;
    let current = state
        .get(&req.surface_id)
        .cloned()
        .ok_or_else(|| format!("surface `{}` was deleted while composing", req.surface_id))?;
    if current.revision != original.revision {
        return Err(format!(
            "surface `{}` changed from revision {} to {} while composing; retry the patch",
            req.surface_id, original.revision, current.revision
        ));
    }
    state
        .surfaces
        .retain(|surface| surface.surface_id != req.surface_id);
    let title = req.title.as_deref().unwrap_or(&current.title);
    apply_messages(&mut state, &messages, Some(title), &cfg)?;
    let surface = state
        .get_mut(&req.surface_id)
        .ok_or_else(|| "patched surface was not retained".to_string())?;
    let entry = snapshot(&current, "generative patch");
    surface.created_at_ms = current.created_at_ms;
    surface.updated_at_ms = now_ms();
    surface.revision = current.revision + 1;
    surface.last_action = current.last_action;
    surface.pinned = current.pinned;
    surface.bindings = current.bindings;
    surface.history = current.history.clone();
    push_history(surface, entry, cfg.max_history_per_surface);
    let result = receipt(surface, SurfaceStatus::Active);
    state.updated_at_ms = surface.updated_at_ms;
    save_state(deps, &mut state, &cfg).await?;
    Ok(result)
}

pub async fn export(deps: &Deps, req: ExportRequest) -> Result<SurfaceExport, String> {
    let session_id = require_session(req.session_id)?;
    let state = deps.store.load(&session_id).await?;
    let surface = state
        .get(&req.surface_id)
        .ok_or_else(|| format!("surface `{}` was not found", req.surface_id))?;
    validate_renderable(surface)?;
    Ok(export_surface(surface))
}

pub async fn action(deps: &Deps, req: ActionRequest) -> Result<ActionResponse, String> {
    let session_id = require_session(req.session_id.clone())?;
    validate_identifier("action id", &req.action_id)?;
    if req.name.trim().is_empty() || req.source_component_id.trim().is_empty() {
        return Err("action name and source_component_id must not be empty".into());
    }
    let cfg = deps.config.read().await.clone();
    validate_data_size(req.data_model.as_ref(), cfg.max_data_bytes)?;
    validate_data_size(Some(&req.context), cfg.max_data_bytes)?;
    let (revision, timestamp_ms, context, data_model) = {
        let _guard = deps.store.mutation_guard(&session_id).await;
        let mut state = deps.store.load(&session_id).await?;
        let surface = state
            .get_mut(&req.surface_id)
            .ok_or_else(|| format!("surface `{}` was not found", req.surface_id))?;
        if let Some(existing) = find_action(surface, &req.action_id) {
            validate_action_retry(existing, &req)?;
            (
                surface.revision,
                existing.timestamp_ms,
                existing.context.clone(),
                existing.data_model.clone(),
            )
        } else {
            if surface.revision != req.expected_revision {
                return Err(format!(
                    "surface `{}` is at revision {}, not expected revision {}",
                    req.surface_id, surface.revision, req.expected_revision
                ));
            }
            validate_action_source(surface, &req.source_component_id, &req.name)?;
            let timestamp_ms = now_ms();
            let entry = snapshot(surface, format!("action {}", req.name));
            if let Some(data_model) = &req.data_model {
                surface.data_model = data_model.clone();
            }
            surface.last_action = Some(ActionRecord {
                action_id: req.action_id.clone(),
                name: req.name.clone(),
                source_component_id: req.source_component_id.clone(),
                context: req.context.clone(),
                data_model: req.data_model.clone(),
                timestamp_ms,
            });
            surface.revision += 1;
            surface.updated_at_ms = timestamp_ms;
            push_history(surface, entry, cfg.max_history_per_surface);
            let revision = surface.revision;
            state.updated_at_ms = timestamp_ms;
            save_state(deps, &mut state, &cfg).await?;
            (
                revision,
                timestamp_ms,
                req.context.clone(),
                req.data_model.clone(),
            )
        }
    };

    let (forwarded, turn_id, forward_error) = if cfg.forward_actions {
        let message = json!({
            "role": "custom",
            "custom_type": "a2ui.action",
            "content": [{
                "type": "text",
                "text": format!(
                    "The user activated `{}` on A2UI surface `{}`.",
                    req.name, req.surface_id
                )
            }],
            "display": format!("A2UI action: {}", req.name),
            "details": {
                "surface_id": req.surface_id,
                "source_component_id": req.source_component_id,
                "action_id": req.action_id,
                "action": req.name,
                "context": context,
                "data_model": data_model,
            },
            "timestamp": timestamp_ms,
        });
        match deps
            .iii
            .trigger(TriggerRequest {
                function_id: "harness::send".into(),
                payload: json!({
                    "session_id": session_id,
                    "message": message,
                    "idempotency_key": format!("a2ui-action-{}", req.action_id),
                }),
                action: None,
                timeout_ms: Some(30_000),
            })
            .await
        {
            Ok(response) => (
                true,
                response
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                None,
            ),
            Err(error) => (false, None, Some(error.to_string())),
        }
    } else {
        (false, None, None)
    };

    Ok(ActionResponse {
        accepted: true,
        forwarded,
        session_id,
        surface_id: req.surface_id,
        revision,
        turn_id,
        forward_error,
    })
}

pub async fn history(deps: &Deps, req: HistoryRequest) -> Result<Vec<SurfaceRevision>, String> {
    Ok(get(
        deps,
        GetRequest {
            surface_id: req.surface_id,
            session_id: req.session_id,
            _caller_worker_id: None,
        },
    )
    .await?
    .history)
}

pub async fn undo(deps: &Deps, req: UndoRequest) -> Result<SurfaceReceipt, String> {
    let session_id = require_session(req.session_id)?;
    let cfg = deps.config.read().await.clone();
    let _guard = deps.store.mutation_guard(&session_id).await;
    let mut state = deps.store.load(&session_id).await?;
    let surface = state
        .get_mut(&req.surface_id)
        .ok_or_else(|| format!("surface `{}` was not found", req.surface_id))?;
    let index = req
        .to_revision
        .map_or_else(
            || surface.history.len().checked_sub(1),
            |revision| {
                surface
                    .history
                    .iter()
                    .position(|item| item.revision == revision)
            },
        )
        .ok_or_else(|| "requested revision is not available".to_string())?;
    let target = surface.history[index].clone();
    let current = snapshot(surface, "before undo");
    surface.title = target.title;
    surface.theme = target.theme;
    surface.send_data_model = target.send_data_model;
    surface.components = target.components;
    surface.data_model = target.data_model;
    surface.last_action = target.last_action;
    surface.revision += 1;
    surface.updated_at_ms = now_ms();
    push_history(surface, current, cfg.max_history_per_surface);
    let result = receipt(surface, SurfaceStatus::Active);
    state.updated_at_ms = surface.updated_at_ms;
    save_state(deps, &mut state, &cfg).await?;
    Ok(result)
}

pub async fn duplicate(deps: &Deps, req: DuplicateRequest) -> Result<SurfaceReceipt, String> {
    let session_id = require_session(req.session_id)?;
    validate_identifier("new_surface_id", &req.new_surface_id)?;
    let cfg = deps.config.read().await.clone();
    let _guard = deps.store.mutation_guard(&session_id).await;
    let mut state = deps.store.load(&session_id).await?;
    if state.get(&req.new_surface_id).is_some() {
        return Err(format!("surface `{}` already exists", req.new_surface_id));
    }
    if state.surfaces.len() >= cfg.max_surfaces_per_session {
        return Err("session has reached its surface limit".into());
    }
    let mut copy = state
        .get(&req.surface_id)
        .cloned()
        .ok_or_else(|| format!("surface `{}` was not found", req.surface_id))?;
    let now = now_ms();
    copy.surface_id = req.new_surface_id;
    copy.title = req.title.unwrap_or_else(|| format!("{} copy", copy.title));
    copy.revision = 1;
    copy.created_at_ms = now;
    copy.updated_at_ms = now;
    copy.last_action = None;
    copy.pinned = false;
    copy.history.clear();
    let result = receipt(&copy, SurfaceStatus::Active);
    state.surfaces.push(copy);
    state.updated_at_ms = now;
    save_state(deps, &mut state, &cfg).await?;
    Ok(result)
}

pub async fn pin(deps: &Deps, req: PinRequest) -> Result<SurfaceReceipt, String> {
    let session_id = require_session(req.session_id)?;
    let cfg = deps.config.read().await.clone();
    let _guard = deps.store.mutation_guard(&session_id).await;
    let mut state = deps.store.load(&session_id).await?;
    let surface = state
        .get_mut(&req.surface_id)
        .ok_or_else(|| format!("surface `{}` was not found", req.surface_id))?;
    if surface.pinned != req.pinned {
        let entry = snapshot(surface, "pin changed");
        surface.pinned = req.pinned;
        surface.revision += 1;
        surface.updated_at_ms = now_ms();
        push_history(surface, entry, cfg.max_history_per_surface);
    }
    let result = receipt(surface, SurfaceStatus::Active);
    state.updated_at_ms = surface.updated_at_ms;
    save_state(deps, &mut state, &cfg).await?;
    Ok(result)
}

fn rewrite_messages(messages: &mut [ServerMessage], id: &str) {
    for message in messages {
        match message {
            ServerMessage::CreateSurface(v) => v.create_surface.surface_id = id.into(),
            ServerMessage::UpdateComponents(v) => v.update_components.surface_id = id.into(),
            ServerMessage::UpdateDataModel(v) => v.update_data_model.surface_id = id.into(),
            ServerMessage::DeleteSurface(v) => v.delete_surface.surface_id = id.into(),
        }
    }
}

pub async fn import_surface(deps: &Deps, req: ImportRequest) -> Result<SurfaceReceipt, String> {
    if req.package.export_format != "a2ui.surface" || req.package.format_version != 1 {
        return Err("unsupported A2UI export package".into());
    }
    if req.package.protocol_version != PROTOCOL_VERSION || req.package.catalog_id != CATALOG_ID {
        return Err(format!(
            "import requires protocol `{PROTOCOL_VERSION}` and catalog `{CATALOG_ID}`"
        ));
    }
    let id = req.surface_id.unwrap_or(req.package.surface_id);
    let mut messages = req.package.messages;
    rewrite_messages(&mut messages, &id);
    apply(
        deps,
        ApplyRequest {
            messages,
            title: req.title.or(Some(req.package.title)),
            session_id: req.session_id,
            _caller_worker_id: None,
        },
    )
    .await
}

pub async fn set_binding(deps: &Deps, req: BindingSetRequest) -> Result<SurfaceReceipt, String> {
    validate_live_binding(&req.binding)?;
    mutate_surface(
        deps,
        req.session_id,
        &req.surface_id,
        "binding changed",
        true,
        |surface| {
            if let Some(item) = surface
                .bindings
                .iter_mut()
                .find(|item| item.id == req.binding.id)
            {
                *item = req.binding;
            } else {
                surface.bindings.push(req.binding);
            }
            Ok(())
        },
    )
    .await
}
pub async fn delete_binding(
    deps: &Deps,
    req: BindingDeleteRequest,
) -> Result<SurfaceReceipt, String> {
    mutate_surface(
        deps,
        req.session_id,
        &req.surface_id,
        "binding removed",
        true,
        |surface| {
            let before = surface.bindings.len();
            surface.bindings.retain(|item| item.id != req.binding_id);
            if before == surface.bindings.len() {
                return Err("binding was not found".into());
            }
            Ok(())
        },
    )
    .await
}
pub async fn apply_binding(
    deps: &Deps,
    req: BindingApplyRequest,
) -> Result<SurfaceReceipt, String> {
    mutate_surface(
        deps,
        req.session_id,
        &req.surface_id,
        "live binding update",
        false,
        |surface| {
            let binding = surface
                .bindings
                .iter()
                .find(|item| item.id == req.binding_id)
                .ok_or_else(|| "binding was not found".to_string())?;
            let path = binding.target_path.clone();
            set_data_path(&mut surface.data_model, &path, req.value)
        },
    )
    .await
}

async fn mutate_surface<F>(
    deps: &Deps,
    session: Option<String>,
    id: &str,
    reason: &str,
    record_history: bool,
    change: F,
) -> Result<SurfaceReceipt, String>
where
    F: FnOnce(&mut SurfaceRecord) -> Result<(), String>,
{
    let session_id = require_session(session)?;
    let cfg = deps.config.read().await.clone();
    let _guard = deps.store.mutation_guard(&session_id).await;
    let mut state = deps.store.load(&session_id).await?;
    let surface = state
        .get_mut(id)
        .ok_or_else(|| format!("surface `{id}` was not found"))?;
    let entry = record_history.then(|| snapshot(surface, reason));
    change(surface)?;
    surface.revision += 1;
    surface.updated_at_ms = now_ms();
    if let Some(entry) = entry {
        push_history(surface, entry, cfg.max_history_per_surface);
    }
    let result = receipt(surface, SurfaceStatus::Active);
    state.updated_at_ms = surface.updated_at_ms;
    save_state(deps, &mut state, &cfg).await?;
    Ok(result)
}

pub async fn save_template(
    deps: &Deps,
    req: TemplateSaveRequest,
) -> Result<SurfaceTemplate, String> {
    let session_id = require_session(req.session_id)?;
    validate_identifier("template_id", &req.template_id)?;
    let cfg = deps.config.read().await.clone();
    let _guard = deps.store.mutation_guard(&session_id).await;
    let mut state = deps.store.load(&session_id).await?;
    let surface = state
        .get(&req.surface_id)
        .cloned()
        .ok_or_else(|| "surface was not found".to_string())?;
    let now = now_ms();
    let created = state
        .templates
        .iter()
        .find(|v| v.template_id == req.template_id)
        .map_or(now, |v| v.created_at_ms);
    let template = SurfaceTemplate {
        template_id: req.template_id,
        title: req.title.unwrap_or(surface.title),
        description: req.description,
        protocol_version: surface.protocol_version,
        catalog_id: surface.catalog_id,
        theme: surface.theme,
        send_data_model: surface.send_data_model,
        components: surface.components,
        data_model: surface.data_model,
        created_at_ms: created,
        updated_at_ms: now,
    };
    state
        .templates
        .retain(|v| v.template_id != template.template_id);
    if state.templates.len() >= cfg.max_templates_per_session {
        return Err("session has reached its template limit".into());
    }
    state.templates.push(template.clone());
    state.updated_at_ms = now;
    save_state(deps, &mut state, &cfg).await?;
    Ok(template)
}
pub async fn list_templates(
    deps: &Deps,
    req: TemplateListRequest,
) -> Result<TemplateListResponse, String> {
    let session = require_session(req.session_id)?;
    let mut templates = deps.store.load(&session).await?.templates;
    templates.sort_by_key(|item| std::cmp::Reverse(item.updated_at_ms));
    Ok(TemplateListResponse {
        count: templates.len(),
        templates,
    })
}
pub async fn get_template(
    deps: &Deps,
    req: TemplateSelectRequest,
) -> Result<SurfaceTemplate, String> {
    let session = require_session(req.session_id)?;
    validate_identifier("template_id", &req.template_id)?;
    deps.store
        .load(&session)
        .await?
        .templates
        .into_iter()
        .find(|v| v.template_id == req.template_id)
        .ok_or_else(|| "template was not found".into())
}
pub async fn apply_template(
    deps: &Deps,
    req: TemplateApplyRequest,
) -> Result<SurfaceReceipt, String> {
    let session_id = require_session(req.session_id)?;
    validate_identifier("template_id", &req.template_id)?;
    validate_identifier("surface_id", &req.surface_id)?;
    let template = get_template(
        deps,
        TemplateSelectRequest {
            template_id: req.template_id,
            session_id: Some(session_id.clone()),
            _caller_worker_id: None,
        },
    )
    .await?;
    let package = SurfaceExport {
        export_format: "a2ui.surface".into(),
        format_version: 1,
        protocol_version: template.protocol_version,
        catalog_id: template.catalog_id,
        surface_id: req.surface_id.clone(),
        title: template.title,
        messages: vec![
            ServerMessage::CreateSurface(crate::protocol::CreateSurfaceMessage {
                version: PROTOCOL_VERSION.into(),
                create_surface: crate::protocol::CreateSurface {
                    surface_id: req.surface_id.clone(),
                    catalog_id: CATALOG_ID.into(),
                    theme: template.theme,
                    send_data_model: Some(template.send_data_model),
                },
            }),
            ServerMessage::UpdateComponents(crate::protocol::UpdateComponentsMessage {
                version: PROTOCOL_VERSION.into(),
                update_components: crate::protocol::UpdateComponents {
                    surface_id: req.surface_id.clone(),
                    components: template.components,
                },
            }),
            ServerMessage::UpdateDataModel(crate::protocol::UpdateDataModelMessage {
                version: PROTOCOL_VERSION.into(),
                update_data_model: crate::protocol::UpdateDataModel {
                    surface_id: req.surface_id.clone(),
                    path: Some("/".into()),
                    value: Some(template.data_model),
                },
            }),
        ],
    };
    import_surface(
        deps,
        ImportRequest {
            package,
            surface_id: Some(req.surface_id),
            title: req.title,
            session_id: Some(session_id),
            _caller_worker_id: None,
        },
    )
    .await
}
pub async fn delete_template(deps: &Deps, req: TemplateSelectRequest) -> Result<bool, String> {
    let session = require_session(req.session_id)?;
    validate_identifier("template_id", &req.template_id)?;
    let cfg = deps.config.read().await.clone();
    let _guard = deps.store.mutation_guard(&session).await;
    let mut state = deps.store.load(&session).await?;
    let before = state.templates.len();
    state.templates.retain(|v| v.template_id != req.template_id);
    if before == state.templates.len() {
        return Err("template was not found".into());
    }
    state.updated_at_ms = now_ms();
    save_state(deps, &mut state, &cfg).await?;
    Ok(true)
}

pub async fn export_code(deps: &Deps, req: ExportCodeRequest) -> Result<CodeExport, String> {
    let surface = get(
        deps,
        GetRequest {
            surface_id: req.surface_id,
            session_id: req.session_id,
            _caller_worker_id: None,
        },
    )
    .await?;
    let json =
        serde_json::to_string_pretty(&export_surface(&surface)).map_err(|e| e.to_string())?;
    let surface_file = |path: &str| CodeFile {
        path: path.into(),
        content: format!("{json}\n"),
    };
    let files = match req.target {
        CodeTarget::React => vec![
            CodeFile {
                path: "package.json".into(),
                content: react_package(),
            },
            CodeFile {
                path: "index.html".into(),
                content: react_index(),
            },
            CodeFile {
                path: "tsconfig.json".into(),
                content: react_tsconfig(),
            },
            CodeFile {
                path: "README.md".into(),
                content: react_readme(),
            },
            surface_file("src/surface.json"),
            CodeFile {
                path: "src/GeneratedSurface.tsx".into(),
                content: react_source(),
            },
            CodeFile {
                path: "src/main.tsx".into(),
                content: react_main(),
            },
            CodeFile {
                path: "src/styles.css".into(),
                content: react_styles(),
            },
            CodeFile {
                path: "src/vite-env.d.ts".into(),
                content: react_vite_env(),
            },
        ],
        CodeTarget::Worker => vec![
            CodeFile {
                path: "worker-compose.yaml".into(),
                content: worker_compose(),
            },
            CodeFile {
                path: "package.json".into(),
                content: worker_package(),
            },
            surface_file("surface.json"),
            CodeFile {
                path: "src/index.mjs".into(),
                content: worker_source(),
            },
        ],
    };
    Ok(CodeExport {
        format: match req.target {
            CodeTarget::React => "react",
            CodeTarget::Worker => "iii-worker",
        }
        .into(),
        surface_id: surface.surface_id,
        files,
    })
}
fn react_package() -> String {
    r#"{
  "name": "generated-a2ui-app",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc --noEmit && vite build",
    "preview": "vite preview"
  },
  "dependencies": {
    "react": "19.2.7",
    "react-dom": "19.2.7"
  },
  "devDependencies": {
    "@types/react": "19.2.17",
    "@types/react-dom": "19.2.3",
    "typescript": "6.0.3",
    "vite": "8.1.5"
  }
}
"#
    .into()
}
fn react_index() -> String {
    r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <link rel="icon" href="data:," />
    <title>Generated A2UI app</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
"#
    .into()
}
fn react_tsconfig() -> String {
    r#"{
  "compilerOptions": {
    "target": "ES2022",
    "useDefineForClassFields": true,
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "allowJs": false,
    "skipLibCheck": true,
    "esModuleInterop": true,
    "allowSyntheticDefaultImports": true,
    "strict": true,
    "forceConsistentCasingInFileNames": true,
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true,
    "jsx": "react-jsx"
  },
  "include": ["src"]
}
"#
    .into()
}
fn react_main() -> String {
    r#"import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import GeneratedSurface from './GeneratedSurface';
import './styles.css';

const root = document.getElementById('root');
if (!root) throw new Error('Missing #root element');

createRoot(root).render(
  <StrictMode>
    <GeneratedSurface onAction={(action) => console.info('A2UI action', action)} />
  </StrictMode>,
);
"#
    .into()
}
fn react_styles() -> String {
    r#":root {
  color-scheme: dark;
  font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  color: #f5f5f5;
  background: #090909;
  font-synthesis: none;
}

* { box-sizing: border-box; }

body {
  min-width: 320px;
  min-height: 100vh;
  margin: 0;
  background: #090909;
}

button, input { font: inherit; }

.a2ui-generated-surface {
  width: min(100% - 2rem, 760px);
  margin: 0 auto;
  padding: clamp(1rem, 4vw, 3rem) 0;
}

.a2ui-generated-surface h1,
.a2ui-generated-surface h2,
.a2ui-generated-surface p { margin: 0; }

.a2ui-generated-surface h1 { font-size: clamp(1.5rem, 5vw, 2.25rem); }
.a2ui-generated-surface h2 { font-size: clamp(1.125rem, 3vw, 1.5rem); }

.a2ui-card {
  padding: clamp(1rem, 3vw, 1.5rem);
  border: 1px solid #2d2d2d;
  border-radius: 12px;
  background: #141414;
  box-shadow: 0 18px 60px rgb(0 0 0 / 28%);
}

.a2ui-badge {
  display: inline-flex;
  width: fit-content;
  padding: 0.25rem 0.5rem;
  border: 1px solid #3a3a3a;
  border-radius: 999px;
  color: #d4d4d4;
  background: #1d1d1d;
  font-size: 0.75rem;
}

.a2ui-field {
  display: grid;
  gap: 0.375rem;
  color: #b4b4b4;
  font-size: 0.8125rem;
}

.a2ui-field input {
  width: 100%;
  min-height: 2.75rem;
  padding: 0.625rem 0.75rem;
  border: 1px solid #353535;
  border-radius: 8px;
  outline: none;
  color: #f5f5f5;
  background: #202020;
}

.a2ui-field input:focus { border-color: #8a8a8a; }

.a2ui-check {
  display: flex;
  min-height: 2.75rem;
  align-items: center;
  gap: 0.625rem;
  color: #d4d4d4;
  font-size: 0.875rem;
}

.a2ui-check input { width: 1rem; height: 1rem; accent-color: #f5f5f5; }

.a2ui-button {
  min-height: 2.75rem;
  padding: 0.625rem 0.875rem;
  border: 0;
  border-radius: 8px;
  color: #111;
  background: #f5f5f5;
  cursor: pointer;
}

.a2ui-button:hover { background: #d8d8d8; }
.a2ui-button:focus-visible { outline: 2px solid #fff; outline-offset: 2px; }
.a2ui-divider { width: 100%; border: 0; border-top: 1px solid #2d2d2d; }

@media (max-width: 560px) {
  .a2ui-generated-surface { width: min(100% - 1rem, 760px); padding-block: 0.5rem; }
  .a2ui-card { padding: 0.875rem; border-radius: 10px; }
  .a2ui-button { width: 100%; }
}
"#
    .into()
}
fn react_vite_env() -> String {
    "/// <reference types=\"vite/client\" />\n".into()
}
fn react_readme() -> String {
    r#"# Generated A2UI React app

This is editable source generated from an A2UI surface.

Run it from the Shell worker:

```bash
pnpm install
pnpm dev --host 127.0.0.1
```

Then open the printed local URL in the Browser worker. `src/GeneratedSurface.tsx` contains the complete renderer and interaction code; `src/surface.json` contains the portable A2UI data.
"#
    .into()
}
fn react_source() -> String {
    r#"import React, { useState } from 'react';
import surfacePackage from './surface.json';

type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };
type Action = { name: string; context: JsonValue; dataModel?: JsonValue };
type Props = { onAction?: (action: Action) => void };
const messages = surfacePackage.messages as any[];
const components = new Map((messages.find((m) => m.updateComponents)?.updateComponents.components ?? []).map((c: any) => [c.id, c]));
const initialModel = (messages.find((m) => m.updateDataModel)?.updateDataModel.value ?? {}) as JsonValue;
const sendDataModel = Boolean(messages.find((m) => m.createSurface)?.createSurface.sendDataModel);
const segments = (path: string) => path.split('/').slice(1).map((key) => key.replaceAll('~1', '/').replaceAll('~0', '~'));
const at = (model: JsonValue, path?: string): JsonValue | undefined => path?.startsWith('/') ? segments(path).reduce<JsonValue | undefined>((value, key) => {
  if (Array.isArray(value)) return value[Number(key)];
  return value && typeof value === 'object' && Object.hasOwn(value, key) ? value[key] : undefined;
}, model) : undefined;
const setAt = (current: JsonValue | undefined, path: string[], value: JsonValue): JsonValue => {
  if (path.length === 0) return value;
  const [key, ...rest] = path;
  if (Array.isArray(current)) {
    const next = [...current];
    const index = Number(key);
    if (!Number.isInteger(index) || index < 0) return current;
    next[index] = setAt(next[index], rest, value);
    return next;
  }
  const source = current && typeof current === 'object' ? current : {};
  return { ...source, [key]: setAt(Object.hasOwn(source, key) ? source[key] : undefined, rest, value) };
};
const resolve = (input: any, model: JsonValue): any => {
  if (Array.isArray(input)) return input.map((value) => resolve(value, model));
  if (input && typeof input === 'object') {
    if (typeof input.path === 'string' && input.path.startsWith('/')) return at(model, input.path) ?? null;
    return Object.fromEntries(Object.entries(input).map(([key, value]) => [key, resolve(value, model)]));
  }
  return input;
};

export default function GeneratedSurface({ onAction }: Props) {
  const [model, setModel] = useState<JsonValue>(initialModel);
  const render = (id: string, ancestry = new Set<string>()): React.ReactNode => {
    if (ancestry.has(id)) return null;
    const component: any = components.get(id);
    if (!component) return null;
    const next = new Set(ancestry).add(id);
    const childIds = [...(component.children ?? []), ...(component.child ? [component.child] : [])];
    const children = childIds.map((child: string) => <React.Fragment key={child}>{render(child, next)}</React.Fragment>);
    switch (component.component) {
      case 'Column': return <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>{children}</div>;
      case 'Row': return <div style={{ display: 'flex', gap: 12, flexWrap: component.wrap ? 'wrap' : undefined }}>{children}</div>;
      case 'Card': return <section className="a2ui-card">{children}</section>;
      case 'Text': { const text = String(resolve(component.text, model) ?? ''); return component.variant === 'h1' ? <h1>{text}</h1> : component.variant === 'h2' ? <h2>{text}</h2> : <p>{text}</p>; }
      case 'Badge': return <span className="a2ui-badge">{String(resolve(component.text, model) ?? '')}</span>;
      case 'Button': { const event = component.action?.event; return <button className="a2ui-button" type="button" disabled={!event?.name} onClick={() => event?.name && onAction?.({ name: event.name, context: resolve(event.context ?? {}, model), dataModel: sendDataModel ? model : undefined })}>{children}</button>; }
      case 'TextField': { const path = component.value?.path; return <label className="a2ui-field">{component.label}<input value={String(at(model, path) ?? '')} placeholder={component.placeholder} onChange={(event) => { const value = event.currentTarget.value; if (path?.startsWith('/')) setModel((current) => setAt(current, segments(path), value)); }} /></label>; }
      case 'CheckBox': { const path = component.value?.path; return <label className="a2ui-check"><input type="checkbox" checked={Boolean(at(model, path))} onChange={(event) => { const checked = event.currentTarget.checked; if (path?.startsWith('/')) setModel((current) => setAt(current, segments(path), checked)); }} />{component.label}</label>; }
      case 'Divider': return <hr className="a2ui-divider" />;
      default: return null;
    }
  };
  return <main className="a2ui-generated-surface" data-a2ui-surface={surfacePackage.surface_id}>{render('root')}</main>;
}
"#
    .into()
}
fn worker_compose() -> String {
    "workers:\n  generated-a2ui:\n    source:\n      path: .\n      package_manifest: package.json\n    artifact:\n      kind: javascript-bundle\n      build_command: [node, --check, src/index.mjs]\n      include: [package.json, surface.json, src/index.mjs]\n    runtime:\n      exec: [node, src/index.mjs]\n    registry:\n      description: Serve one exported A2UI surface as an iii function.\n      license: Apache-2.0\n      tags: [a2ui, generated-ui]\n      dependencies: {}\n      publish: true\n    validation:\n      interface: required\nstacks: {}\n"
        .into()
}
fn worker_package() -> String {
    "{\n  \"name\": \"generated-a2ui-worker\",\n  \"version\": \"0.1.0\",\n  \"private\": true,\n  \"type\": \"module\",\n  \"engines\": { \"node\": \">=22\" },\n  \"scripts\": { \"start\": \"node src/index.mjs\" },\n  \"dependencies\": { \"iii-sdk\": \"^0.21.6\" }\n}\n"
        .into()
}
fn worker_source() -> String {
    "import { registerWorker } from 'iii-sdk';\nimport surface from '../surface.json' with { type: 'json' };\n\nconst url = process.env.III_URL ?? process.env.III_ENGINE_URL ?? 'ws://127.0.0.1:49134';\nconst iii = registerWorker(url, { workerName: 'generated-a2ui' });\niii.registerFunction('generated-a2ui::surface', async () => surface);\n"
        .into()
}

fn receipt(surface: &SurfaceRecord, status: SurfaceStatus) -> SurfaceReceipt {
    SurfaceReceipt {
        session_id: surface.session_id.clone(),
        surface_id: surface.surface_id.clone(),
        title: surface.title.clone(),
        status,
        protocol_version: surface.protocol_version.clone(),
        catalog_id: surface.catalog_id.clone(),
        revision: surface.revision,
        component_count: surface.components.len(),
        page: PAGE_HASH.into(),
    }
}

fn generated_surface_id() -> String {
    let id = Uuid::new_v4().simple().to_string();
    format!("surface-{}", &id[..12])
}

fn title_from(description: &str) -> String {
    let title = description
        .split_whitespace()
        .take(9)
        .collect::<Vec<_>>()
        .join(" ");
    title.chars().take(120).collect()
}

fn require_session(session_id: Option<String>) -> Result<String, String> {
    let session_id = session_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "A2UI call has no Harness session context".to_string())?;
    if session_id.len() > 256 || session_id.chars().any(char::is_control) {
        return Err("session_id is invalid".into());
    }
    Ok(session_id)
}

fn validate_data_size(data: Option<&Value>, max_bytes: usize) -> Result<(), String> {
    let Some(data) = data else {
        return Ok(());
    };
    let bytes = serde_json::to_vec(data)
        .map_err(|error| format!("data is not serializable: {error}"))?
        .len();
    if bytes > max_bytes {
        return Err(format!("data is {bytes} bytes; maximum is {max_bytes}"));
    }
    Ok(())
}

fn validate_action_source(
    surface: &SurfaceRecord,
    source_component_id: &str,
    action_name: &str,
) -> Result<(), String> {
    let component = surface
        .components
        .iter()
        .find(|component| component.id == source_component_id)
        .ok_or_else(|| format!("component `{source_component_id}` was not found"))?;
    if component.component != "Button" {
        return Err(format!(
            "component `{source_component_id}` does not emit actions"
        ));
    }
    let declared = component
        .properties
        .get("action")
        .and_then(Value::as_object)
        .and_then(|action| action.get("event"))
        .and_then(Value::as_object)
        .and_then(|event| event.get("name"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("component `{source_component_id}` has no action event"))?;
    if declared != action_name {
        return Err(format!(
            "component `{source_component_id}` declares action `{declared}`, not `{action_name}`"
        ));
    }
    Ok(())
}

fn find_action<'a>(surface: &'a SurfaceRecord, action_id: &str) -> Option<&'a ActionRecord> {
    surface
        .last_action
        .as_ref()
        .filter(|action| action.action_id == action_id)
        .or_else(|| {
            surface
                .history
                .iter()
                .rev()
                .filter_map(|revision| revision.last_action.as_ref())
                .find(|action| action.action_id == action_id)
        })
}

fn validate_action_retry(action: &ActionRecord, request: &ActionRequest) -> Result<(), String> {
    if action.name != request.name
        || action.source_component_id != request.source_component_id
        || action.context != request.context
        || action.data_model != request.data_model
    {
        return Err(format!(
            "action id `{}` was already used for a different gesture",
            request.action_id
        ));
    }
    Ok(())
}

async fn save_state(
    deps: &Deps,
    state: &mut SessionState,
    cfg: &crate::config::WorkerConfig,
) -> Result<(), String> {
    enforce_state_limits(state, cfg)?;
    deps.store.save(state).await
}

pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

fn schema_of<T: JsonSchema>() -> schemars::schema::RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

fn spec<Req: JsonSchema, Resp: JsonSchema>(
    function_id: &'static str,
    description: &'static str,
) -> FunctionSpec {
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<GenerateRequest, SurfaceReceipt>(GENERATE_ID, GENERATE_DESC),
        spec::<ApplyRequest, SurfaceReceipt>(APPLY_ID, APPLY_DESC),
        spec::<GetRequest, SurfaceRecord>(GET_ID, GET_DESC),
        spec::<ListRequest, ListResponse>(LIST_ID, LIST_DESC),
        spec::<DeleteRequest, SurfaceReceipt>(DELETE_ID, DELETE_DESC),
        spec::<PatchRequest, SurfaceReceipt>(PATCH_ID, PATCH_DESC),
        spec::<ExportRequest, SurfaceExport>(EXPORT_ID, EXPORT_DESC),
        spec::<ActionRequest, ActionResponse>(ACTION_ID, ACTION_DESC),
        spec::<HistoryRequest, Vec<SurfaceRevision>>(HISTORY_ID, HISTORY_DESC),
        spec::<UndoRequest, SurfaceReceipt>(UNDO_ID, UNDO_DESC),
        spec::<DuplicateRequest, SurfaceReceipt>(DUPLICATE_ID, DUPLICATE_DESC),
        spec::<PinRequest, SurfaceReceipt>(PIN_ID, PIN_DESC),
        spec::<ImportRequest, SurfaceReceipt>(IMPORT_ID, IMPORT_DESC),
        spec::<ExportCodeRequest, CodeExport>(EXPORT_CODE_ID, EXPORT_CODE_DESC),
        spec::<BindingSetRequest, SurfaceReceipt>(BINDING_SET_ID, BINDING_SET_DESC),
        spec::<BindingDeleteRequest, SurfaceReceipt>(BINDING_DELETE_ID, BINDING_DELETE_DESC),
        spec::<BindingApplyRequest, SurfaceReceipt>(BINDING_APPLY_ID, BINDING_APPLY_DESC),
        spec::<TemplateSaveRequest, SurfaceTemplate>(TEMPLATE_SAVE_ID, TEMPLATE_SAVE_DESC),
        spec::<TemplateListRequest, TemplateListResponse>(TEMPLATE_LIST_ID, TEMPLATE_LIST_DESC),
        spec::<TemplateSelectRequest, SurfaceTemplate>(TEMPLATE_GET_ID, TEMPLATE_GET_DESC),
        spec::<TemplateApplyRequest, SurfaceReceipt>(TEMPLATE_APPLY_ID, TEMPLATE_APPLY_DESC),
        spec::<TemplateSelectRequest, bool>(TEMPLATE_DELETE_ID, TEMPLATE_DELETE_DESC),
        spec::<StampSessionEvent, Option<StampSessionResponse>>(
            STAMP_SESSION_ID,
            "Internal: stamp authoritative Harness context onto A2UI function arguments before dispatch.",
        ),
        spec::<OnConfigChangeEvent, OnConfigChangeResponse>(
            CONFIG_FN_ID,
            "Internal: reload authoritative A2UI configuration after a configuration update event.",
        ),
    ]
}

pub fn register_all(iii: &Arc<IIIClient>, deps: Deps) {
    let generate_deps = deps.clone();
    iii.register_function(
        GENERATE_ID,
        RegisterFunction::new_async(move |request: GenerateRequest| {
            let deps = generate_deps.clone();
            async move { generate(&deps, request).await.map_err(Error::Handler) }
        })
        .description(GENERATE_DESC),
    );

    let apply_deps = deps.clone();
    iii.register_function(
        APPLY_ID,
        RegisterFunction::new_async(move |request: ApplyRequest| {
            let deps = apply_deps.clone();
            async move { apply(&deps, request).await.map_err(Error::Handler) }
        })
        .description(APPLY_DESC),
    );

    let get_deps = deps.clone();
    iii.register_function(
        GET_ID,
        RegisterFunction::new_async(move |request: GetRequest| {
            let deps = get_deps.clone();
            async move { get(&deps, request).await.map_err(Error::Handler) }
        })
        .description(GET_DESC),
    );

    let list_deps = deps.clone();
    iii.register_function(
        LIST_ID,
        RegisterFunction::new_async(move |request: ListRequest| {
            let deps = list_deps.clone();
            async move { list(&deps, request).await.map_err(Error::Handler) }
        })
        .description(LIST_DESC),
    );

    let delete_deps = deps.clone();
    iii.register_function(
        DELETE_ID,
        RegisterFunction::new_async(move |request: DeleteRequest| {
            let deps = delete_deps.clone();
            async move { delete(&deps, request).await.map_err(Error::Handler) }
        })
        .description(DELETE_DESC),
    );

    let patch_deps = deps.clone();
    iii.register_function(
        PATCH_ID,
        RegisterFunction::new_async(move |request: PatchRequest| {
            let deps = patch_deps.clone();
            async move { patch(&deps, request).await.map_err(Error::Handler) }
        })
        .description(PATCH_DESC),
    );

    let export_deps = deps.clone();
    iii.register_function(
        EXPORT_ID,
        RegisterFunction::new_async(move |request: ExportRequest| {
            let deps = export_deps.clone();
            async move { export(&deps, request).await.map_err(Error::Handler) }
        })
        .description(EXPORT_DESC),
    );

    let action_deps = deps.clone();
    iii.register_function(
        ACTION_ID,
        RegisterFunction::new_async(move |request: ActionRequest| {
            let deps = action_deps.clone();
            async move { action(&deps, request).await.map_err(Error::Handler) }
        })
        .description(ACTION_DESC),
    );

    macro_rules! register_async {
        ($id:expr, $desc:expr, $request:ty, $handler:ident) => {{
            let handler_deps = deps.clone();
            iii.register_function(
                $id,
                RegisterFunction::new_async(move |request: $request| {
                    let deps = handler_deps.clone();
                    async move { $handler(&deps, request).await.map_err(Error::Handler) }
                })
                .description($desc),
            );
        }};
    }
    register_async!(HISTORY_ID, HISTORY_DESC, HistoryRequest, history);
    register_async!(UNDO_ID, UNDO_DESC, UndoRequest, undo);
    register_async!(DUPLICATE_ID, DUPLICATE_DESC, DuplicateRequest, duplicate);
    register_async!(PIN_ID, PIN_DESC, PinRequest, pin);
    register_async!(IMPORT_ID, IMPORT_DESC, ImportRequest, import_surface);
    register_async!(
        EXPORT_CODE_ID,
        EXPORT_CODE_DESC,
        ExportCodeRequest,
        export_code
    );
    register_async!(
        BINDING_SET_ID,
        BINDING_SET_DESC,
        BindingSetRequest,
        set_binding
    );
    register_async!(
        BINDING_DELETE_ID,
        BINDING_DELETE_DESC,
        BindingDeleteRequest,
        delete_binding
    );
    register_async!(
        BINDING_APPLY_ID,
        BINDING_APPLY_DESC,
        BindingApplyRequest,
        apply_binding
    );
    register_async!(
        TEMPLATE_SAVE_ID,
        TEMPLATE_SAVE_DESC,
        TemplateSaveRequest,
        save_template
    );
    register_async!(
        TEMPLATE_LIST_ID,
        TEMPLATE_LIST_DESC,
        TemplateListRequest,
        list_templates
    );
    register_async!(
        TEMPLATE_GET_ID,
        TEMPLATE_GET_DESC,
        TemplateSelectRequest,
        get_template
    );
    register_async!(
        TEMPLATE_APPLY_ID,
        TEMPLATE_APPLY_DESC,
        TemplateApplyRequest,
        apply_template
    );
    register_async!(
        TEMPLATE_DELETE_ID,
        TEMPLATE_DELETE_DESC,
        TemplateSelectRequest,
        delete_template
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_complete_and_ids_follow_namespace_rules() {
        let ids: Vec<&str> = catalog().iter().map(|spec| spec.function_id).collect();
        assert_eq!(
            ids,
            vec![
                GENERATE_ID,
                APPLY_ID,
                GET_ID,
                LIST_ID,
                DELETE_ID,
                PATCH_ID,
                EXPORT_ID,
                ACTION_ID,
                HISTORY_ID,
                UNDO_ID,
                DUPLICATE_ID,
                PIN_ID,
                IMPORT_ID,
                EXPORT_CODE_ID,
                BINDING_SET_ID,
                BINDING_DELETE_ID,
                BINDING_APPLY_ID,
                TEMPLATE_SAVE_ID,
                TEMPLATE_LIST_ID,
                TEMPLATE_GET_ID,
                TEMPLATE_APPLY_ID,
                TEMPLATE_DELETE_ID,
                STAMP_SESSION_ID,
                CONFIG_FN_ID,
            ]
        );
        for spec in catalog() {
            assert!(spec.function_id.starts_with("a2ui::"));
            assert!(!spec.function_id.contains('_'));
            assert!(spec.description.len() > 40);
        }
    }

    #[test]
    fn requests_accept_only_the_engine_metadata_field() {
        let request: ListRequest = serde_json::from_value(json!({
            "session_id": "session-1",
            "_caller_worker_id": "console"
        }))
        .unwrap();
        assert_eq!(request.session_id.as_deref(), Some("session-1"));
        assert!(serde_json::from_value::<ListRequest>(json!({
            "session_id": "session-1",
            "caller_worker_id": "console"
        }))
        .is_err());
    }

    #[test]
    fn exported_react_source_does_not_interpolate_surface_ids() {
        let source = react_source();
        assert!(source.contains("surfacePackage.surface_id"));
        assert!(!source.contains("__SURFACE_ID__"));
        assert!(source.contains("useState<JsonValue>"));
        assert!(source.contains("dataModel: sendDataModel ? model : undefined"));
        assert!(source.contains("const value = event.currentTarget.value"));
        assert!(source.contains("const checked = event.currentTarget.checked"));
        assert!(!source.contains("setAt(current, segments(path), event.currentTarget"));
    }

    #[test]
    fn exported_react_project_is_runnable_and_documents_the_handoff() {
        assert!(react_package().contains("\"dev\": \"vite\""));
        assert!(react_package().contains("\"build\": \"tsc --noEmit && vite build\""));
        assert!(react_index().contains("/src/main.tsx"));
        assert!(react_main().contains("<GeneratedSurface"));
        assert!(react_main().contains("./styles.css"));
        assert!(react_styles().contains("@media (max-width: 560px)"));
        assert!(react_vite_env().contains("vite/client"));
        assert!(react_tsconfig().contains("\"resolveJsonModule\": true"));
        assert!(react_readme().contains("Browser worker"));
    }

    #[test]
    fn exported_worker_uses_the_current_javascript_contract() {
        assert!(worker_compose().contains("kind: javascript-bundle"));
        assert!(worker_compose().contains("package_manifest: package.json"));
        assert!(worker_package().contains("\"iii-sdk\": \"^0.21.6\""));
        assert!(worker_source().contains("registerWorker(url"));
        assert!(worker_source().contains("process.env.III_URL"));
    }

    #[test]
    fn action_ids_are_reused_only_for_the_same_persisted_gesture() {
        let request = ActionRequest {
            session_id: Some("session-1".into()),
            surface_id: "surface-1".into(),
            source_component_id: "approve".into(),
            action_id: "gesture-1".into(),
            name: "approve".into(),
            context: json!({"release": "v1"}),
            data_model: Some(json!({"ready": true})),
            expected_revision: 1,
            _caller_worker_id: None,
        };
        let record = ActionRecord {
            action_id: request.action_id.clone(),
            name: request.name.clone(),
            source_component_id: request.source_component_id.clone(),
            context: request.context.clone(),
            data_model: request.data_model.clone(),
            timestamp_ms: 1,
        };
        assert!(validate_action_retry(&record, &request).is_ok());

        let mut changed = request;
        changed.name = "reject".into();
        assert!(validate_action_retry(&record, &changed).is_err());
    }
}
