//! The `stories` configuration entry: which workspaces to index, where the
//! data folder lives, how to reach the console for headless renders, and
//! the default viewport. Stored in the `configuration` worker; every read
//! goes through [`normalize`], which repairs hostile input instead of
//! refusing it.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub type ConfigCell = Arc<RwLock<StoriesConfig>>;

pub const CONFIG_ID: &str = "stories";
pub const CONFIG_NAME: &str = "Stories";
pub const CONFIG_DESCRIPTION: &str = "Which workspaces hold story files, where snapshots are kept, and how headless renders reach the console.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectConfig {
    /// Display name; defaults to the package name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Folder relative to the workspace.
    pub path: String,
    /// Preview file (decorators, parameters, globals). Auto-detected at
    /// `.storybook/preview.*` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    /// Vite config file to build with. Auto-detected when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceConfig {
    /// Slug used in function calls, urls and the data folder.
    pub name: String,
    /// Repository root, relative to the project directory or absolute.
    pub path: String,
    /// Projects to build. Empty: discovered from the story files.
    #[serde(default)]
    pub projects: Vec<ProjectConfig>,
    /// Story file globs, relative to each project.
    #[serde(default)]
    pub stories: Vec<String>,
    /// Extra directory names or globs to skip while discovering.
    #[serde(default)]
    pub ignore: Vec<String>,
    /// Git ref the explorer compares the working tree against by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    /// Device scale factor.
    pub dpr: u32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            width: 1024,
            height: 768,
            dpr: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoriesConfig {
    /// Folder holding lines, files, renders and the compiler. Relative paths
    /// resolve from the Compose project directory or the process directory.
    pub data_path: String,
    /// Console origin the browser worker navigates to for headless renders.
    pub console_url: String,
    /// Rebuild the working tree line when a story input changes.
    pub watch: bool,
    /// Built ref lines kept per workspace before the oldest are pruned.
    pub keep_lines: u32,
    pub viewport: Viewport,
    pub workspaces: Vec<WorkspaceConfig>,
}

impl Default for StoriesConfig {
    fn default() -> Self {
        Self {
            data_path: iii_worker_paths::default_path("data/stories"),
            console_url: "http://127.0.0.1:3113".to_string(),
            watch: true,
            keep_lines: 12,
            viewport: Viewport::default(),
            workspaces: vec![WorkspaceConfig {
                name: "workspace".to_string(),
                path: ".".to_string(),
                projects: Vec::new(),
                stories: Vec::new(),
                ignore: Vec::new(),
                base: None,
            }],
        }
    }
}

impl StoriesConfig {
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("config serializes")
    }

    pub fn data_path_resolved(&self) -> PathBuf {
        iii_worker_paths::resolve_path(&self.data_path)
    }

    pub fn workspace(&self, name: Option<&str>) -> Option<&WorkspaceConfig> {
        match name {
            Some(name) => self.workspaces.iter().find(|w| w.name == name),
            None => self.workspaces.first(),
        }
    }

    /// The default base line of a workspace: its configured `base`, else `HEAD`.
    pub fn base_of(&self, workspace: &WorkspaceConfig) -> String {
        workspace
            .base
            .clone()
            .filter(|b| !b.trim().is_empty())
            .unwrap_or_else(|| "HEAD".to_string())
    }
}

impl WorkspaceConfig {
    pub fn path_resolved(&self) -> PathBuf {
        iii_worker_paths::resolve_path(&self.path)
    }
}

pub fn schema() -> Value {
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "additionalProperties": false,
        "required": ["data_path", "workspaces"],
        "properties": {
            "data_path": { "type": "string", "minLength": 1, "description": "Folder holding lines, files, renders and the compiler. Relative paths resolve from the project root." },
            "console_url": { "type": "string", "description": "Console origin the browser worker navigates to for headless renders." },
            "watch": { "type": "boolean", "description": "Rebuild the working tree line when a story input changes." },
            "keep_lines": { "type": "integer", "minimum": 1, "description": "Built ref lines kept per workspace before the oldest are pruned." },
            "viewport": {
                "type": "object", "additionalProperties": false,
                "properties": {
                    "width": { "type": "integer", "minimum": 200 },
                    "height": { "type": "integer", "minimum": 200 },
                    "dpr": { "type": "integer", "minimum": 1, "maximum": 3 }
                }
            },
            "workspaces": {
                "type": "array", "minItems": 1,
                "description": "Repositories to index.",
                "items": {
                    "type": "object", "additionalProperties": false, "required": ["name", "path"],
                    "properties": {
                        "name": { "type": "string", "pattern": "^[a-z0-9][a-z0-9_-]*$", "description": "Slug used in function calls, urls and the data folder." },
                        "path": { "type": "string", "minLength": 1, "description": "Repository root, relative to the project directory or absolute." },
                        "projects": {
                            "type": "array",
                            "description": "Projects to build. Empty: discovered from the story files.",
                            "items": {
                                "type": "object", "additionalProperties": false, "required": ["path"],
                                "properties": {
                                    "name": { "type": "string" },
                                    "path": { "type": "string", "minLength": 1 },
                                    "preview": { "type": "string" },
                                    "config": { "type": "string" }
                                }
                            }
                        },
                        "stories": { "type": "array", "items": { "type": "string" }, "description": "Story file globs, relative to each project." },
                        "ignore": { "type": "array", "items": { "type": "string" }, "description": "Extra directory names or globs skipped while discovering." },
                        "base": { "type": "string", "description": "Git ref the explorer compares the working tree against by default (HEAD)." }
                    }
                }
            }
        }
    })
}

fn clean(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

fn slug(value: &str, fallback: &str) -> String {
    let mut out = String::new();
    for c in value.trim().to_ascii_lowercase().chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        fallback.to_string()
    } else {
        out
    }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| clean(Some(item)))
                .filter(|item| !item.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Repair whatever is stored into a usable config: empty paths fall back to
/// the defaults, workspace names become slugs and deduplicate, and an empty
/// workspace list becomes the single default workspace at the project root.
pub fn normalize(raw: &Value) -> StoriesConfig {
    let defaults = StoriesConfig::default();
    let field = |key: &str| raw.get(key);

    let mut workspaces: Vec<WorkspaceConfig> = Vec::new();
    if let Some(entries) = field("workspaces").and_then(Value::as_array) {
        for (i, entry) in entries.iter().enumerate() {
            let path = clean(entry.get("path"));
            if path.is_empty() {
                continue;
            }
            let name = slug(&clean(entry.get("name")), &format!("workspace-{}", i + 1));
            if workspaces.iter().any(|w| w.name == name) {
                continue;
            }
            let projects = entry
                .get("projects")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            let path = clean(item.get("path"));
                            (!path.is_empty()).then(|| ProjectConfig {
                                name: Some(clean(item.get("name"))).filter(|n| !n.is_empty()),
                                path,
                                preview: Some(clean(item.get("preview"))).filter(|n| !n.is_empty()),
                                config: Some(clean(item.get("config"))).filter(|n| !n.is_empty()),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            workspaces.push(WorkspaceConfig {
                name,
                path,
                projects,
                stories: strings(entry.get("stories")),
                ignore: strings(entry.get("ignore")),
                base: Some(clean(entry.get("base"))).filter(|b| !b.is_empty()),
            });
        }
    }
    if workspaces.is_empty() {
        workspaces = defaults.workspaces.clone();
    }

    let viewport = field("viewport")
        .map(|v| Viewport {
            width: v
                .get("width")
                .and_then(Value::as_u64)
                .unwrap_or(1024)
                .clamp(200, 8192) as u32,
            height: v
                .get("height")
                .and_then(Value::as_u64)
                .unwrap_or(768)
                .clamp(200, 8192) as u32,
            dpr: v
                .get("dpr")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 3) as u32,
        })
        .unwrap_or_default();

    let data_path = clean(field("data_path"));
    let console_url = clean(field("console_url"));
    StoriesConfig {
        data_path: if data_path.is_empty() {
            defaults.data_path
        } else {
            data_path
        },
        console_url: if console_url.is_empty() {
            defaults.console_url
        } else {
            console_url.trim_end_matches('/').to_string()
        },
        watch: field("watch").and_then(Value::as_bool).unwrap_or(true),
        keep_lines: field("keep_lines")
            .and_then(Value::as_u64)
            .unwrap_or(12)
            .clamp(1, 1000) as u32,
        viewport,
        workspaces,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let defaults = StoriesConfig::default();
        assert_eq!(normalize(&defaults.to_json()), defaults);
        assert_eq!(normalize(&json!({})).workspaces.len(), 1);
    }

    #[test]
    fn normalize_repairs_hostile_input() {
        let repaired = normalize(&json!({
            "data_path": "  ./data/x  ",
            "console_url": "http://localhost:4000/",
            "keep_lines": 0,
            "viewport": { "width": 10, "dpr": 9 },
            "workspaces": [
                { "name": "My Repo!", "path": "repo", "projects": [{ "path": "app" }, { "name": "x" }], "stories": ["src/**/*.stories.tsx", ""] },
                { "name": "my-repo", "path": "dup" },
                { "path": "" }
            ]
        }));
        assert_eq!(repaired.data_path, "./data/x");
        assert_eq!(repaired.console_url, "http://localhost:4000");
        assert_eq!(repaired.keep_lines, 1);
        assert_eq!(repaired.viewport.width, 200);
        assert_eq!(repaired.viewport.dpr, 3);
        assert_eq!(repaired.workspaces.len(), 1);
        assert_eq!(repaired.workspaces[0].name, "my-repo");
        assert_eq!(repaired.workspaces[0].projects.len(), 1);
        assert_eq!(repaired.workspaces[0].stories, vec!["src/**/*.stories.tsx"]);
    }

    #[test]
    fn defaults_never_serialize_null_for_optional_strings() {
        let json = StoriesConfig::default().to_json();
        let workspace = &json["workspaces"][0];
        assert!(workspace.get("base").is_none(), "{workspace}");
        let project = serde_json::to_value(ProjectConfig {
            name: None,
            path: "app".into(),
            preview: None,
            config: None,
        })
        .unwrap();
        assert_eq!(project, json!({ "path": "app" }));
    }

    #[test]
    fn schema_is_an_object_schema() {
        let schema = schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"][1], "workspaces");
    }
}
