//! The index of one line and the algorithms over it: change classification
//! between two lines and the list filters. Pure; no I/O.

use std::collections::{BTreeMap, HashMap};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Which kind of source tree a line photographs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LineKind {
    /// The live working tree, rebuilt by the watcher. Key `worktree`.
    Worktree,
    /// The working tree as it was before the last rebuild. Key `worktree.prev`.
    Prev,
    /// A commit reached through a ref or a sha. Key: the commit sha.
    Ref,
    /// A chat turn's before/after tree from the ide worker. Key: the tree sha.
    Turn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LineInfo {
    /// Stable key: `worktree`, `worktree.prev`, or a git object sha.
    pub key: String,
    pub kind: LineKind,
    /// Human label: the ref name, the turn, or `working tree`.
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    /// Whether the working tree had uncommitted changes when built.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dirty: Option<bool>,
    /// RFC 3339.
    pub built_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Input {
    /// Workspace-relative path with `/` separators; absolute when the module
    /// lives outside the workspace.
    pub path: String,
    /// 0 = the story file, 1 = a direct import, n = n imports away.
    pub hop: u32,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Control {
    pub name: String,
    /// select | boolean | number | text | object | element | function | readonly
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Value>,
    #[serde(default)]
    pub default: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct State {
    /// Storybook id: `ui-button--primary`.
    pub id: String,
    pub name: String,
    pub export_name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Merged meta + story args with placeholders for functions and elements.
    #[serde(default)]
    pub args: Value,
    #[serde(default)]
    pub arg_types: Value,
    #[serde(default)]
    pub parameters: Value,
    #[serde(default)]
    pub controls: Vec<Control>,
    #[serde(default)]
    pub has_render: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Component {
    /// Sanitized title: `ui-button`. Unique per project.
    pub id: String,
    pub project: String,
    /// Story file, project-relative.
    pub file: String,
    /// Story file, workspace-relative.
    pub path: String,
    pub title: String,
    /// Title segments before the last one, joined with `/`.
    pub group: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// The component's display name when the meta names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    /// Hash of every input's content; equal versions render identically.
    pub version: String,
    /// Served path of the story file's html, relative to the line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(default)]
    pub states: Vec<State>,
    #[serde(default)]
    pub inputs: Vec<Input>,
    #[serde(default)]
    pub globals: Value,
    #[serde(default)]
    pub global_types: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectInfo {
    pub name: String,
    /// Workspace-relative folder.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vite: Option<Value>,
    pub files: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Index {
    pub workspace: String,
    pub line: LineInfo,
    #[serde(default)]
    pub projects: Vec<ProjectInfo>,
    #[serde(default)]
    pub components: Vec<Component>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl Index {
    pub fn find(&self, id: &str, project: Option<&str>) -> Vec<&Component> {
        self.components
            .iter()
            .filter(|c| c.id == id && project.is_none_or(|p| c.project == p))
            .collect()
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// The version of a component: the hash of every input's path and content.
pub fn version_of(inputs: &[Input]) -> String {
    let mut pairs: Vec<(&str, &str)> = inputs
        .iter()
        .map(|i| (i.path.as_str(), i.sha256.as_str()))
        .collect();
    pairs.sort();
    let mut hasher = Sha256::new();
    for (path, sha) in pairs {
        hasher.update(path.as_bytes());
        hasher.update(b":");
        hasher.update(sha.as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

/* ── compare ──────────────────────────────────────────────────────────── */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Unchanged,
    /// The story file or one of its direct imports changed.
    Direct,
    /// Only deeper inputs changed.
    Indirect,
    New,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChangedFile {
    pub path: String,
    /// Smallest hop at which the file reaches the component.
    pub hop: u32,
    /// added | removed | modified
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StateChange {
    pub id: String,
    pub name: String,
    /// added | removed | kept
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Change {
    pub id: String,
    pub project: String,
    pub title: String,
    pub file: String,
    pub kind: ChangeKind,
    #[serde(default)]
    pub files: Vec<ChangedFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b_version: Option<String>,
    #[serde(default)]
    pub states: Vec<StateChange>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChangeSummary {
    pub direct: usize,
    pub indirect: usize,
    pub new: usize,
    pub removed: usize,
    pub unchanged: usize,
}

pub fn summarize(changes: &[Change]) -> ChangeSummary {
    let mut summary = ChangeSummary::default();
    for change in changes {
        match change.kind {
            ChangeKind::Direct => summary.direct += 1,
            ChangeKind::Indirect => summary.indirect += 1,
            ChangeKind::New => summary.new += 1,
            ChangeKind::Removed => summary.removed += 1,
            ChangeKind::Unchanged => summary.unchanged += 1,
        }
    }
    summary
}

fn changed_files(a: &Component, b: &Component) -> Vec<ChangedFile> {
    let before: HashMap<&str, &Input> = a.inputs.iter().map(|i| (i.path.as_str(), i)).collect();
    let after: HashMap<&str, &Input> = b.inputs.iter().map(|i| (i.path.as_str(), i)).collect();
    let mut files: BTreeMap<String, ChangedFile> = BTreeMap::new();
    for (path, input) in &after {
        match before.get(path) {
            None => {
                files.insert(
                    path.to_string(),
                    ChangedFile {
                        path: path.to_string(),
                        hop: input.hop,
                        status: "added".into(),
                    },
                );
            }
            Some(prev) if prev.sha256 != input.sha256 => {
                files.insert(
                    path.to_string(),
                    ChangedFile {
                        path: path.to_string(),
                        hop: input.hop.min(prev.hop),
                        status: "modified".into(),
                    },
                );
            }
            Some(_) => {}
        }
    }
    for (path, input) in &before {
        if !after.contains_key(path) {
            files.insert(
                path.to_string(),
                ChangedFile {
                    path: path.to_string(),
                    hop: input.hop,
                    status: "removed".into(),
                },
            );
        }
    }
    let mut out: Vec<ChangedFile> = files.into_values().collect();
    out.sort_by(|x, y| x.hop.cmp(&y.hop).then_with(|| x.path.cmp(&y.path)));
    out
}

fn state_changes(a: &Component, b: &Component) -> Vec<StateChange> {
    let mut out = Vec::new();
    for state in &b.states {
        let status = if a.states.iter().any(|s| s.id == state.id) {
            "kept"
        } else {
            "added"
        };
        out.push(StateChange {
            id: state.id.clone(),
            name: state.name.clone(),
            status: status.into(),
        });
    }
    for state in &a.states {
        if !b.states.iter().any(|s| s.id == state.id) {
            out.push(StateChange {
                id: state.id.clone(),
                name: state.name.clone(),
                status: "removed".into(),
            });
        }
    }
    out
}

fn key(component: &Component) -> (String, String) {
    (component.project.clone(), component.id.clone())
}

/// Every component of `b` classified against `a`, plus the components only
/// `a` had. Direct when the story file or a hop-1 input differs; indirect
/// when only deeper inputs differ.
pub fn compare(a: &Index, b: &Index) -> Vec<Change> {
    let before: HashMap<(String, String), &Component> =
        a.components.iter().map(|c| (key(c), c)).collect();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for component in &b.components {
        let k = key(component);
        seen.insert(k.clone());
        let base = Change {
            id: component.id.clone(),
            project: component.project.clone(),
            title: component.title.clone(),
            file: component.path.clone(),
            kind: ChangeKind::New,
            files: Vec::new(),
            a_version: None,
            b_version: Some(component.version.clone()),
            states: component
                .states
                .iter()
                .map(|s| StateChange {
                    id: s.id.clone(),
                    name: s.name.clone(),
                    status: "added".into(),
                })
                .collect(),
        };
        match before.get(&k) {
            None => out.push(base),
            Some(prev) => {
                let files = changed_files(prev, component);
                let kind = if prev.version == component.version && files.is_empty() {
                    ChangeKind::Unchanged
                } else if files.iter().any(|f| f.hop <= 1) || files.is_empty() {
                    ChangeKind::Direct
                } else {
                    ChangeKind::Indirect
                };
                out.push(Change {
                    kind,
                    files,
                    a_version: Some(prev.version.clone()),
                    states: state_changes(prev, component),
                    ..base
                });
            }
        }
    }
    for component in &a.components {
        if seen.contains(&key(component)) {
            continue;
        }
        out.push(Change {
            id: component.id.clone(),
            project: component.project.clone(),
            title: component.title.clone(),
            file: component.path.clone(),
            kind: ChangeKind::Removed,
            files: Vec::new(),
            a_version: Some(component.version.clone()),
            b_version: None,
            states: component
                .states
                .iter()
                .map(|s| StateChange {
                    id: s.id.clone(),
                    name: s.name.clone(),
                    status: "removed".into(),
                })
                .collect(),
        });
    }
    out.sort_by(|x, y| {
        x.project
            .cmp(&y.project)
            .then_with(|| x.title.cmp(&y.title))
    });
    out
}

/* ── filters ──────────────────────────────────────────────────────────── */

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ListFilter {
    /// Project name or workspace-relative folder.
    #[serde(default)]
    pub project: Option<String>,
    /// Title group (`UI` for `UI/Button`); matches the group or a prefix of it.
    #[serde(default)]
    pub group: Option<String>,
    /// Every listed tag must be on the component or one of its states.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Story file path prefix or substring, workspace-relative.
    #[serde(default)]
    pub path: Option<String>,
    /// Case-insensitive substring over title, id, file and component name.
    #[serde(default)]
    pub query: Option<String>,
    /// direct | indirect | any | new | removed | unchanged. Requires a base line.
    #[serde(default)]
    pub change: Option<String>,
}

pub fn matches_filter(component: &Component, filter: &ListFilter) -> bool {
    if let Some(project) = filter
        .project
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        && component.project != project
        && !component
            .path
            .starts_with(&format!("{}/", project.trim_end_matches('/')))
    {
        return false;
    }
    if let Some(group) = filter
        .group
        .as_deref()
        .map(str::trim)
        .filter(|g| !g.is_empty())
    {
        let wanted = group.trim_matches('/').to_ascii_lowercase();
        let have = component.group.to_ascii_lowercase();
        if have != wanted && !have.starts_with(&format!("{wanted}/")) {
            return false;
        }
    }
    if let Some(tags) = &filter.tags {
        for tag in tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
            let on_component = component.tags.iter().any(|t| t == tag);
            let on_state = component
                .states
                .iter()
                .any(|s| s.tags.iter().any(|t| t == tag));
            if !on_component && !on_state {
                return false;
            }
        }
    }
    if let Some(path) = filter
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        && !component.path.starts_with(path)
        && !component.path.contains(path)
    {
        return false;
    }
    if let Some(query) = filter
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
    {
        let q = query.to_ascii_lowercase();
        let hay = format!(
            "{} {} {} {}",
            component.title,
            component.id,
            component.path,
            component.component.clone().unwrap_or_default()
        )
        .to_ascii_lowercase();
        if !hay.contains(&q) {
            return false;
        }
    }
    true
}

pub fn matches_change(kind: Option<ChangeKind>, wanted: Option<&str>) -> bool {
    let Some(wanted) = wanted.map(str::trim).filter(|w| !w.is_empty()) else {
        return true;
    };
    match (wanted, kind) {
        ("any", Some(k)) => k != ChangeKind::Unchanged,
        ("any", None) => false,
        ("direct", Some(ChangeKind::Direct)) => true,
        ("indirect", Some(ChangeKind::Indirect)) => true,
        ("new", Some(ChangeKind::New)) => true,
        ("removed", Some(ChangeKind::Removed)) => true,
        ("unchanged", Some(ChangeKind::Unchanged)) | ("unchanged", None) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(path: &str, hop: u32, sha: &str) -> Input {
        Input {
            path: path.into(),
            hop,
            sha256: sha.into(),
        }
    }

    fn component(id: &str, inputs: Vec<Input>, states: &[&str]) -> Component {
        Component {
            id: id.into(),
            project: "app".into(),
            file: format!("src/{id}.stories.tsx"),
            path: format!("app/src/{id}.stories.tsx"),
            title: format!("UI/{id}"),
            group: "UI".into(),
            tags: vec!["core".into()],
            component: None,
            version: version_of(&inputs),
            html: None,
            states: states
                .iter()
                .map(|s| State {
                    id: format!("ui-{id}--{s}"),
                    name: s.to_string(),
                    export_name: s.to_string(),
                    tags: vec![],
                    args: Value::Null,
                    arg_types: Value::Null,
                    parameters: Value::Null,
                    controls: vec![],
                    has_render: false,
                })
                .collect(),
            inputs,
            globals: Value::Null,
            global_types: Value::Null,
            error: None,
        }
    }

    fn index(components: Vec<Component>) -> Index {
        Index {
            workspace: "ws".into(),
            line: LineInfo {
                key: "worktree".into(),
                kind: LineKind::Worktree,
                label: "working tree".into(),
                sha: None,
                dirty: None,
                built_at: "2026-01-01T00:00:00Z".into(),
            },
            projects: vec![],
            components,
            warnings: vec![],
        }
    }

    #[test]
    fn version_ignores_input_order() {
        let a = version_of(&[input("a", 0, "1"), input("b", 1, "2")]);
        let b = version_of(&[input("b", 1, "2"), input("a", 0, "1")]);
        assert_eq!(a, b);
        assert_ne!(a, version_of(&[input("a", 0, "1"), input("b", 1, "3")]));
    }

    #[test]
    fn compare_classifies_direct_indirect_new_and_removed() {
        let before = index(vec![
            component(
                "button",
                vec![
                    input("app/src/button.stories.tsx", 0, "s"),
                    input("app/src/Button.tsx", 1, "b"),
                    input("app/src/cn.ts", 2, "c"),
                ],
                &["primary"],
            ),
            component(
                "card",
                vec![
                    input("app/src/card.stories.tsx", 0, "s"),
                    input("app/src/Card.tsx", 1, "k"),
                    input("app/src/cn.ts", 2, "c"),
                ],
                &["ok"],
            ),
            component(
                "gone",
                vec![input("app/src/gone.stories.tsx", 0, "g")],
                &["x"],
            ),
            component(
                "same",
                vec![input("app/src/same.stories.tsx", 0, "z")],
                &["x"],
            ),
        ]);
        let after = index(vec![
            component(
                "button",
                vec![
                    input("app/src/button.stories.tsx", 0, "s"),
                    input("app/src/Button.tsx", 1, "b2"),
                    input("app/src/cn.ts", 2, "c"),
                ],
                &["primary", "ghost"],
            ),
            component(
                "card",
                vec![
                    input("app/src/card.stories.tsx", 0, "s"),
                    input("app/src/Card.tsx", 1, "k"),
                    input("app/src/cn.ts", 2, "c2"),
                ],
                &["ok"],
            ),
            component(
                "fresh",
                vec![input("app/src/fresh.stories.tsx", 0, "f")],
                &["x"],
            ),
            component(
                "same",
                vec![input("app/src/same.stories.tsx", 0, "z")],
                &["x"],
            ),
        ]);
        let changes = compare(&before, &after);
        let by_id: HashMap<&str, &Change> = changes.iter().map(|c| (c.id.as_str(), c)).collect();
        assert_eq!(by_id["button"].kind, ChangeKind::Direct);
        assert_eq!(by_id["button"].files[0].path, "app/src/Button.tsx");
        assert_eq!(
            by_id["button"]
                .states
                .iter()
                .find(|s| s.name == "ghost")
                .unwrap()
                .status,
            "added"
        );
        assert_eq!(by_id["card"].kind, ChangeKind::Indirect);
        assert_eq!(by_id["card"].files[0].hop, 2);
        assert_eq!(by_id["fresh"].kind, ChangeKind::New);
        assert_eq!(by_id["gone"].kind, ChangeKind::Removed);
        assert_eq!(by_id["same"].kind, ChangeKind::Unchanged);
        let summary = summarize(&changes);
        assert_eq!(
            (
                summary.direct,
                summary.indirect,
                summary.new,
                summary.removed,
                summary.unchanged
            ),
            (1, 1, 1, 1, 1)
        );
    }

    #[test]
    fn filters_match_group_tags_path_and_query() {
        let c = component(
            "button",
            vec![input("app/src/button.stories.tsx", 0, "s")],
            &["primary"],
        );
        let f = |json: Value| serde_json::from_value::<ListFilter>(json).unwrap();
        assert!(matches_filter(&c, &f(serde_json::json!({ "group": "ui" }))));
        assert!(!matches_filter(
            &c,
            &f(serde_json::json!({ "group": "forms" }))
        ));
        assert!(matches_filter(
            &c,
            &f(serde_json::json!({ "tags": ["core"] }))
        ));
        assert!(!matches_filter(
            &c,
            &f(serde_json::json!({ "tags": ["core", "nope"] }))
        ));
        assert!(matches_filter(
            &c,
            &f(serde_json::json!({ "path": "app/src" }))
        ));
        assert!(matches_filter(
            &c,
            &f(serde_json::json!({ "project": "app" }))
        ));
        assert!(!matches_filter(
            &c,
            &f(serde_json::json!({ "project": "admin" }))
        ));
        assert!(matches_filter(
            &c,
            &f(serde_json::json!({ "query": "BUTTON" }))
        ));
        assert!(matches_change(Some(ChangeKind::Direct), Some("any")));
        assert!(!matches_change(Some(ChangeKind::Unchanged), Some("any")));
        assert!(matches_change(None, Some("unchanged")));
        assert!(matches_change(None, None));
    }
}
