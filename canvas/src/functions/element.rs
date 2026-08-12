//! `canvas::element::*` — element-level operations on a freeform canvas.
//!
//! The whole point is incremental agent drawing: instead of rewriting the
//! scene JSON per change, an agent adds, updates, and deletes individual
//! elements call by call, and every mutation lands as a state write the
//! console page streams — shapes appear on the open whiteboard as they
//! are drawn.
//!
//! Elements are stored verbatim inside the record's scene JSON. The worker
//! validates shape minimally (an object, a `type` string) and assigns ids;
//! the console pane runs the scene through excalidraw's own skeleton
//! conversion and restore, which are built to absorb partial shapes. That
//! keeps this worker free of any drawing library: it is a store with a
//! contract, not a renderer.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config::WorkerConfig;
use crate::store::{CanvasFormat, CanvasRecord, Store};

use super::create;

/// Spell a format for error text (the enum has no Display on purpose —
/// serde owns its wire spelling).
fn format_name(format: CanvasFormat) -> &'static str {
    match format {
        CanvasFormat::Mermaid => "mermaid",
        CanvasFormat::Freeform => "freeform",
    }
}

/// Hard per-canvas element cap — a runaway agent loop must not grow one
/// scene without bound (the record itself is also size-capped).
const MAX_ELEMENTS: usize = 2000;

pub const ADD_ID: &str = "canvas::element::add";
pub const ADD_DESC: &str =
    "Add elements to a freeform canvas, one call per drawing step. Each element is an \
     excalidraw-style object ({type, x, y, width?, height?, text?, label?, start?, end?, \
     ...}); skeleton shorthand is accepted and converted at render time. Elements without \
     an id get a stable generated one. The open console canvas streams every call, so \
     shapes appear as they are added. Returns the assigned ids and the new element count.";

pub const UPDATE_ID: &str = "canvas::element::update";
pub const UPDATE_DESC: &str =
    "Merge properties into one element of a freeform canvas by element id (move it, \
     recolor it, change its text). Unknown ids error and name the canvas. The open \
     console canvas streams the change live.";

pub const DELETE_ID: &str = "canvas::element::delete";
pub const DELETE_DESC: &str =
    "Remove elements from a freeform canvas by element id. Unknown ids are ignored; the \
     response reports how many were actually removed. The open console canvas streams \
     the change live.";

pub const LIST_ID: &str = "canvas::element::list";
pub const LIST_DESC: &str =
    "List the elements of a freeform canvas: id, type, position and size per element — \
     the map an agent reads before updating or connecting shapes. Full element bodies \
     are in the record source via canvas::get.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddRequest {
    /// Stable 8-character canvas id (format must be freeform).
    pub id: String,
    /// Elements to append, in z-order. Objects with at least a `type`
    /// string; unknown fields pass through to the scene untouched.
    pub elements: Vec<Value>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AddResponse {
    /// The canvas the elements were added to.
    pub id: String,
    /// Assigned element ids, in the order the elements were given.
    pub element_ids: Vec<String>,
    /// Total elements in the scene after the add.
    pub element_count: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateRequest {
    /// Stable 8-character canvas id (format must be freeform).
    pub id: String,
    /// Element id to update (from element::add or element::list).
    pub element_id: String,
    /// Properties to merge into the element. `null` values remove the key.
    pub props: Map<String, Value>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UpdateResponse {
    pub id: String,
    pub element_id: String,
    /// The element after the merge.
    pub element: Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteRequest {
    /// Stable 8-character canvas id (format must be freeform).
    pub id: String,
    /// Element ids to remove.
    pub element_ids: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DeleteResponse {
    pub id: String,
    /// How many of the given ids existed and were removed.
    pub removed: usize,
    /// Total elements left in the scene.
    pub element_count: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListRequest {
    /// Stable 8-character canvas id (format must be freeform).
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ElementSummary {
    pub id: String,
    /// Element type (`rectangle`, `ellipse`, `arrow`, `text`, ...).
    pub r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    /// The element's own `text`, or its `label.text` shorthand.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListResponse {
    pub id: String,
    pub elements: Vec<ElementSummary>,
}

/// Parse the record's scene JSON and hand back (scene object, elements).
/// A blank source becomes an empty scene, so element::add works on a
/// freshly created freeform canvas with no source yet.
fn scene_of(record: &CanvasRecord) -> Result<(Map<String, Value>, Vec<Value>), String> {
    if record.format != CanvasFormat::Freeform {
        return Err(format!(
            "canvas '{}' is a {} canvas — element operations work on freeform canvases; \
             edit mermaid canvases through canvas::update source",
            record.id,
            format_name(record.format)
        ));
    }
    let trimmed = record.source.trim();
    if trimmed.is_empty() {
        let mut scene = Map::new();
        scene.insert("type".into(), Value::String("excalidraw".into()));
        scene.insert("version".into(), Value::Number(2.into()));
        return Ok((scene, Vec::new()));
    }
    let value: Value = serde_json::from_str(trimmed)
        .map_err(|e| format!("canvas '{}' scene JSON does not parse: {e}", record.id))?;
    let Value::Object(mut scene) = value else {
        return Err(format!("canvas '{}' scene is not a JSON object", record.id));
    };
    let elements = match scene.remove("elements") {
        Some(Value::Array(items)) => items,
        Some(_) => {
            return Err(format!(
                "canvas '{}' scene has a non-array elements field",
                record.id
            ))
        }
        None => Vec::new(),
    };
    Ok((scene, elements))
}

/// Persist the mutated elements back into the record.
async fn save_scene(
    store: &Store,
    mut record: CanvasRecord,
    mut scene: Map<String, Value>,
    elements: Vec<Value>,
    cfg: &WorkerConfig,
) -> Result<CanvasRecord, String> {
    scene.insert("elements".into(), Value::Array(elements));
    let source = serde_json::to_string(&Value::Object(scene))
        .map_err(|e| format!("scene serialization failed: {e}"))?;
    create::check_source(&source, CanvasFormat::Freeform, cfg)?;
    record.source = source;
    record.updated_at = create::unix_now();
    store.save(&record).await?;
    Ok(record)
}

async fn load_freeform(store: &Store, id: &str) -> Result<CanvasRecord, String> {
    store
        .load(id)
        .await?
        .ok_or_else(|| format!("canvas '{id}' not found — canvas::list shows the stored ids"))
}

fn element_id(el: &Value) -> Option<&str> {
    el.get("id").and_then(Value::as_str)
}

pub async fn handle_add(
    store: &Store,
    req: AddRequest,
    cfg: &WorkerConfig,
) -> Result<AddResponse, String> {
    if req.elements.is_empty() {
        return Err("no elements given — pass at least one {type, ...} object".to_string());
    }
    let _guard = store.mutation_guard().await;
    let record = load_freeform(store, &req.id).await?;
    let (scene, mut elements) = scene_of(&record)?;
    if elements.len() + req.elements.len() > MAX_ELEMENTS {
        return Err(format!(
            "canvas '{}' would exceed {MAX_ELEMENTS} elements — split across canvases",
            req.id
        ));
    }
    let mut assigned = Vec::with_capacity(req.elements.len());
    for (i, el) in req.elements.into_iter().enumerate() {
        let Value::Object(mut obj) = el else {
            return Err(format!("element {i} is not an object"));
        };
        if !obj.get("type").map(Value::is_string).unwrap_or(false) {
            return Err(format!("element {i} has no `type` string"));
        }
        let id = match obj.get("id").and_then(Value::as_str) {
            Some(existing) if !existing.trim().is_empty() => existing.to_string(),
            _ => {
                let generated = create::random_id();
                obj.insert("id".into(), Value::String(generated.clone()));
                generated
            }
        };
        assigned.push(id);
        elements.push(Value::Object(obj));
    }
    let record = save_scene(store, record, scene, elements, cfg).await?;
    let (_, saved) = scene_of(&record)?;
    Ok(AddResponse {
        id: req.id,
        element_ids: assigned,
        element_count: saved.len(),
    })
}

pub async fn handle_update(
    store: &Store,
    req: UpdateRequest,
    cfg: &WorkerConfig,
) -> Result<UpdateResponse, String> {
    if req.props.is_empty() {
        return Err("props is empty — pass the properties to merge".to_string());
    }
    let _guard = store.mutation_guard().await;
    let record = load_freeform(store, &req.id).await?;
    let (scene, mut elements) = scene_of(&record)?;
    let Some(target) = elements
        .iter_mut()
        .find(|el| element_id(el) == Some(req.element_id.as_str()))
    else {
        return Err(format!(
            "element '{}' not found on canvas '{}' — canvas::element::list shows the ids",
            req.element_id, req.id
        ));
    };
    let Value::Object(obj) = target else {
        return Err(format!(
            "element '{}' on canvas '{}' is not an object",
            req.element_id, req.id
        ));
    };
    for (key, value) in req.props {
        if key == "id" {
            continue; // element ids are stable, like canvas ids
        }
        if value.is_null() {
            obj.remove(&key);
        } else {
            obj.insert(key, value);
        }
    }
    let updated = Value::Object(obj.clone());
    let record = save_scene(store, record, scene, elements, cfg).await?;
    Ok(UpdateResponse {
        id: record.id,
        element_id: req.element_id,
        element: updated,
    })
}

pub async fn handle_delete(
    store: &Store,
    req: DeleteRequest,
    cfg: &WorkerConfig,
) -> Result<DeleteResponse, String> {
    if req.element_ids.is_empty() {
        return Err("element_ids is empty — pass the ids to remove".to_string());
    }
    let _guard = store.mutation_guard().await;
    let record = load_freeform(store, &req.id).await?;
    let (scene, elements) = scene_of(&record)?;
    let before = elements.len();
    let keep: Vec<Value> = elements
        .into_iter()
        .filter(|el| {
            element_id(el)
                .map(|id| !req.element_ids.iter().any(|d| d == id))
                .unwrap_or(true)
        })
        .collect();
    let removed = before - keep.len();
    let element_count = keep.len();
    save_scene(store, record, scene, keep, cfg).await?;
    Ok(DeleteResponse {
        id: req.id,
        removed,
        element_count,
    })
}

pub async fn handle_list(
    store: &Store,
    req: ListRequest,
    _cfg: &WorkerConfig,
) -> Result<ListResponse, String> {
    let record = load_freeform(store, &req.id).await?;
    let (_, elements) = scene_of(&record)?;
    let summaries = elements
        .iter()
        .filter_map(|el| {
            let obj = el.as_object()?;
            Some(ElementSummary {
                id: obj.get("id")?.as_str()?.to_string(),
                r#type: obj
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                x: obj.get("x").and_then(Value::as_f64),
                y: obj.get("y").and_then(Value::as_f64),
                width: obj.get("width").and_then(Value::as_f64),
                height: obj.get("height").and_then(Value::as_f64),
                text: obj
                    .get("text")
                    .and_then(Value::as_str)
                    .or_else(|| {
                        obj.get("label")
                            .and_then(|l| l.get("text"))
                            .and_then(Value::as_str)
                    })
                    .map(str::to_string),
            })
        })
        .collect();
    Ok(ListResponse {
        id: req.id,
        elements: summaries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkerConfig;
    use serde_json::json;

    async fn freeform_store() -> (Store, String) {
        let store = Store::in_memory();
        let cfg = WorkerConfig::default();
        let record = create::handle(
            &store,
            create::Request {
                name: Some("board".into()),
                format: Some(CanvasFormat::Freeform),
                source: r#"{"type":"excalidraw","version":2,"elements":[]}"#.into(),
            },
            &cfg,
        )
        .await
        .unwrap();
        (store, record.id)
    }

    #[tokio::test]
    async fn add_assigns_ids_and_appends_in_order() {
        let (store, id) = freeform_store().await;
        let cfg = WorkerConfig::default();
        let out = handle_add(
            &store,
            AddRequest {
                id: id.clone(),
                elements: vec![
                    json!({"type": "rectangle", "x": 0, "y": 0}),
                    json!({"type": "ellipse", "x": 100, "y": 0, "id": "keepme"}),
                ],
            },
            &cfg,
        )
        .await
        .unwrap();
        assert_eq!(out.element_count, 2);
        assert_eq!(out.element_ids.len(), 2);
        assert_eq!(out.element_ids[1], "keepme");
        let listed = handle_list(&store, ListRequest { id }, &cfg).await.unwrap();
        assert_eq!(listed.elements[0].r#type, "rectangle");
        assert_eq!(listed.elements[1].id, "keepme");
    }

    #[tokio::test]
    async fn update_merges_and_null_removes() {
        let (store, id) = freeform_store().await;
        let cfg = WorkerConfig::default();
        let added = handle_add(
            &store,
            AddRequest {
                id: id.clone(),
                elements: vec![json!({"type": "rectangle", "x": 0, "y": 0, "strokeColor": "#f00"})],
            },
            &cfg,
        )
        .await
        .unwrap();
        let el = added.element_ids[0].clone();
        let mut props = Map::new();
        props.insert("x".into(), json!(50));
        props.insert("strokeColor".into(), Value::Null);
        props.insert("id".into(), json!("hijack"));
        let out = handle_update(
            &store,
            UpdateRequest {
                id: id.clone(),
                element_id: el.clone(),
                props,
            },
            &cfg,
        )
        .await
        .unwrap();
        assert_eq!(out.element.get("x"), Some(&json!(50)));
        assert!(out.element.get("strokeColor").is_none());
        assert_eq!(out.element.get("id"), Some(&json!(el.clone())));
        let missing = handle_update(
            &store,
            UpdateRequest {
                id,
                element_id: "nope1234".into(),
                props: Map::from_iter([("x".to_string(), json!(1))]),
            },
            &cfg,
        )
        .await;
        assert!(missing.is_err());
    }

    #[tokio::test]
    async fn delete_reports_removed_count() {
        let (store, id) = freeform_store().await;
        let cfg = WorkerConfig::default();
        let added = handle_add(
            &store,
            AddRequest {
                id: id.clone(),
                elements: vec![json!({"type": "rectangle"}), json!({"type": "ellipse"})],
            },
            &cfg,
        )
        .await
        .unwrap();
        let out = handle_delete(
            &store,
            DeleteRequest {
                id: id.clone(),
                element_ids: vec![added.element_ids[0].clone(), "unknown1".into()],
            },
            &cfg,
        )
        .await
        .unwrap();
        assert_eq!(out.removed, 1);
        assert_eq!(out.element_count, 1);
    }

    #[tokio::test]
    async fn element_ops_reject_mermaid_canvases() {
        let store = Store::in_memory();
        let cfg = WorkerConfig::default();
        let record = create::handle(
            &store,
            create::Request {
                name: None,
                format: None,
                source: "flowchart TD\n  A --> B".into(),
            },
            &cfg,
        )
        .await
        .unwrap();
        let err = handle_add(
            &store,
            AddRequest {
                id: record.id,
                elements: vec![json!({"type": "rectangle"})],
            },
            &cfg,
        )
        .await
        .unwrap_err();
        assert!(err.contains("freeform"), "got: {err}");
    }

    /// A record whose source lost its elements key (or is blank) still
    /// accepts adds — scene_of resynthesizes the envelope.
    #[test]
    fn scene_of_tolerates_blank_and_missing_elements() {
        let record = CanvasRecord {
            id: "abcd1234".into(),
            name: "b".into(),
            format: CanvasFormat::Freeform,
            source: String::new(),
            family: None,
            created_at: 0,
            updated_at: 0,
        };
        let (scene, elements) = scene_of(&record).unwrap();
        assert!(elements.is_empty());
        assert_eq!(scene.get("type"), Some(&json!("excalidraw")));
    }
}
