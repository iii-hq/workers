//! Assignable agent profiles: `directory::agents::list` when iii-directory
//! is installed, else the frontmatter of `<agents_path>/*.md`.

use std::path::Path;

use iii_sdk::IIIClient;
use iii_sdk::protocol::TriggerRequest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentProfile {
    /// Agent profile id used as the ticket assignee.
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub logo: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentList {
    /// directory | agents-folder | unavailable
    pub source: String,
    pub agents: Vec<AgentProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub fn normalize_agent(raw: &Value) -> Option<AgentProfile> {
    let id = text(raw.get("id"))?;
    Some(AgentProfile {
        name: text(raw.get("name")).unwrap_or_else(|| id.clone()),
        id,
        description: text(raw.get("description")),
        logo: text(raw.get("logo")),
        icon: text(raw.get("icon")),
        color: text(raw.get("color")),
        model: text(raw.get("model")),
    })
}

fn frontmatter(raw: &str) -> &str {
    let Some(rest) = raw.strip_prefix("---") else {
        return "";
    };
    match rest.find("\n---") {
        Some(end) => &rest[..end],
        None => rest,
    }
}

fn field(front: &str, key: &str) -> Option<String> {
    front.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.trim() != key {
            return None;
        }
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// One profile from a `<id>.md` file's frontmatter; the file stem is the id.
pub fn profile_from_markdown(stem: &str, raw: &str) -> AgentProfile {
    let front = frontmatter(raw);
    AgentProfile {
        id: stem.to_string(),
        name: field(front, "name").unwrap_or_else(|| stem.to_string()),
        description: field(front, "description"),
        logo: field(front, "logo"),
        icon: field(front, "icon"),
        color: field(front, "color"),
        model: field(front, "model"),
    }
}

pub async fn profiles_from_disk(dir: &Path) -> Vec<AgentProfile> {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return Vec::new();
    };
    let mut profiles = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let Some(stem) = path
            .extension()
            .filter(|ext| *ext == "md")
            .and_then(|_| path.file_stem())
            .and_then(|stem| stem.to_str())
        else {
            continue;
        };
        if let Ok(raw) = tokio::fs::read_to_string(&path).await {
            profiles.push(profile_from_markdown(stem, &raw));
        }
    }
    profiles.sort_by(|a, b| a.id.cmp(&b.id));
    profiles
}

pub async fn list_agents(iii: &IIIClient, agents_dir: &Path) -> AgentList {
    let directory = iii
        .trigger(TriggerRequest {
            function_id: "directory::agents::list".to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await;
    let error = match directory {
        Ok(result) => {
            let agents: Vec<AgentProfile> = result
                .get("agents")
                .and_then(Value::as_array)
                .map(|rows| rows.iter().filter_map(normalize_agent).collect())
                .unwrap_or_default();
            if !agents.is_empty() {
                return AgentList {
                    source: "directory".to_string(),
                    agents,
                    error: None,
                };
            }
            None
        }
        Err(error) => Some(error.to_string()),
    };
    let agents = profiles_from_disk(agents_dir).await;
    AgentList {
        source: if agents.is_empty() {
            "unavailable"
        } else {
            "agents-folder"
        }
        .to_string(),
        agents,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_fields_are_read_and_unquoted() {
        let raw = "---\nname: Product Manager\ndescription: \"Plans: features\"\nlogo: \"📋\"\ncolor: amber\nextends: iii-minimal\n---\n# Body\nname: not this\n";
        let profile = profile_from_markdown("product-manager", raw);
        assert_eq!(profile.id, "product-manager");
        assert_eq!(profile.name, "Product Manager");
        assert_eq!(profile.description.as_deref(), Some("Plans: features"));
        assert_eq!(profile.logo.as_deref(), Some("📋"));
        assert_eq!(profile.color.as_deref(), Some("amber"));
        assert_eq!(profile.model, None);
    }

    #[test]
    fn a_file_without_frontmatter_falls_back_to_its_stem() {
        let profile = profile_from_markdown("tech-lead", "# Tech Lead\n");
        assert_eq!(profile.name, "tech-lead");
        assert_eq!(profile.description, None);
    }

    #[test]
    fn normalize_agent_requires_an_id() {
        assert!(normalize_agent(&json!({ "name": "x" })).is_none());
        let agent = normalize_agent(&json!({ "id": " tech-lead ", "logo": "" })).unwrap();
        assert_eq!(agent.id, "tech-lead");
        assert_eq!(agent.name, "tech-lead");
        assert_eq!(agent.logo, None);
    }
}
