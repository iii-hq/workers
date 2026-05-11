//! `skills::roots::*` — multi-root SKILL.md discovery.
//!
//! Existing fs_source.rs walks user-configured glob patterns rooted at the
//! config directory. This module adds a complementary surface for the
//! common case where an agent wants to enumerate SKILL.md files from
//! conventional XDG-style locations without writing any config.
//!
//! Default roots (overridable in payload):
//!
//!   - `./skills`            (project-local)
//!   - `~/.iii/skills`       (per-user)
//!   - `~/.claude/skills`    (interop with Claude Code)
//!   - `~/.codex/skills`     (interop with Codex)
//!
//! Functions registered:
//!
//!   - `skills::roots::index` — slim list `{id, title, description, path}[]`
//!     across all configured roots. Frontmatter only — no body in the
//!     response.
//!   - `skills::roots::load`  — read full body for one discovered id.
//!   - `skills::roots::rescan` — force-rewalk and return new count.
//!
//! Pure additions: nothing else in the `skills` worker is touched. The
//! existing state-backed `skills::*` CRUD surface continues to own the
//! iii-state-resident registry. This module owns the on-disk side.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunctionMessage, III};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use walkdir::WalkDir;

const MAX_SKILL_BYTES: u64 = 524_288;
const MAX_WALK_DEPTH: usize = 6;

#[derive(Debug, Clone, Serialize)]
pub struct RootSkill {
    pub id: String,
    pub title: String,
    pub description: String,
    pub path: PathBuf,
}

pub type RootIndex = HashMap<String, RootSkill>;

/// Resolve default roots if the caller did not supply any.
pub fn default_roots() -> Vec<String> {
    vec![
        "./skills".to_string(),
        "~/.iii/skills".to_string(),
        "~/.claude/skills".to_string(),
        "~/.codex/skills".to_string(),
    ]
}

fn expand(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }
    PathBuf::from(path)
}

/// Walk the given roots (or defaults if empty) and return the index.
pub fn scan(roots: &[String]) -> RootIndex {
    let resolved: Vec<PathBuf> = if roots.is_empty() {
        default_roots().iter().map(|s| expand(s)).collect()
    } else {
        roots.iter().map(|s| expand(s)).collect()
    };
    let mut index = RootIndex::new();
    for root in resolved {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(&root)
            .max_depth(MAX_WALK_DEPTH)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.eq_ignore_ascii_case("SKILL.md") {
                continue;
            }
            if let Ok(meta) = std::fs::metadata(path) {
                if meta.len() > MAX_SKILL_BYTES {
                    continue;
                }
            }
            let text = match std::fs::read_to_string(path) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let Some(fm) = parse_frontmatter(&text) else {
                continue;
            };
            let id = match fm.get("id").cloned() {
                Some(s) if !s.is_empty() => s,
                _ => continue,
            };
            let title = fm.get("title").cloned().unwrap_or_else(|| id.clone());
            let description = fm.get("description").cloned().unwrap_or_default();
            index.insert(
                id.clone(),
                RootSkill {
                    id,
                    title,
                    description,
                    path: path.to_path_buf(),
                },
            );
        }
    }
    index
}

fn parse_frontmatter(text: &str) -> Option<HashMap<String, String>> {
    let stripped = text.strip_prefix("---\n")?;
    let end = stripped.find("\n---")?;
    let yaml = &stripped[..end];
    let map: serde_yaml::Mapping = serde_yaml::from_str(yaml).ok()?;
    let mut out = HashMap::new();
    for (k, v) in map {
        if let (Some(ks), Some(vs)) = (k.as_str(), v.as_str()) {
            out.insert(ks.to_string(), vs.to_string());
        }
    }
    Some(out)
}

pub fn body_after_frontmatter(text: &str) -> &str {
    if let Some(stripped) = text.strip_prefix("---\n") {
        if let Some(end) = stripped.find("\n---") {
            let after = &stripped[end + 4..];
            return after.trim_start_matches('\n');
        }
    }
    text
}

pub fn register(iii: &Arc<III>) {
    let index: Arc<RwLock<RootIndex>> = Arc::new(RwLock::new(scan(&[])));

    // skills::roots::index
    {
        let index = index.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id("skills::roots::index".to_string())
                .with_description(
                    "Slim list { id, title, description, path }[] of every SKILL.md discovered across configured roots. No body in the response."
                        .into(),
                ),
            move |payload: Value| {
                let index = index.clone();
                Box::pin(async move {
                    let filter = payload
                        .get("filter")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_lowercase());
                    let roots = payload
                        .get("roots")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|s| s.as_str().map(String::from))
                                .collect::<Vec<_>>()
                        });
                    let live: RootIndex = if let Some(roots) = roots {
                        scan(&roots)
                    } else {
                        index.read().await.clone()
                    };
                    let mut entries: Vec<Value> = live
                        .values()
                        .filter(|e| {
                            filter
                                .as_ref()
                                .map(|q| {
                                    e.id.to_lowercase().contains(q)
                                        || e.title.to_lowercase().contains(q)
                                        || e.description.to_lowercase().contains(q)
                                })
                                .unwrap_or(true)
                        })
                        .map(|e| {
                            json!({
                                "id": e.id,
                                "title": e.title,
                                "description": e.description,
                                "path": e.path.display().to_string(),
                            })
                        })
                        .collect();
                    entries.sort_by(|a, b| {
                        a["id"].as_str().unwrap_or("").cmp(b["id"].as_str().unwrap_or(""))
                    });
                    Ok::<Value, IIIError>(json!({
                        "count": entries.len(),
                        "skills": entries,
                    }))
                })
                    as std::pin::Pin<
                        Box<
                            dyn std::future::Future<Output = Result<Value, IIIError>>
                                + Send,
                        >,
                    >
            },
        ));
    }

    // skills::roots::load
    {
        let index = index.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id("skills::roots::load".to_string())
                .with_description(
                    "Read the full SKILL.md body for one discovered id. Returns { id, title, description, path, body }."
                        .into(),
                ),
            move |payload: Value| {
                let index = index.clone();
                Box::pin(async move {
                    let id = payload
                        .get("id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| IIIError::Handler("missing required field: id".into()))?
                        .to_string();
                    let entry = {
                        let read = index.read().await;
                        read.get(&id).cloned()
                    };
                    let entry = match entry {
                        Some(e) => e,
                        None => {
                            // miss — rescan once, retry.
                            let fresh = scan(&[]);
                            *index.write().await = fresh;
                            let read = index.read().await;
                            read.get(&id).cloned().ok_or_else(|| {
                                IIIError::Handler(format!("skill not found: {id}"))
                            })?
                        }
                    };
                    let text = std::fs::read_to_string(&entry.path).map_err(|e| {
                        IIIError::Handler(format!(
                            "read failed: {} — {e}",
                            entry.path.display()
                        ))
                    })?;
                    let body = body_after_frontmatter(&text).to_string();
                    Ok::<Value, IIIError>(json!({
                        "id": entry.id,
                        "title": entry.title,
                        "description": entry.description,
                        "path": entry.path.display().to_string(),
                        "body": body,
                    }))
                })
                    as std::pin::Pin<
                        Box<
                            dyn std::future::Future<Output = Result<Value, IIIError>>
                                + Send,
                        >,
                    >
            },
        ));
    }

    // skills::roots::rescan
    {
        let index = index.clone();
        iii.register_function((
            RegisterFunctionMessage::with_id("skills::roots::rescan".to_string())
                .with_description(
                    "Re-walk the configured roots and rebuild the in-memory index. Returns the new count."
                        .into(),
                ),
            move |payload: Value| {
                let index = index.clone();
                Box::pin(async move {
                    let roots = payload
                        .get("roots")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|s| s.as_str().map(String::from))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let fresh = scan(&roots);
                    let count = fresh.len();
                    *index.write().await = fresh;
                    Ok::<Value, IIIError>(json!({ "count": count }))
                })
                    as std::pin::Pin<
                        Box<
                            dyn std::future::Future<Output = Result<Value, IIIError>>
                                + Send,
                        >,
                    >
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_extracts_required_keys() {
        let text = "---\nid: foo\ntitle: Foo\ndescription: hook\n---\n\nbody\n";
        let fm = parse_frontmatter(text).expect("frontmatter parses");
        assert_eq!(fm.get("id").map(String::as_str), Some("foo"));
        assert_eq!(fm.get("title").map(String::as_str), Some("Foo"));
    }

    #[test]
    fn body_strips_frontmatter() {
        let text = "---\nid: foo\n---\n\nthis is the body";
        assert_eq!(body_after_frontmatter(text), "this is the body");
    }

    #[test]
    fn default_roots_includes_xdg_style_paths() {
        let roots = default_roots();
        assert!(roots.iter().any(|r| r.contains(".iii/skills")));
        assert!(roots.iter().any(|r| r.contains(".claude/skills")));
        assert!(roots.iter().any(|r| r.contains(".codex/skills")));
    }
}
