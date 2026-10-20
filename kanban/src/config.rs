//! The `kanban` configuration entry: where the board file lives, how tickets
//! are numbered, which columns and priorities the board offers, and the
//! fallback agents folder. Stored in the `configuration` worker; every read
//! goes through [`normalize`], which repairs hostile input instead of
//! refusing it so a half-edited YAML file can never take the board down.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const CONFIG_ID: &str = "kanban";
pub const CONFIG_NAME: &str = "Kanban";
pub const CONFIG_DESCRIPTION: &str = "Where the board file lives, how tickets are numbered, which columns the board has, and which one counts as done.";
pub const DEFAULT_PRIORITIES: [&str; 4] = ["low", "medium", "high", "urgent"];
const DEFAULT_PREFIX: &str = "KAN";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Column {
    /// Stable column id stored on tickets.
    pub id: String,
    /// Name shown on the board.
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KanbanConfig {
    /// Folder holding `board.json`. Relative paths resolve from the Compose
    /// project directory (`III_COMPOSE_DIR`) or the process directory.
    pub data_path: String,
    /// Prefix for human-readable ticket keys, e.g. `KAN` -> `KAN-1`.
    pub id_prefix: String,
    /// Column id a new ticket lands in when none is given.
    pub default_status: String,
    /// Column id that counts as done: progress, release status, shipping and
    /// the subtask warning use only this column. Defaults to the last column
    /// that is not the canceled one.
    pub done_status: String,
    /// Column id that counts as canceled: its tickets leave progress, project
    /// and release totals, and never hold a release open. `None` when the
    /// board has no canceled column.
    #[serde(default)]
    pub canceled_status: Option<String>,
    /// Board columns, left to right.
    pub columns: Vec<Column>,
    /// Priority values offered on a ticket, lowest first.
    pub priorities: Vec<String>,
    /// Fallback folder of agent profiles used when iii-directory is unavailable.
    pub agents_path: String,
}

pub fn default_columns() -> Vec<Column> {
    [
        ("backlog", "Backlog"),
        ("todo", "To do"),
        ("in_progress", "In progress"),
        ("in_review", "In review"),
        ("done", "Done"),
        ("canceled", "Canceled"),
    ]
    .into_iter()
    .map(|(id, label)| Column {
        id: id.to_string(),
        label: label.to_string(),
    })
    .collect()
}

fn default_priorities() -> Vec<String> {
    DEFAULT_PRIORITIES.iter().map(|p| p.to_string()).collect()
}

impl Default for KanbanConfig {
    fn default() -> Self {
        Self {
            data_path: iii_worker_paths::default_path(format!(
                "data/{}",
                crate::namespace::worker_name()
            )),
            id_prefix: DEFAULT_PREFIX.to_string(),
            default_status: "backlog".to_string(),
            done_status: "done".to_string(),
            canceled_status: Some("canceled".to_string()),
            columns: default_columns(),
            priorities: default_priorities(),
            agents_path: iii_worker_paths::default_path("agents"),
        }
    }
}

impl KanbanConfig {
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("config serializes")
    }

    pub fn data_path_resolved(&self) -> PathBuf {
        iii_worker_paths::resolve_path(&self.data_path)
    }

    pub fn board_file(&self) -> PathBuf {
        self.data_path_resolved().join("board.json")
    }

    pub fn agents_path_resolved(&self) -> PathBuf {
        iii_worker_paths::resolve_path(&self.agents_path)
    }

    pub fn has_column(&self, id: &str) -> bool {
        self.columns.iter().any(|column| column.id == id)
    }
}

/// The Compose project directory when supervised, else the process directory.
pub fn project_root() -> PathBuf {
    std::env::var_os(iii_worker_paths::COMPOSE_DIR_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default()
}

/// JSON Schema registered with the `configuration` worker (what the console
/// validates edits against).
pub fn schema() -> Value {
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "additionalProperties": false,
        "required": ["data_path", "id_prefix", "default_status", "columns"],
        "properties": {
            "data_path": {
                "type": "string",
                "minLength": 1,
                "description": "Folder holding the board file (board.json). Relative paths resolve from the project root."
            },
            "id_prefix": {
                "type": "string",
                "pattern": "^[A-Z][A-Z0-9]*$",
                "description": "Prefix for human-readable ticket keys, e.g. KAN -> KAN-1."
            },
            "default_status": {
                "type": "string",
                "description": "Column id a new ticket lands in when none is given."
            },
            "done_status": {
                "type": "string",
                "description": "Column id that counts as done (progress, releases, shipping, the subtask warning). Defaults to the last column that is not the canceled one; an unknown id falls back to it."
            },
            "canceled_status": {
                "type": ["string", "null"],
                "description": "Column id that counts as canceled: its tickets leave progress, project and release totals. Defaults to `canceled` when the board has that column; null, an unknown id or the done column means no canceled column."
            },
            "columns": {
                "type": "array",
                "minItems": 1,
                "description": "Board columns, left to right.",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "label"],
                    "properties": {
                        "id": { "type": "string", "minLength": 1, "description": "Stable column id stored on tickets." },
                        "label": { "type": "string", "minLength": 1, "description": "Name shown on the board." }
                    }
                }
            },
            "priorities": {
                "type": "array",
                "minItems": 1,
                "items": { "type": "string", "minLength": 1 },
                "description": "Priority values offered on a ticket, lowest first."
            },
            "agents_path": {
                "type": "string",
                "minLength": 1,
                "description": "Fallback folder of agent profiles used when iii-directory is unavailable."
            }
        }
    })
}

fn clean_string(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

fn valid_prefix(prefix: &str) -> bool {
    let mut chars = prefix.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// Repair whatever is stored into a usable config: unknown default column
/// falls to the first column, unknown done column to the last one that is not
/// the canceled one, an unknown canceled column to none, an invalid
/// prefix falls back to `KAN`, empty lists fall back to the defaults,
/// duplicate column ids collapse.
pub fn normalize(raw: &Value) -> KanbanConfig {
    let defaults = KanbanConfig::default();
    let input = raw.as_object();
    let field = |key: &str| input.and_then(|object| object.get(key));

    let mut columns: Vec<Column> = Vec::new();
    if let Some(entries) = field("columns").and_then(Value::as_array) {
        for entry in entries {
            let id = clean_string(entry.get("id"));
            if id.is_empty() || columns.iter().any(|column| column.id == id) {
                continue;
            }
            let label = clean_string(entry.get("label"));
            columns.push(Column {
                label: if label.is_empty() { id.clone() } else { label },
                id,
            });
        }
    }
    if columns.is_empty() {
        columns = default_columns();
    }

    let prefix = clean_string(field("id_prefix")).to_ascii_uppercase();
    let id_prefix = if valid_prefix(&prefix) {
        prefix
    } else {
        DEFAULT_PREFIX.to_string()
    };

    let requested_default = clean_string(field("default_status"));
    let default_status = if columns.iter().any(|column| column.id == requested_default) {
        requested_default
    } else {
        columns[0].id.clone()
    };

    // A missing key means the default `canceled` column; an explicit null or
    // empty string means none.
    let requested_canceled = match field("canceled_status") {
        None => "canceled".to_string(),
        Some(value) => clean_string(Some(value)),
    };
    let requested_done = clean_string(field("done_status"));
    let done_status = if columns.iter().any(|column| column.id == requested_done) {
        requested_done
    } else {
        columns
            .iter()
            .rev()
            .find(|column| column.id != requested_canceled)
            .unwrap_or(&columns[columns.len() - 1])
            .id
            .clone()
    };
    let canceled_status = Some(requested_canceled)
        .filter(|id| *id != done_status && columns.iter().any(|column| column.id == *id));

    let priorities: Vec<String> = field("priorities")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .map(|entry| clean_string(Some(entry)))
                .filter(|entry| !entry.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let data_path = clean_string(field("data_path"));
    let agents_path = clean_string(field("agents_path"));

    KanbanConfig {
        data_path: if data_path.is_empty() {
            defaults.data_path
        } else {
            data_path
        },
        id_prefix,
        default_status,
        done_status,
        canceled_status,
        columns,
        priorities: if priorities.is_empty() {
            default_priorities()
        } else {
            priorities
        },
        agents_path: if agents_path.is_empty() {
            defaults.agents_path
        } else {
            agents_path
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_through_normalize() {
        let defaults = KanbanConfig::default();
        assert_eq!(normalize(&defaults.to_json()), defaults);
        assert_eq!(normalize(&json!({})).columns.len(), 6);
    }

    #[test]
    fn normalize_repairs_hostile_input_and_preserves_good_columns() {
        let repaired = normalize(&json!({
            "data_path": "  ./data/board  ",
            "id_prefix": "bad prefix!",
            "default_status": "nope",
            "columns": [
                { "id": "open", "label": "Open" },
                { "id": "open", "label": "Dup" },
                "junk",
                { "id": "closed" }
            ]
        }));
        assert_eq!(repaired.data_path, "./data/board");
        assert_eq!(repaired.id_prefix, "KAN", "an invalid prefix falls back");
        assert_eq!(
            repaired.columns,
            vec![
                Column {
                    id: "open".into(),
                    label: "Open".into()
                },
                Column {
                    id: "closed".into(),
                    label: "closed".into()
                },
            ]
        );
        assert_eq!(
            repaired.default_status, "open",
            "an unknown default falls to the first column"
        );
        assert_eq!(
            repaired.done_status, "closed",
            "an absent done column falls to the last column"
        );
        assert_eq!(repaired.priorities, default_priorities());
    }

    #[test]
    fn done_status_is_kept_when_it_is_a_column_and_repaired_otherwise() {
        let columns = json!([{ "id": "a" }, { "id": "b" }, { "id": "c" }]);
        let kept = normalize(&json!({ "columns": columns, "done_status": " b " }));
        assert_eq!(kept.done_status, "b");
        let unknown = normalize(&json!({ "columns": columns, "done_status": "nope" }));
        assert_eq!(unknown.done_status, "c");
        assert_eq!(normalize(&json!({})).done_status, "done");
        assert_eq!(schema()["properties"]["done_status"]["type"], "string");
    }

    #[test]
    fn canceled_status_defaults_to_the_canceled_column_and_never_takes_done() {
        let empty = normalize(&json!({}));
        assert_eq!(
            empty.done_status, "done",
            "the canceled column is never the done fallback"
        );
        assert_eq!(empty.canceled_status.as_deref(), Some("canceled"));
        let columns = json!([{ "id": "todo" }, { "id": "done" }, { "id": "canceled" }]);
        let missing = normalize(&json!({ "columns": columns }));
        assert_eq!(missing.done_status, "done");
        assert_eq!(missing.canceled_status.as_deref(), Some("canceled"));
        let none = normalize(&json!({ "columns": columns, "canceled_status": null }));
        assert_eq!(
            none.canceled_status, None,
            "an explicit null means no canceled column"
        );
        assert_eq!(
            none.done_status, "canceled",
            "with none, the last column is done again"
        );
        let renamed = normalize(&json!({
            "columns": [{ "id": "todo" }, { "id": "done" }, { "id": "dropped" }],
            "canceled_status": "dropped",
        }));
        assert_eq!(renamed.canceled_status.as_deref(), Some("dropped"));
        assert_eq!(renamed.done_status, "done");
        let unknown = normalize(&json!({ "columns": columns, "canceled_status": "nope" }));
        assert_eq!(unknown.canceled_status, None);
        let clash = normalize(&json!({
            "columns": columns,
            "done_status": "canceled",
            "canceled_status": "canceled",
        }));
        assert_eq!(clash.done_status, "canceled");
        assert_eq!(clash.canceled_status, None, "done wins a clash");
        let legacy = normalize(&json!({ "columns": [{ "id": "todo" }, { "id": "done" }] }));
        assert_eq!(
            legacy.canceled_status, None,
            "a stored board without the column keeps none"
        );
        assert_eq!(legacy.done_status, "done");
    }

    #[test]
    fn lowercase_prefix_is_upcased_and_paths_keep_absolute_form() {
        let config = normalize(&json!({ "id_prefix": "proj2", "data_path": "/srv/board" }));
        assert_eq!(config.id_prefix, "PROJ2");
        assert_eq!(config.board_file(), PathBuf::from("/srv/board/board.json"));
    }

    #[test]
    fn schema_is_an_object_schema_with_the_required_fields() {
        let schema = schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"][0], "data_path");
        assert_eq!(
            schema["properties"]["id_prefix"]["pattern"],
            "^[A-Z][A-Z0-9]*$"
        );
    }
}
