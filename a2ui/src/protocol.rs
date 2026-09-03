//! A2UI v0.9.1 server-to-client messages plus the iii Console catalog.
//!
//! A2UI deliberately separates the protocol envelope from the component
//! catalog. This worker implements the stable v0.9.1 envelope and a small,
//! safe catalog rendered with the Console's own components and design tokens.

use std::collections::{BTreeMap, HashMap, HashSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::WorkerConfig;

pub const PROTOCOL_VERSION: &str = "v0.9.1";
pub const CATALOG_ID: &str = "urn:iii:a2ui:console:v0.1";
pub const PAGE_HASH: &str = "#/ext/a2ui";

const COMPONENT_TYPES: &[&str] = &[
    "Column",
    "Row",
    "Card",
    "Text",
    "Badge",
    "Button",
    "TextField",
    "CheckBox",
    "Divider",
];
const MAX_RENDER_DEPTH: usize = 32;
const MAX_JSON_POINTER_BYTES: usize = 1024;
const MAX_JSON_POINTER_SEGMENTS: usize = 64;
const UNSAFE_POINTER_SEGMENTS: &[&str] = &["__proto__", "prototype", "constructor"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSurface {
    #[serde(rename = "surfaceId")]
    pub surface_id: String,
    #[serde(rename = "catalogId")]
    pub catalog_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<Value>,
    #[serde(
        default,
        rename = "sendDataModel",
        skip_serializing_if = "Option::is_none"
    )]
    pub send_data_model: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSurfaceMessage {
    pub version: String,
    #[serde(rename = "createSurface")]
    pub create_surface: CreateSurface,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Component {
    /// Unique component id within the surface. `root` is the render root.
    pub id: String,
    /// Component name from the negotiated catalog.
    pub component: String,
    /// Catalog-defined properties, validated against the worker's supported
    /// component names and relationship rules before persistence.
    // The protocol type stays catalog-agnostic; validation lives in the worker.
    #[serde(flatten)]
    pub properties: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateComponents {
    #[serde(rename = "surfaceId")]
    pub surface_id: String,
    pub components: Vec<Component>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateComponentsMessage {
    pub version: String,
    #[serde(rename = "updateComponents")]
    pub update_components: UpdateComponents,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateDataModel {
    #[serde(rename = "surfaceId")]
    pub surface_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Omit or set null to remove the path. At `/` this replaces the whole
    /// data model, matching A2UI v0.9.1 root-update semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateDataModelMessage {
    pub version: String,
    #[serde(rename = "updateDataModel")]
    pub update_data_model: UpdateDataModel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteSurface {
    #[serde(rename = "surfaceId")]
    pub surface_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteSurfaceMessage {
    pub version: String,
    #[serde(rename = "deleteSurface")]
    pub delete_surface: DeleteSurface,
}

/// Exactly one of the four A2UI v0.9.1 server-to-client envelope messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ServerMessage {
    CreateSurface(CreateSurfaceMessage),
    UpdateComponents(UpdateComponentsMessage),
    UpdateDataModel(UpdateDataModelMessage),
    DeleteSurface(DeleteSurfaceMessage),
}

pub fn ensure_interactive_data_submission(messages: &mut [ServerMessage]) {
    let has_input = messages.iter().any(|message| match message {
        ServerMessage::UpdateComponents(message) => message
            .update_components
            .components
            .iter()
            .any(|component| matches!(component.component.as_str(), "TextField" | "CheckBox")),
        _ => false,
    });
    if !has_input {
        return;
    }
    for message in messages {
        if let ServerMessage::CreateSurface(message) = message {
            message.create_surface.send_data_model = Some(true);
        }
    }
}

impl ServerMessage {
    pub fn surface_id(&self) -> &str {
        match self {
            Self::CreateSurface(message) => &message.create_surface.surface_id,
            Self::UpdateComponents(message) => &message.update_components.surface_id,
            Self::UpdateDataModel(message) => &message.update_data_model.surface_id,
            Self::DeleteSurface(message) => &message.delete_surface.surface_id,
        }
    }

    fn version(&self) -> &str {
        match self {
            Self::CreateSurface(message) => &message.version,
            Self::UpdateComponents(message) => &message.version,
            Self::UpdateDataModel(message) => &message.version,
            Self::DeleteSurface(message) => &message.version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceStatus {
    Active,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActionRecord {
    #[serde(default)]
    pub action_id: String,
    pub name: String,
    pub source_component_id: String,
    pub context: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_model: Option<Value>,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SurfaceRecord {
    /// Authoritative Harness session stamped by the pre-trigger hook.
    pub session_id: String,
    /// Globally unique inside the session.
    pub surface_id: String,
    pub protocol_version: String,
    pub catalog_id: String,
    pub title: String,
    pub theme: Option<Value>,
    pub send_data_model: bool,
    pub components: Vec<Component>,
    pub data_model: Value,
    pub revision: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub last_action: Option<ActionRecord>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub bindings: Vec<LiveBinding>,
    #[serde(default)]
    pub history: Vec<SurfaceRevision>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SurfaceRevision {
    pub revision: u64,
    pub title: String,
    pub theme: Option<Value>,
    pub send_data_model: bool,
    pub components: Vec<Component>,
    pub data_model: Value,
    pub last_action: Option<ActionRecord>,
    pub updated_at_ms: i64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LiveBinding {
    pub id: String,
    pub trigger_type: String,
    pub config: Value,
    pub target_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SurfaceTemplate {
    pub template_id: String,
    pub title: String,
    pub description: String,
    pub protocol_version: String,
    pub catalog_id: String,
    pub theme: Option<Value>,
    pub send_data_model: bool,
    pub components: Vec<Component>,
    pub data_model: Value,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SurfaceExport {
    #[serde(rename = "format")]
    pub export_format: String,
    pub format_version: u32,
    pub protocol_version: String,
    pub catalog_id: String,
    pub surface_id: String,
    pub title: String,
    pub messages: Vec<ServerMessage>,
}

pub fn export_surface(surface: &SurfaceRecord) -> SurfaceExport {
    SurfaceExport {
        export_format: "a2ui.surface".into(),
        format_version: 1,
        protocol_version: surface.protocol_version.clone(),
        catalog_id: surface.catalog_id.clone(),
        surface_id: surface.surface_id.clone(),
        title: surface.title.clone(),
        messages: vec![
            ServerMessage::CreateSurface(CreateSurfaceMessage {
                version: surface.protocol_version.clone(),
                create_surface: CreateSurface {
                    surface_id: surface.surface_id.clone(),
                    catalog_id: surface.catalog_id.clone(),
                    theme: surface.theme.clone(),
                    send_data_model: Some(surface.send_data_model),
                },
            }),
            ServerMessage::UpdateComponents(UpdateComponentsMessage {
                version: surface.protocol_version.clone(),
                update_components: UpdateComponents {
                    surface_id: surface.surface_id.clone(),
                    components: surface.components.clone(),
                },
            }),
            ServerMessage::UpdateDataModel(UpdateDataModelMessage {
                version: surface.protocol_version.clone(),
                update_data_model: UpdateDataModel {
                    surface_id: surface.surface_id.clone(),
                    path: Some("/".into()),
                    value: Some(surface.data_model.clone()),
                },
            }),
        ],
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SurfaceSummary {
    pub surface_id: String,
    pub title: String,
    pub protocol_version: String,
    pub catalog_id: String,
    pub component_count: usize,
    pub revision: u64,
    pub updated_at_ms: i64,
    pub pinned: bool,
    pub binding_count: usize,
}

impl From<&SurfaceRecord> for SurfaceSummary {
    fn from(surface: &SurfaceRecord) -> Self {
        Self {
            surface_id: surface.surface_id.clone(),
            title: surface.title.clone(),
            protocol_version: surface.protocol_version.clone(),
            catalog_id: surface.catalog_id.clone(),
            component_count: surface.components.len(),
            revision: surface.revision,
            updated_at_ms: surface.updated_at_ms,
            pinned: surface.pinned,
            binding_count: surface.bindings.len(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionState {
    pub session_id: String,
    pub surfaces: Vec<SurfaceRecord>,
    #[serde(default)]
    pub templates: Vec<SurfaceTemplate>,
    pub updated_at_ms: i64,
}

impl SessionState {
    pub fn empty(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            surfaces: Vec::new(),
            templates: Vec::new(),
            updated_at_ms: now_ms(),
        }
    }

    pub fn get(&self, surface_id: &str) -> Option<&SurfaceRecord> {
        self.surfaces
            .iter()
            .find(|surface| surface.surface_id == surface_id)
    }

    pub fn get_mut(&mut self, surface_id: &str) -> Option<&mut SurfaceRecord> {
        self.surfaces
            .iter_mut()
            .find(|surface| surface.surface_id == surface_id)
    }
}

#[derive(Debug)]
pub struct ApplyOutcome {
    pub surface_id: String,
    pub status: SurfaceStatus,
    pub revision: u64,
    pub component_count: usize,
}

pub fn apply_messages(
    state: &mut SessionState,
    messages: &[ServerMessage],
    title: Option<&str>,
    cfg: &WorkerConfig,
) -> Result<ApplyOutcome, String> {
    if messages.is_empty() {
        return Err("messages must contain at least one A2UI envelope".into());
    }
    let surface_id = messages[0].surface_id().to_string();
    validate_identifier("surface_id", &surface_id)?;
    if messages
        .iter()
        .any(|message| message.surface_id() != surface_id)
    {
        return Err("one apply batch may target only one surface_id".into());
    }
    for message in messages {
        if message.version() != PROTOCOL_VERSION {
            return Err(format!(
                "unsupported A2UI version `{}`; expected `{PROTOCOL_VERSION}`",
                message.version()
            ));
        }
    }

    let mut last_revision = state.get(&surface_id).map_or(0, |surface| surface.revision);
    let mut status = SurfaceStatus::Active;
    for message in messages {
        match message {
            ServerMessage::CreateSurface(message) => {
                if state.get(&surface_id).is_some() {
                    return Err(format!(
                        "surface `{surface_id}` already exists; delete it before createSurface"
                    ));
                }
                if state.surfaces.len() >= cfg.max_surfaces_per_session {
                    return Err(format!(
                        "session already has the configured maximum of {} surfaces",
                        cfg.max_surfaces_per_session
                    ));
                }
                if message.create_surface.catalog_id != CATALOG_ID {
                    return Err(format!(
                        "unsupported catalog `{}`; this worker renders `{CATALOG_ID}`",
                        message.create_surface.catalog_id
                    ));
                }
                let now = now_ms();
                last_revision = 1;
                status = SurfaceStatus::Active;
                state.surfaces.push(SurfaceRecord {
                    session_id: state.session_id.clone(),
                    surface_id: surface_id.clone(),
                    protocol_version: PROTOCOL_VERSION.into(),
                    catalog_id: CATALOG_ID.into(),
                    title: clean_title(title.unwrap_or(&surface_id)),
                    theme: message.create_surface.theme.clone(),
                    send_data_model: message.create_surface.send_data_model.unwrap_or(false),
                    components: Vec::new(),
                    data_model: Value::Object(Default::default()),
                    revision: last_revision,
                    created_at_ms: now,
                    updated_at_ms: now,
                    last_action: None,
                    pinned: false,
                    bindings: Vec::new(),
                    history: Vec::new(),
                });
            }
            ServerMessage::UpdateComponents(message) => {
                validate_components(&message.update_components.components, cfg)?;
                let surface = state
                    .get_mut(&surface_id)
                    .ok_or_else(|| format!("surface `{surface_id}` must be created first"))?;
                for component in &message.update_components.components {
                    if let Some(existing) = surface
                        .components
                        .iter_mut()
                        .find(|existing| existing.id == component.id)
                    {
                        *existing = component.clone();
                    } else {
                        surface.components.push(component.clone());
                    }
                }
                if surface.components.len() > cfg.max_components_per_surface {
                    return Err(format!(
                        "surface exceeds the configured maximum of {} components",
                        cfg.max_components_per_surface
                    ));
                }
                surface.revision += 1;
                surface.updated_at_ms = now_ms();
                last_revision = surface.revision;
            }
            ServerMessage::UpdateDataModel(message) => {
                let surface = state
                    .get_mut(&surface_id)
                    .ok_or_else(|| format!("surface `{surface_id}` must be created first"))?;
                apply_data_update(
                    &mut surface.data_model,
                    message.update_data_model.path.as_deref(),
                    message.update_data_model.value.clone(),
                )?;
                surface.revision += 1;
                surface.updated_at_ms = now_ms();
                last_revision = surface.revision;
            }
            ServerMessage::DeleteSurface(_) => {
                let before = state.surfaces.len();
                state
                    .surfaces
                    .retain(|surface| surface.surface_id != surface_id);
                if state.surfaces.len() == before {
                    return Err(format!("surface `{surface_id}` was not found"));
                }
                last_revision += 1;
                status = SurfaceStatus::Deleted;
            }
        }
    }
    state.updated_at_ms = now_ms();
    if matches!(status, SurfaceStatus::Active) {
        let surface = state
            .get(&surface_id)
            .ok_or_else(|| "active surface disappeared during apply".to_string())?;
        validate_renderable(surface)?;
        validate_surface_payload(surface, cfg)?;
    }
    let component_count = state
        .get(&surface_id)
        .map_or(0, |surface| surface.components.len());
    Ok(ApplyOutcome {
        surface_id,
        status,
        revision: last_revision,
        component_count,
    })
}

pub fn snapshot(surface: &SurfaceRecord, reason: impl Into<String>) -> SurfaceRevision {
    SurfaceRevision {
        revision: surface.revision,
        title: surface.title.clone(),
        theme: surface.theme.clone(),
        send_data_model: surface.send_data_model,
        components: surface.components.clone(),
        data_model: surface.data_model.clone(),
        last_action: surface.last_action.clone(),
        updated_at_ms: surface.updated_at_ms,
        reason: reason.into(),
    }
}

pub fn push_history(surface: &mut SurfaceRecord, entry: SurfaceRevision, limit: usize) {
    surface.history.push(entry);
    if surface.history.len() > limit {
        surface.history.drain(..surface.history.len() - limit);
    }
}

pub fn validate_live_binding(binding: &LiveBinding) -> Result<(), String> {
    validate_identifier("binding id", &binding.id)?;
    validate_json_pointer("binding target_path", &binding.target_path)?;
    if let Some(path) = binding.event_path.as_deref() {
        validate_json_pointer("binding event_path", path)?;
    }
    match binding.trigger_type.as_str() {
        "state" => {
            validate_binding_config(&binding.config, &["scope", "key"], &[])?;
            if binding.config.get("scope").and_then(Value::as_str) == Some("a2ui") {
                return Err("bindings cannot subscribe to A2UI's own state scope".into());
            }
        }
        "stream" => {
            validate_binding_config(&binding.config, &["stream_name", "group_id"], &["item_id"])?
        }
        "shell::changed" => validate_binding_config(&binding.config, &["path"], &[])?,
        _ => return Err("binding trigger_type must be state, stream, or shell::changed".into()),
    }
    Ok(())
}

fn validate_binding_config(
    config: &Value,
    required: &[&str],
    optional: &[&str],
) -> Result<(), String> {
    let object = config
        .as_object()
        .ok_or_else(|| "binding config must be an object".to_string())?;
    for key in object.keys() {
        if !required.contains(&key.as_str()) && !optional.contains(&key.as_str()) {
            return Err(format!("binding config contains unsupported field `{key}`"));
        }
    }
    for key in required {
        let value = object
            .get(*key)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| format!("binding config `{key}` must be a non-empty string"))?;
        if value.len() > MAX_JSON_POINTER_BYTES {
            return Err(format!("binding config `{key}` is too long"));
        }
    }
    for key in optional {
        if let Some(value) = object.get(*key) {
            let value = value
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| format!("binding config `{key}` must be a non-empty string"))?;
            if value.len() > MAX_JSON_POINTER_BYTES {
                return Err(format!("binding config `{key}` is too long"));
            }
        }
    }
    Ok(())
}

pub fn set_data_path(target: &mut Value, path: &str, value: Value) -> Result<(), String> {
    apply_data_update(target, Some(path), Some(value))
}

pub fn validate_renderable(surface: &SurfaceRecord) -> Result<(), String> {
    if surface
        .components
        .iter()
        .all(|component| component.id != "root")
    {
        return Err("surface has no `root` component".into());
    }
    validate_graph(&surface.components)
}

fn validate_components(components: &[Component], cfg: &WorkerConfig) -> Result<(), String> {
    if components.is_empty() {
        return Err("updateComponents.components must not be empty".into());
    }
    if components.len() > cfg.max_components_per_surface {
        return Err(format!(
            "batch exceeds the configured maximum of {} components",
            cfg.max_components_per_surface
        ));
    }
    let mut ids = HashSet::new();
    for component in components {
        validate_identifier("component id", &component.id)?;
        if !ids.insert(component.id.as_str()) {
            return Err(format!(
                "duplicate component id `{}` in batch",
                component.id
            ));
        }
        if !COMPONENT_TYPES.contains(&component.component.as_str()) {
            return Err(format!(
                "unsupported component `{}`; supported Console catalog components: {}",
                component.component,
                COMPONENT_TYPES.join(", ")
            ));
        }
        validate_component_bindings(component)?;
    }
    Ok(())
}

fn validate_graph(components: &[Component]) -> Result<(), String> {
    let ids: HashSet<&str> = components
        .iter()
        .map(|component| component.id.as_str())
        .collect();
    let mut parents: HashMap<String, String> = HashMap::new();
    for component in components {
        let mut local_children = HashSet::new();
        for child in child_ids(component) {
            if !ids.contains(child.as_str()) {
                return Err(format!(
                    "component `{}` references missing child `{child}`",
                    component.id
                ));
            }
            if !local_children.insert(child.clone()) {
                return Err(format!(
                    "component `{}` references child `{child}` more than once",
                    component.id
                ));
            }
            if let Some(previous) = parents.insert(child.clone(), component.id.clone()) {
                return Err(format!(
                    "component `{child}` has multiple parents: `{previous}` and `{}`",
                    component.id
                ));
            }
        }
    }
    if parents.contains_key("root") {
        return Err("root component must not have a parent".into());
    }
    let mut visited = HashSet::new();
    for id in &ids {
        visit(id, components, &mut HashSet::new(), &mut visited, 0)?;
    }
    Ok(())
}

fn visit(
    id: &str,
    components: &[Component],
    visiting: &mut HashSet<String>,
    visited: &mut HashSet<String>,
    depth: usize,
) -> Result<(), String> {
    if visited.contains(id) {
        return Ok(());
    }
    if depth > MAX_RENDER_DEPTH {
        return Err(format!(
            "component graph exceeds the maximum render depth of {MAX_RENDER_DEPTH}"
        ));
    }
    if !visiting.insert(id.to_string()) {
        return Err(format!("component graph contains a cycle at `{id}`"));
    }
    let component = components
        .iter()
        .find(|component| component.id == id)
        .ok_or_else(|| format!("component `{id}` was not found"))?;
    for child in child_ids(component) {
        visit(&child, components, visiting, visited, depth + 1)?;
    }
    visiting.remove(id);
    visited.insert(id.to_string());
    Ok(())
}

fn child_ids(component: &Component) -> Vec<String> {
    let mut children = Vec::new();
    if let Some(child) = component.properties.get("child").and_then(Value::as_str) {
        children.push(child.to_string());
    }
    if let Some(values) = component
        .properties
        .get("children")
        .and_then(Value::as_array)
    {
        children.extend(values.iter().filter_map(Value::as_str).map(str::to_string));
    }
    children
}

fn apply_data_update(
    target: &mut Value,
    path: Option<&str>,
    value: Option<Value>,
) -> Result<(), String> {
    let path = path.unwrap_or("/");
    if path.is_empty() || path == "/" {
        *target = value.unwrap_or(Value::Object(Default::default()));
        return Ok(());
    }
    let segments = validate_json_pointer("data model path", path)?;
    apply_data_segments(target, &segments, value, path)
}

fn apply_data_segments(
    target: &mut Value,
    segments: &[String],
    value: Option<Value>,
    path: &str,
) -> Result<(), String> {
    let (segment, rest) = segments
        .split_first()
        .ok_or_else(|| "data model path must not be empty".to_string())?;
    if rest.is_empty() {
        return match target {
            Value::Object(object) => {
                if let Some(value) = value {
                    object.insert(segment.clone(), value);
                } else {
                    object.remove(segment);
                }
                Ok(())
            }
            Value::Array(array) => {
                let index = segment.parse::<usize>().map_err(|_| {
                    format!("array segment `{segment}` in `{path}` is not an index")
                })?;
                if index > array.len() {
                    return Err(format!("array index {index} is out of bounds in `{path}`"));
                }
                match value {
                    Some(value) if index == array.len() => array.push(value),
                    Some(value) => array[index] = value,
                    None if index < array.len() => {
                        array.remove(index);
                    }
                    None => {}
                }
                Ok(())
            }
            _ => Err(format!("data model parent for `{path}` is not a container")),
        };
    }
    match target {
        Value::Object(object) => {
            let child = object
                .entry(segment.clone())
                .or_insert_with(|| Value::Object(Default::default()));
            apply_data_segments(child, rest, value, path)
        }
        Value::Array(array) => {
            let index = segment
                .parse::<usize>()
                .map_err(|_| format!("array segment `{segment}` in `{path}` is not an index"))?;
            let child = array
                .get_mut(index)
                .ok_or_else(|| format!("array index {index} is out of bounds in `{path}`"))?;
            apply_data_segments(child, rest, value, path)
        }
        _ => Err(format!("data model parent for `{path}` is not a container")),
    }
}

fn decode_pointer(segment: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut chars = segment.chars();
    while let Some(ch) = chars.next() {
        if ch != '~' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('0') => out.push('~'),
            Some('1') => out.push('/'),
            Some(other) => return Err(format!("invalid JSON Pointer escape `~{other}`")),
            None => return Err("invalid trailing `~` in JSON Pointer".into()),
        }
    }
    Ok(out)
}

pub(crate) fn validate_identifier(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 128 {
        return Err(format!("{label} must be between 1 and 128 bytes"));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(format!(
            "{label} may contain only ASCII letters, digits, dash, underscore, dot, or colon"
        ));
    }
    Ok(())
}

fn validate_json_pointer(label: &str, path: &str) -> Result<Vec<String>, String> {
    if path.len() > MAX_JSON_POINTER_BYTES {
        return Err(format!(
            "{label} is {} bytes; maximum is {MAX_JSON_POINTER_BYTES}",
            path.len()
        ));
    }
    if path.is_empty() || path == "/" {
        return Ok(Vec::new());
    }
    if !path.starts_with('/') {
        return Err(format!("{label} `{path}` must be a JSON Pointer"));
    }
    let segments: Vec<String> = path[1..]
        .split('/')
        .map(decode_pointer)
        .collect::<Result<_, _>>()?;
    if segments.len() > MAX_JSON_POINTER_SEGMENTS {
        return Err(format!(
            "{label} has {} segments; maximum is {MAX_JSON_POINTER_SEGMENTS}",
            segments.len()
        ));
    }
    if let Some(segment) = segments
        .iter()
        .find(|segment| UNSAFE_POINTER_SEGMENTS.contains(&segment.as_str()))
    {
        return Err(format!("{label} contains unsafe segment `{segment}`"));
    }
    Ok(segments)
}

fn validate_component_bindings(component: &Component) -> Result<(), String> {
    match component.component.as_str() {
        "Text" | "Badge" => {
            if let Some(value) = component.properties.get("text") {
                validate_binding_paths(value)?;
            }
        }
        "TextField" | "CheckBox" => {
            if let Some(value) = component.properties.get("value") {
                validate_binding_paths(value)?;
            }
        }
        "Button" => {
            if let Some(value) = component
                .properties
                .get("action")
                .and_then(Value::as_object)
                .and_then(|action| action.get("event"))
                .and_then(Value::as_object)
                .and_then(|event| event.get("context"))
            {
                validate_binding_paths(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_binding_paths(value: &Value) -> Result<(), String> {
    match value {
        Value::Array(values) => {
            for value in values {
                validate_binding_paths(value)?;
            }
        }
        Value::Object(values) => {
            if let Some(path) = values
                .get("path")
                .and_then(Value::as_str)
                .filter(|path| path.starts_with('/'))
            {
                validate_json_pointer("component binding path", path)?;
            }
            for value in values.values() {
                validate_binding_paths(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn serialized_len<T: Serialize>(label: &str, value: &T) -> Result<usize, String> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|error| format!("{label} is not serializable: {error}"))
}

pub fn validate_surface_payload(surface: &SurfaceRecord, cfg: &WorkerConfig) -> Result<(), String> {
    let data_bytes = serialized_len("surface data model", &surface.data_model)?;
    if data_bytes > cfg.max_data_bytes {
        return Err(format!(
            "surface data model is {data_bytes} bytes; maximum is {}",
            cfg.max_data_bytes
        ));
    }
    let surface_bytes = serialized_len("surface", surface)?;
    if surface_bytes > cfg.max_surface_bytes {
        return Err(format!(
            "surface is {surface_bytes} bytes; maximum is {}",
            cfg.max_surface_bytes
        ));
    }
    Ok(())
}

pub fn enforce_state_limits(state: &mut SessionState, cfg: &WorkerConfig) -> Result<(), String> {
    for surface in &mut state.surfaces {
        while serialized_len("surface", surface)? > cfg.max_surface_bytes
            && !surface.history.is_empty()
        {
            surface.history.remove(0);
        }
        validate_surface_payload(surface, cfg)?;
    }
    for template in &state.templates {
        let data_bytes = serialized_len("template data model", &template.data_model)?;
        if data_bytes > cfg.max_data_bytes {
            return Err(format!(
                "template data model is {data_bytes} bytes; maximum is {}",
                cfg.max_data_bytes
            ));
        }
        let template_bytes = serialized_len("template", template)?;
        if template_bytes > cfg.max_surface_bytes {
            return Err(format!(
                "template is {template_bytes} bytes; maximum is {}",
                cfg.max_surface_bytes
            ));
        }
    }
    loop {
        let session_bytes = serialized_len("session", state)?;
        if session_bytes <= cfg.max_session_bytes {
            return Ok(());
        }
        let oldest = state
            .surfaces
            .iter()
            .enumerate()
            .filter_map(|(index, surface)| {
                surface
                    .history
                    .first()
                    .map(|entry| (index, entry.updated_at_ms))
            })
            .min_by_key(|(_, updated_at_ms)| *updated_at_ms)
            .map(|(index, _)| index);
        let Some(index) = oldest else {
            return Err(format!(
                "session is {session_bytes} bytes; maximum is {}",
                cfg.max_session_bytes
            ));
        };
        state.surfaces[index].history.remove(0);
    }
}

fn clean_title(title: &str) -> String {
    let title = title.trim();
    if title.is_empty() {
        "generated surface".into()
    } else {
        title.chars().take(120).collect()
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create(id: &str) -> ServerMessage {
        serde_json::from_value(json!({
            "version": PROTOCOL_VERSION,
            "createSurface": {"surfaceId": id, "catalogId": CATALOG_ID}
        }))
        .unwrap()
    }

    fn components(id: &str) -> ServerMessage {
        serde_json::from_value(json!({
            "version": PROTOCOL_VERSION,
            "updateComponents": {"surfaceId": id, "components": [
                {"id": "root", "component": "Column", "children": ["title"]},
                {"id": "title", "component": "Text", "text": "Hello"}
            ]}
        }))
        .unwrap()
    }

    #[test]
    fn stable_envelope_round_trips_with_camel_case_keys() {
        let message = create("main");
        let value = serde_json::to_value(message).unwrap();
        assert_eq!(value["version"], PROTOCOL_VERSION);
        assert_eq!(value["createSurface"]["surfaceId"], "main");
    }

    #[test]
    fn create_update_and_data_form_a_renderable_surface() {
        let mut state = SessionState::empty("session-1");
        let data: ServerMessage = serde_json::from_value(json!({
            "version": PROTOCOL_VERSION,
            "updateDataModel": {"surfaceId": "main", "path": "/user", "value": {"name": "Rohit"}}
        }))
        .unwrap();
        let outcome = apply_messages(
            &mut state,
            &[create("main"), components("main"), data],
            Some("Profile"),
            &WorkerConfig::default(),
        )
        .unwrap();
        assert_eq!(outcome.component_count, 2);
        assert_eq!(
            state.get("main").unwrap().data_model["user"]["name"],
            "Rohit"
        );
        validate_renderable(state.get("main").unwrap()).unwrap();
    }

    #[test]
    fn interactive_components_force_full_model_submission() {
        let mut messages = vec![
            create("form"),
            serde_json::from_value(json!({
                "version": PROTOCOL_VERSION,
                "updateComponents": {"surfaceId": "form", "components": [
                    {"id": "root", "component": "Column", "children": ["name"]},
                    {"id": "name", "component": "TextField", "label": "Name", "value": {"path": "/name"}}
                ]}
            }))
            .unwrap(),
        ];
        ensure_interactive_data_submission(&mut messages);
        let ServerMessage::CreateSurface(message) = &messages[0] else {
            panic!("expected createSurface");
        };
        assert_eq!(message.create_surface.send_data_model, Some(true));
    }

    #[test]
    fn exported_surface_replays_as_portable_protocol_messages() {
        let mut original = SessionState::empty("session-a");
        apply_messages(
            &mut original,
            &[create("main"), components("main")],
            Some("Portable surface"),
            &WorkerConfig::default(),
        )
        .unwrap();
        let exported = export_surface(original.get("main").unwrap());
        assert_eq!(exported.export_format, "a2ui.surface");
        assert_eq!(exported.format_version, 1);

        let mut restored = SessionState::empty("session-b");
        apply_messages(
            &mut restored,
            &exported.messages,
            Some(&exported.title),
            &WorkerConfig::default(),
        )
        .unwrap();
        let surface = restored.get("main").unwrap();
        assert_eq!(surface.title, "Portable surface");
        assert_eq!(surface.components, original.get("main").unwrap().components);
        assert_eq!(surface.data_model, original.get("main").unwrap().data_model);
    }

    #[test]
    fn rejects_wrong_versions_unknown_catalogs_and_cycles() {
        let wrong: ServerMessage = serde_json::from_value(json!({
            "version": "v1.0",
            "createSurface": {"surfaceId": "main", "catalogId": CATALOG_ID}
        }))
        .unwrap();
        assert!(apply_messages(
            &mut SessionState::empty("s"),
            &[wrong],
            None,
            &WorkerConfig::default()
        )
        .is_err());

        let cycle: ServerMessage = serde_json::from_value(json!({
            "version": PROTOCOL_VERSION,
            "updateComponents": {"surfaceId": "main", "components": [
                {"id": "root", "component": "Column", "children": ["root"]}
            ]}
        }))
        .unwrap();
        let mut state = SessionState::empty("s");
        assert!(apply_messages(
            &mut state,
            &[create("main"), cycle],
            None,
            &WorkerConfig::default()
        )
        .is_err());
    }

    #[test]
    fn rejects_oversized_data_without_changing_the_original_state() {
        let cfg = WorkerConfig {
            max_data_bytes: 32,
            ..WorkerConfig::default()
        };
        let original = SessionState::empty("s");
        let mut candidate = original.clone();
        let data: ServerMessage = serde_json::from_value(json!({
            "version": PROTOCOL_VERSION,
            "updateDataModel": {
                "surfaceId": "main",
                "path": "/payload",
                "value": "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
            }
        }))
        .unwrap();
        let error = apply_messages(
            &mut candidate,
            &[create("main"), components("main"), data],
            None,
            &cfg,
        )
        .unwrap_err();
        assert!(error.contains("surface data model"), "{error}");
        assert!(original.surfaces.is_empty());
    }

    #[test]
    fn rejects_unsafe_pointers_and_expanding_component_graphs() {
        let unsafe_data: ServerMessage = serde_json::from_value(json!({
            "version": PROTOCOL_VERSION,
            "updateDataModel": {
                "surfaceId": "main",
                "path": "/__proto__/polluted",
                "value": true
            }
        }))
        .unwrap();
        assert!(apply_messages(
            &mut SessionState::empty("s"),
            &[create("main"), unsafe_data],
            None,
            &WorkerConfig::default(),
        )
        .is_err());

        let expanding: ServerMessage = serde_json::from_value(json!({
            "version": PROTOCOL_VERSION,
            "updateComponents": {"surfaceId": "main", "components": [
                {"id": "root", "component": "Column", "children": ["shared", "shared"]},
                {"id": "shared", "component": "Text", "text": "once"}
            ]}
        }))
        .unwrap();
        assert!(apply_messages(
            &mut SessionState::empty("s"),
            &[create("main"), expanding],
            None,
            &WorkerConfig::default(),
        )
        .is_err());
    }

    #[test]
    fn json_pointer_updates_preserve_array_parents() {
        let mut state = SessionState::empty("s");
        let initial: ServerMessage = serde_json::from_value(json!({
            "version": PROTOCOL_VERSION,
            "updateDataModel": {
                "surfaceId": "main",
                "path": "/",
                "value": {"items": [{"status": "pending"}]}
            }
        }))
        .unwrap();
        apply_messages(
            &mut state,
            &[create("main"), components("main"), initial],
            None,
            &WorkerConfig::default(),
        )
        .unwrap();
        let update: ServerMessage = serde_json::from_value(json!({
            "version": PROTOCOL_VERSION,
            "updateDataModel": {
                "surfaceId": "main",
                "path": "/items/0/status",
                "value": "ready"
            }
        }))
        .unwrap();
        apply_messages(&mut state, &[update], None, &WorkerConfig::default()).unwrap();
        let data = &state.get("main").unwrap().data_model;
        assert!(data["items"].is_array());
        assert_eq!(data["items"][0]["status"], "ready");
    }

    #[test]
    fn json_pointers_have_bounded_size_and_depth() {
        let oversized = format!("/{}", "x".repeat(MAX_JSON_POINTER_BYTES));
        assert!(validate_json_pointer("test path", &oversized).is_err());
        let deep = format!("/{}", vec!["x"; MAX_JSON_POINTER_SEGMENTS + 1].join("/"));
        assert!(validate_json_pointer("test path", &deep).is_err());
        let at_limit = format!("/{}", vec!["x"; MAX_JSON_POINTER_SEGMENTS].join("/"));
        assert_eq!(
            validate_json_pointer("test path", &at_limit).unwrap().len(),
            MAX_JSON_POINTER_SEGMENTS
        );
    }

    #[test]
    fn active_batches_require_renderable_surfaces_and_recreate_after_delete() {
        assert!(apply_messages(
            &mut SessionState::empty("s"),
            &[create("empty")],
            None,
            &WorkerConfig::default(),
        )
        .is_err());

        let mut state = SessionState::empty("s");
        apply_messages(
            &mut state,
            &[create("main"), components("main")],
            None,
            &WorkerConfig::default(),
        )
        .unwrap();
        let delete: ServerMessage = serde_json::from_value(json!({
            "version": PROTOCOL_VERSION,
            "deleteSurface": {"surfaceId": "main"}
        }))
        .unwrap();
        let outcome = apply_messages(
            &mut state,
            &[delete, create("main"), components("main")],
            None,
            &WorkerConfig::default(),
        )
        .unwrap();
        assert!(matches!(outcome.status, SurfaceStatus::Active));
        assert!(state.get("main").is_some());
    }

    #[test]
    fn history_is_bounded_and_old_state_gets_new_defaults() {
        let mut state = SessionState::empty("s");
        apply_messages(
            &mut state,
            &[create("main"), components("main")],
            None,
            &WorkerConfig::default(),
        )
        .unwrap();
        let surface = state.get_mut("main").unwrap();
        let first = snapshot(surface, "first");
        push_history(surface, first, 2);
        let second = snapshot(surface, "second");
        push_history(surface, second, 2);
        let third = snapshot(surface, "third");
        push_history(surface, third, 2);
        assert_eq!(surface.history.len(), 2);

        let mut value = serde_json::to_value(surface).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("history");
        object.remove("bindings");
        object.remove("pinned");
        let restored: SurfaceRecord = serde_json::from_value(value).unwrap();
        assert!(restored.history.is_empty());
        assert!(restored.bindings.is_empty());
        assert!(!restored.pinned);
    }

    #[test]
    fn stored_byte_limits_prune_old_history_before_rejecting_a_surface() {
        let mut state = SessionState::empty("s");
        apply_messages(
            &mut state,
            &[create("main"), components("main")],
            None,
            &WorkerConfig::default(),
        )
        .unwrap();
        let surface = state.get_mut("main").unwrap();
        for reason in ["one", "two", "three"] {
            let entry = snapshot(surface, reason);
            push_history(surface, entry, 8);
        }
        let mut current_only = surface.clone();
        current_only.history.clear();
        let base = serialized_len("surface", &current_only).unwrap();
        let cfg = WorkerConfig {
            max_surface_bytes: base + 64,
            max_session_bytes: base + 512,
            ..WorkerConfig::default()
        };
        enforce_state_limits(&mut state, &cfg).unwrap();
        assert!(state.get("main").unwrap().history.is_empty());
    }

    #[test]
    fn live_bindings_are_allowlisted_and_cannot_loop_on_a2ui_state() {
        let valid = LiveBinding {
            id: "shell-files".into(),
            trigger_type: "shell::changed".into(),
            config: json!({"path": "/workspace"}),
            target_path: "/last_change".into(),
            event_path: None,
        };
        assert!(validate_live_binding(&valid).is_ok());
        let mut invalid = valid.clone();
        invalid.trigger_type = "harness::pre-turn".into();
        assert!(validate_live_binding(&invalid).is_err());
        invalid.trigger_type = "state".into();
        invalid.config = json!({"scope": "a2ui", "key": "s"});
        assert!(validate_live_binding(&invalid).is_err());

        invalid.config = Value::Null;
        assert!(validate_live_binding(&invalid).is_err());
        invalid.config = json!({"scope": "orders"});
        assert!(validate_live_binding(&invalid).is_err());
        invalid.config = json!({"scope": "orders", "key": "active", "function_id": "unsafe"});
        assert!(validate_live_binding(&invalid).is_err());

        invalid.trigger_type = "stream".into();
        invalid.config = json!({"stream_name": "agent::events", "group_id": "session-1"});
        assert!(validate_live_binding(&invalid).is_ok());
        invalid.config = json!({"stream_name": "agent::events"});
        assert!(validate_live_binding(&invalid).is_err());

        invalid.trigger_type = "browser::console-event".into();
        invalid.config = json!({"session_id": "browser-session"});
        assert!(validate_live_binding(&invalid).is_err());

        invalid.trigger_type = "shell::changed".into();
        invalid.target_path = "/constructor/polluted".into();
        assert!(validate_live_binding(&invalid).is_err());
    }
}
