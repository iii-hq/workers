//! State-backed skills registry, ported from the `mcp` worker.
//!
//! Public API (reachable by any worker over `iii.trigger`):
//!
//!   * `skills::register`   — store a markdown skill body keyed by id.
//!   * `skills::unregister` — delete one by id (idempotent).
//!   * `skills::list`       — metadata-only listing, sorted by id.
//!
//! Internal RPC called only by the `mcp` worker (hard-floored under
//! `skills::*` so never an MCP tool):
//!
//!   * `skills::resources-list`      — `{ resources: [...] }` for MCP `resources/list`.
//!   * `skills::resources-read`      — `{ contents: [...] }` for MCP `resources/read`.
//!   * `skills::resources-templates` — `{ resourceTemplates: [...] }` for MCP `resources/templates/list`.
//!
//! Each mutation fans out through [`trigger_types::dispatch`] on the
//! `skills::on-change` trigger type so interested workers (the `mcp`
//! worker today) can forward MCP notifications.

use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, TriggerRequest, III};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::SkillsConfig;
use crate::state;
use crate::trigger_types::{self, SubscriberSet};

const SKILL_BODY_MAX_BYTES: usize = 256 * 1024;
const ID_MAX_LEN: usize = 64;
const INDEX_URI: &str = "iii://skills";
const URI_PREFIX: &str = "iii://";

/// Prefixes that are NEVER allowed as the `function_id` half of an
/// `iii://{skill}/{fn}` resource URI. Mirrors the hard-floor list in
/// [mcp/src/handler.rs](../../mcp/src/handler.rs); duplicated here so
/// `skills` can enforce the recursion guard without importing from
/// the mcp crate (each worker is its own cargo workspace). Keep the
/// two lists in sync when adding an infra namespace.
pub const ALWAYS_HIDDEN_PREFIXES: &[&str] = &[
    "engine::",
    "state::",
    "stream::",
    "iii.",
    "iii::",
    "mcp::",
    "a2a::",
    "skills::",
    "prompts::",
];

pub fn is_always_hidden(function_id: &str) -> bool {
    ALWAYS_HIDDEN_PREFIXES
        .iter()
        .any(|p| function_id.starts_with(p))
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RegisterSkillInput {
    /// Unique skill id (lowercase ASCII, kebab/underscore, max 64 chars).
    id: String,
    /// Markdown body served at iii://{id}.
    skill: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct RegisterSkillOutput {
    id: String,
    registered_at: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UnregisterSkillInput {
    id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct UnregisterSkillOutput {
    id: String,
    removed: bool,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct ListSkillsInput {}

#[derive(Debug, Serialize, JsonSchema)]
struct SkillEntry {
    id: String,
    bytes: usize,
    registered_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ListSkillsOutput {
    skills: Vec<SkillEntry>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct EmptyInput {}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReadResourceInput {
    uri: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct StoredSkill {
    id: String,
    skill: String,
    registered_at: String,
}

pub fn register(iii: &Arc<III>, cfg: &Arc<SkillsConfig>, subscribers: &SubscriberSet) {
    register_register_skill(iii, cfg, subscribers);
    register_unregister_skill(iii, cfg, subscribers);
    register_list_skills(iii, cfg);
    register_resources_list(iii, cfg);
    register_resources_read(iii, cfg);
    register_resources_templates(iii);
}

fn register_register_skill(iii: &Arc<III>, cfg: &Arc<SkillsConfig>, subscribers: &SubscriberSet) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    let subs_inner = subscribers.clone();
    iii.register_function(
        RegisterFunction::new_async("skills::register", move |req: RegisterSkillInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.clone();
            let subs = subs_inner.clone();
            async move {
                validate_id(&req.id).map_err(IIIError::Handler)?;
                if req.skill.is_empty() {
                    return Err(IIIError::Handler("skill must be non-empty".into()));
                }
                if req.skill.len() > SKILL_BODY_MAX_BYTES {
                    return Err(IIIError::Handler(format!(
                        "skill body too large ({} bytes; max {SKILL_BODY_MAX_BYTES})",
                        req.skill.len()
                    )));
                }

                let stored = StoredSkill {
                    id: req.id.clone(),
                    skill: req.skill,
                    registered_at: chrono::Utc::now().to_rfc3339(),
                };
                let value = serde_json::to_value(&stored)
                    .map_err(|e| IIIError::Handler(format!("encode skill: {e}")))?;
                state::state_set(
                    &iii,
                    &cfg.scopes.skills,
                    &req.id,
                    value,
                    cfg.state_timeout_ms,
                )
                .await?;
                tracing::info!(skill_id = %req.id, "skill registered");

                // Fan out to any `skills::on-change` subscribers with a
                // Void dispatch so the write path doesn't block on
                // downstream latency.
                trigger_types::dispatch(&iii, &subs, json!({ "op": "register", "id": req.id }))
                    .await;

                Ok::<_, IIIError>(RegisterSkillOutput {
                    id: req.id,
                    registered_at: stored.registered_at,
                })
            }
        })
        .description("Register a markdown skill so it appears as iii://{id} in resources/list."),
    );
}

fn register_unregister_skill(iii: &Arc<III>, cfg: &Arc<SkillsConfig>, subscribers: &SubscriberSet) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    let subs_inner = subscribers.clone();
    iii.register_function(
        RegisterFunction::new_async("skills::unregister", move |req: UnregisterSkillInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.clone();
            let subs = subs_inner.clone();
            async move {
                validate_id(&req.id).map_err(IIIError::Handler)?;
                let prior =
                    state::state_delete(&iii, &cfg.scopes.skills, &req.id, cfg.state_timeout_ms)
                        .await?;
                let removed = !prior.is_null();
                tracing::info!(skill_id = %req.id, removed, "skill unregister");

                // Only fan out when the state actually changed so
                // idempotent deletes stay quiet.
                if removed {
                    trigger_types::dispatch(
                        &iii,
                        &subs,
                        json!({ "op": "unregister", "id": req.id }),
                    )
                    .await;
                }

                Ok::<_, IIIError>(UnregisterSkillOutput {
                    id: req.id,
                    removed,
                })
            }
        })
        .description("Remove a registered skill by id."),
    );
}

fn register_list_skills(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async("skills::list", move |_input: ListSkillsInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.clone();
            async move {
                let entries = list_stored(&iii, &cfg).await?;
                let mut out: Vec<SkillEntry> = entries
                    .into_iter()
                    .map(|s| SkillEntry {
                        bytes: s.skill.len(),
                        id: s.id,
                        registered_at: s.registered_at,
                    })
                    .collect();
                out.sort_by(|a, b| a.id.cmp(&b.id));
                Ok::<_, IIIError>(ListSkillsOutput { skills: out })
            }
        })
        .description("List registered skills (id, body length, registered_at) without bodies."),
    );
}

fn register_resources_list(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async(
            "skills::resources-list",
            move |_input: EmptyInput| {
                let iii = iii_inner.clone();
                let cfg = cfg_inner.clone();
                async move { Ok::<_, IIIError>(list_resources(&iii, &cfg).await) }
            },
        )
        .description(
            "Internal: returns the MCP resources/list envelope with the iii://skills index + one iii://{id} entry per registered skill.",
        ),
    );
}

fn register_resources_read(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async("skills::resources-read", move |req: ReadResourceInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.clone();
            async move { read(&iii, &cfg, &req.uri).await.map_err(IIIError::Handler) }
        })
        .description(
            "Internal: resolves an iii:// URI and returns the MCP resources/read contents envelope.",
        ),
    );
}

fn register_resources_templates(iii: &Arc<III>) {
    iii.register_function(
        RegisterFunction::new_async(
            "skills::resources-templates",
            move |_input: EmptyInput| async move { Ok::<_, IIIError>(list_templates()) },
        )
        .description("Internal: returns the MCP resources/templates/list envelope."),
    );
}

// ---------- core resource helpers (also usable by in-process tests) ----------

pub async fn list_resources(iii: &III, cfg: &SkillsConfig) -> Value {
    let mut resources: Vec<Value> = vec![json!({
        "uri": INDEX_URI,
        "name": "skills",
        "description": "Index of every registered skill",
        "mimeType": "text/markdown",
    })];
    if let Ok(skills) = list_stored(iii, cfg).await {
        for s in skills {
            let title = extract_title(&s.skill).unwrap_or(&s.id);
            resources.push(json!({
                "uri": format!("{URI_PREFIX}{}", s.id),
                "name": title,
                "mimeType": "text/markdown",
            }));
        }
    }
    json!({ "resources": resources })
}

pub fn list_templates() -> Value {
    json!({
        "resourceTemplates": [
            {
                "uriTemplate": "iii://{skill_id}",
                "name": "Skill",
                "description": "Markdown body of a registered skill",
                "mimeType": "text/markdown"
            },
            {
                "uriTemplate": "iii://{skill_id}/{function_id}",
                "name": "Skill section",
                "description": "Markdown returned by triggering function_id with empty input",
                "mimeType": "text/markdown"
            }
        ]
    })
}

pub async fn read(iii: &III, cfg: &SkillsConfig, uri: &str) -> Result<Value, String> {
    let parsed = parse_uri(uri)?;
    match parsed {
        ParsedUri::Index => {
            let body = render_index(iii, cfg).await;
            Ok(wrap_contents(uri, "text/markdown", &body))
        }
        ParsedUri::Skill(id) => {
            validate_id(&id)?;
            let stored = read_skill(iii, cfg, &id)
                .await?
                .ok_or_else(|| format!("Skill not found: {id}"))?;
            Ok(wrap_contents(uri, "text/markdown", &stored.skill))
        }
        ParsedUri::Section { function_id, .. } => {
            // Recursion guard — a client that crafts iii://x/state::set would
            // otherwise tunnel into infra. We also block skills::* / prompts::*
            // so the resource resolver can't drive the admin API.
            if is_always_hidden(&function_id) {
                return Err(format!(
                    "Function '{function_id}' is in an internal namespace and cannot back a skill resource"
                ));
            }
            let value = iii
                .trigger(TriggerRequest {
                    function_id: function_id.clone(),
                    payload: json!({}),
                    action: None,
                    timeout_ms: Some(cfg.state_timeout_ms),
                })
                .await
                .map_err(|e| format!("trigger {function_id}: {e}"))?;
            let (text, mime) = normalize_function_output(value);
            Ok(wrap_contents(uri, mime, &text))
        }
    }
}

// ---------- URI parsing ----------

#[derive(Debug, PartialEq, Eq)]
pub enum ParsedUri {
    Index,
    Skill(String),
    Section {
        skill_id: String,
        function_id: String,
    },
}

pub fn parse_uri(uri: &str) -> Result<ParsedUri, String> {
    let rest = uri
        .strip_prefix(URI_PREFIX)
        .ok_or_else(|| format!("Resource URI must start with iii://: {uri}"))?;
    if rest.is_empty() {
        return Err(format!("Empty resource id: {uri}"));
    }
    if rest == "skills" {
        return Ok(ParsedUri::Index);
    }
    match rest.split_once('/') {
        None => Ok(ParsedUri::Skill(rest.to_string())),
        Some((skill_id, function_id)) => {
            if skill_id.is_empty() {
                return Err(format!("Empty skill id in URI: {uri}"));
            }
            if function_id.is_empty() {
                return Err(format!("Empty function id in URI: {uri}"));
            }
            // Function ids contain `::` but not `/`; reject extra path
            // segments rather than silently joining them.
            if function_id.contains('/') {
                return Err(format!(
                    "Resource URI may not have more than one path segment after the skill id: {uri}"
                ));
            }
            Ok(ParsedUri::Section {
                skill_id: skill_id.to_string(),
                function_id: function_id.to_string(),
            })
        }
    }
}

pub fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("id must be non-empty".into());
    }
    if id.len() > ID_MAX_LEN {
        return Err(format!(
            "id too long ({} chars; max {ID_MAX_LEN})",
            id.len()
        ));
    }
    for c in id.chars() {
        let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_';
        if !ok {
            return Err(format!(
                "id may only contain lowercase ASCII letters, digits, '-' and '_': {id:?}"
            ));
        }
    }
    Ok(())
}

// ---------- markdown helpers ----------

async fn render_index(iii: &III, cfg: &SkillsConfig) -> String {
    let skills = match list_stored(iii, cfg).await {
        Ok(s) => s,
        Err(e) => {
            return format!("# Skills\n\n_Error reading skills index: {e}_\n");
        }
    };
    let mut out = String::from(
        "# Skills\n\nRead each skill's resource for orientation on when and why to call its functions.\n\n",
    );
    if skills.is_empty() {
        out.push_str("_No skills are currently registered._\n");
        return out;
    }
    for s in skills {
        let title = extract_title(&s.skill).unwrap_or(&s.id);
        let desc = extract_description(&s.skill).unwrap_or_default();
        let suffix = if desc.is_empty() {
            String::new()
        } else {
            format!(" — {desc}")
        };
        out.push_str(&format!(
            "- [`{}`](iii://{}) — {}{}\n",
            s.id, s.id, title, suffix
        ));
    }
    out
}

pub fn extract_title(markdown: &str) -> Option<&str> {
    markdown.lines().find_map(|line| {
        let trimmed = line.trim_start();
        trimmed.strip_prefix("# ").map(|s| s.trim())
    })
}

pub fn extract_description(markdown: &str) -> Option<String> {
    let mut buf = String::new();
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !buf.is_empty() {
                break;
            }
            continue;
        }
        if trimmed.starts_with('#') {
            if !buf.is_empty() {
                break;
            }
            continue;
        }
        if !buf.is_empty() {
            buf.push(' ');
        }
        buf.push_str(trimmed);
    }
    if buf.is_empty() {
        return None;
    }
    Some(truncate_chars(&buf, 140))
}

pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        Some((byte_end, _)) => format!("{}...", &s[..byte_end]),
        None => s.to_string(),
    }
}

// ---------- output normalization for iii://{skill}/{function} ----------

pub fn normalize_function_output(v: Value) -> (String, &'static str) {
    if let Value::String(s) = &v {
        return (s.clone(), "text/markdown");
    }
    if let Some(content) = v.get("content").and_then(|c| c.as_str()) {
        return (content.to_string(), "text/markdown");
    }
    let pretty = serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string());
    (pretty, "application/json")
}

fn wrap_contents(uri: &str, mime: &str, text: &str) -> Value {
    json!({
        "contents": [
            { "uri": uri, "mimeType": mime, "text": text }
        ]
    })
}

// ---------- state wrappers ----------

async fn read_skill(
    iii: &III,
    cfg: &SkillsConfig,
    id: &str,
) -> Result<Option<StoredSkill>, String> {
    let raw = state::state_get(iii, &cfg.scopes.skills, id, cfg.state_timeout_ms)
        .await
        .map_err(|e| format!("state::get: {e}"))?;
    if raw.is_null() {
        return Ok(None);
    }
    serde_json::from_value::<StoredSkill>(raw)
        .map(Some)
        .map_err(|e| format!("decode stored skill {id}: {e}"))
}

async fn list_stored(iii: &III, cfg: &SkillsConfig) -> Result<Vec<StoredSkill>, IIIError> {
    let raw = state::state_list(iii, &cfg.scopes.skills, cfg.state_timeout_ms).await?;
    let entries = state::extract_state_entries(raw);
    let mut out: Vec<StoredSkill> = entries
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_index_uri() {
        assert_eq!(parse_uri("iii://skills").unwrap(), ParsedUri::Index);
    }

    #[test]
    fn parse_single_skill_uri() {
        assert_eq!(
            parse_uri("iii://brain").unwrap(),
            ParsedUri::Skill("brain".into())
        );
    }

    #[test]
    fn parse_section_uri() {
        assert_eq!(
            parse_uri("iii://brain/brain::summarize").unwrap(),
            ParsedUri::Section {
                skill_id: "brain".into(),
                function_id: "brain::summarize".into(),
            }
        );
    }

    #[test]
    fn rejects_missing_prefix() {
        assert!(parse_uri("brain").is_err());
        assert!(parse_uri("https://example.com").is_err());
    }

    #[test]
    fn rejects_empty_id() {
        assert!(parse_uri("iii://").is_err());
        assert!(parse_uri("iii:///fn").is_err());
        assert!(parse_uri("iii://skill/").is_err());
    }

    #[test]
    fn rejects_extra_path_segments() {
        assert!(parse_uri("iii://x/y/z").is_err());
    }

    #[test]
    fn id_validation_accepts_kebab_and_underscore() {
        assert!(validate_id("brain").is_ok());
        assert!(validate_id("agent_memory").is_ok());
        assert!(validate_id("my-skill-1").is_ok());
        assert!(validate_id("a").is_ok());
    }

    #[test]
    fn id_validation_rejects_bad_chars() {
        assert!(validate_id("").is_err());
        assert!(validate_id("UpperCase").is_err());
        assert!(validate_id("with space").is_err());
        assert!(validate_id("with/slash").is_err());
        assert!(validate_id("with::colon").is_err());
        assert!(validate_id(&"x".repeat(ID_MAX_LEN + 1)).is_err());
    }

    #[test]
    fn extract_title_finds_h1() {
        let md = "# my skill\n\nbody\n";
        assert_eq!(extract_title(md), Some("my skill"));
    }

    #[test]
    fn extract_title_ignores_h2() {
        let md = "## sub\n\nbody\n";
        assert_eq!(extract_title(md), None);
    }

    #[test]
    fn extract_description_grabs_first_paragraph() {
        let md = "# title\n\nfirst paragraph here.\n\nsecond paragraph.\n";
        assert_eq!(
            extract_description(md).as_deref(),
            Some("first paragraph here.")
        );
    }

    #[test]
    fn extract_description_skips_subheadings() {
        let md = "# title\n\n## sub\n\n### deeper\n\nfinally text.\n";
        assert_eq!(extract_description(md).as_deref(), Some("finally text."));
    }

    #[test]
    fn extract_description_handles_missing_paragraph() {
        let md = "# only a title\n";
        assert_eq!(extract_description(md), None);
    }

    #[test]
    fn extract_description_truncates_long_lines() {
        let body = "x".repeat(200);
        let md = format!("# t\n\n{body}\n");
        let desc = extract_description(&md).unwrap();
        assert!(desc.ends_with("..."));
        assert!(desc.chars().count() <= 144);
    }

    #[test]
    fn normalize_string_returns_markdown() {
        let (text, mime) = normalize_function_output(Value::String("hello".into()));
        assert_eq!(text, "hello");
        assert_eq!(mime, "text/markdown");
    }

    #[test]
    fn normalize_content_object_returns_markdown() {
        let (text, mime) = normalize_function_output(json!({ "content": "hi" }));
        assert_eq!(text, "hi");
        assert_eq!(mime, "text/markdown");
    }

    #[test]
    fn normalize_other_falls_back_to_json() {
        let (text, mime) = normalize_function_output(json!({ "x": 1 }));
        assert_eq!(mime, "application/json");
        assert!(text.contains("\"x\""));
    }

    #[test]
    fn truncate_chars_handles_multibyte() {
        let s = "áéíóú".repeat(50);
        let out = truncate_chars(&s, 5);
        assert!(out.starts_with("áéíóú"));
        assert!(out.ends_with("..."));
        assert_eq!(out.chars().count(), 5 + 3);
    }

    #[test]
    fn hard_floor_rejects_infra_namespaces() {
        assert!(is_always_hidden("engine::foo"));
        assert!(is_always_hidden("state::get"));
        assert!(is_always_hidden("stream::publish"));
        assert!(is_always_hidden("iii.on_foo"));
        assert!(is_always_hidden("iii::internal"));
        assert!(is_always_hidden("mcp::handler"));
        assert!(is_always_hidden("a2a::send"));
        assert!(is_always_hidden("skills::register"));
        assert!(is_always_hidden("prompts::register"));
    }

    #[test]
    fn hard_floor_allows_ordinary_namespaces() {
        assert!(!is_always_hidden("mem::observe"));
        assert!(!is_always_hidden("brain::summarize"));
        assert!(!is_always_hidden("my-worker::my-fn"));
    }

    #[test]
    fn list_templates_has_two_entries() {
        let v = list_templates();
        let templates = v["resourceTemplates"].as_array().unwrap();
        assert_eq!(templates.len(), 2);
    }
}
