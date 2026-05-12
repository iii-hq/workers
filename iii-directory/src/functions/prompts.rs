//! Filesystem-backed prompts reader.
//!
//! Public API (reachable by any worker over `iii.trigger`):
//!
//!   * `prompts::list` — metadata-only listing of every prompt in
//!     `<skills_folder>/<ns>/prompts/*.md`, sorted by name.
//!   * `prompts::get`  — fetch one prompt's body + metadata.
//!
//! Both responses are plain JSON shapes — no MCP envelope, no role/
//! messages wrapper — so this worker stays agnostic to MCP and any
//! other adapter. Adapters can shape the response on their own side.
//!
//! There is no `prompts::register` / `prompts::unregister`. Prompts
//! arrive on disk via `skills::download` (or by direct editing) and are
//! re-read on every list/get call.

use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, III};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::SkillsConfig;
use crate::fs_source::{self, FsPrompt};

const NAME_MAX_LEN: usize = 64;

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct ListPromptsInput {}

#[derive(Debug, Serialize, JsonSchema)]
struct PromptEntry {
    name: String,
    description: String,
    /// File mtime as RFC 3339.
    modified_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ListPromptsOutput {
    prompts: Vec<PromptEntry>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PromptGetInput {
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PromptGetOutput {
    pub name: String,
    pub description: String,
    /// Raw markdown body (post-frontmatter) from disk.
    pub body: String,
    /// File mtime as RFC 3339.
    pub modified_at: String,
}

pub fn register(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    register_list_prompts(iii, cfg);
    register_get_prompt(iii, cfg);
}

fn register_list_prompts(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async("prompts::list", move |_input: ListPromptsInput| {
            let cfg = cfg_inner.clone();
            async move {
                let (prompts, _skipped) = fs_source::scan_prompts(&cfg.resolved_skills_folder());
                let out: Vec<PromptEntry> = prompts
                    .into_iter()
                    .map(|p| {
                        let modified_at = fs_modified_at(&p.abs_path);
                        PromptEntry {
                            name: p.name,
                            description: p.description,
                            modified_at,
                        }
                    })
                    .collect();
                Ok::<_, IIIError>(ListPromptsOutput { prompts: out })
            }
        })
        .description(
            "List filesystem-backed prompts (name, description, modified_at) from skills_folder.",
        ),
    );
}

fn register_get_prompt(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async("prompts::get", move |req: PromptGetInput| {
            let cfg = cfg_inner.clone();
            async move { get_prompt(&cfg, req).await.map_err(IIIError::Handler) }
        })
        .description(
            "Fetch one filesystem-backed prompt by name. Returns the raw markdown body plus name, \
             description, and modified_at — no envelope, no templating.",
        ),
    );
}

// ---------- core helpers (reusable in tests) ----------

pub async fn get_prompt(
    cfg: &SkillsConfig,
    req: PromptGetInput,
) -> Result<PromptGetOutput, String> {
    let name = req.name;
    validate_name(&name)?;
    let Some(fs) = find_fs_prompt(cfg, &name) else {
        return Err(format!("Prompt not found: {name}"));
    };
    let body = fs_source::read_body(&fs.abs_path)?;
    let modified_at = fs_modified_at(&fs.abs_path);
    Ok(PromptGetOutput {
        name: fs.name,
        description: fs.description,
        body,
        modified_at,
    })
}

// ---------- validation ----------

pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("name must be non-empty".into());
    }
    if name.len() > NAME_MAX_LEN {
        return Err(format!(
            "name too long ({} chars; max {NAME_MAX_LEN})",
            name.len()
        ));
    }
    for c in name.chars() {
        let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_';
        if !ok {
            return Err(format!(
                "name may only contain lowercase ASCII letters, digits, '-' and '_': {name:?}"
            ));
        }
    }
    Ok(())
}

// ---------- fs lookup ----------

fn find_fs_prompt(cfg: &SkillsConfig, name: &str) -> Option<FsPrompt> {
    let (prompts, _skipped) = fs_source::scan_prompts(&cfg.resolved_skills_folder());
    prompts.into_iter().find(|p| p.name == name)
}

fn fs_modified_at(path: &std::path::Path) -> String {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_validation_accepts_kebab_and_underscore() {
        assert!(validate_name("send-email").is_ok());
        assert!(validate_name("triage_ticket").is_ok());
        assert!(validate_name("a").is_ok());
        assert!(validate_name("v2").is_ok());
    }

    #[test]
    fn name_validation_rejects_bad_chars() {
        assert!(validate_name("").is_err());
        assert!(validate_name("Send-Email").is_err());
        assert!(validate_name("send email").is_err());
        assert!(validate_name("send/email").is_err());
        assert!(validate_name("mcp::send").is_err());
        assert!(validate_name(&"x".repeat(NAME_MAX_LEN + 1)).is_err());
    }
}
