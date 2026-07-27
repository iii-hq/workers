//! Filesystem-backed prompts reader.
//!
//! Public API (reachable by any worker over `iii.trigger`):
//!
//!   * `directory::prompts::list` — metadata-only listing of every prompt
//!     in `<skills_folder>/<ns>/prompts/*.md`, sorted by name.
//!   * `directory::prompts::get`  — fetch one prompt's body + metadata.
//!
//! Both responses are plain JSON shapes — no MCP envelope, no role/
//! messages wrapper — so this worker stays agnostic to MCP and any
//! other adapter. Adapters can shape the response on their own side.
//!
//! There is no `prompts::register` / `prompts::unregister`. Prompts
//! arrive on disk via `directory::skills::download` (or by direct
//! editing) and are re-read on every list/get call.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::{SharedConfig, SkillsConfig};
use crate::fs_source;
use crate::functions::error::{not_found_message, NextAction};

const NAME_MAX_LEN: usize = 64;

/// Recovery pointer attached to a `directory::prompts::get` miss.
const PROMPT_NOT_FOUND_NEXT: &[NextAction] = &[NextAction::new(
    "directory::prompts::list",
    "browse prompt names",
)];

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
    /// When `true`, the response includes the FULL on-disk file content
    /// (frontmatter block included) as `raw`. For editors that need to
    /// round-trip the exact file (`directory::prompts::update` takes the
    /// same full-file form); agent readers should leave this unset and
    /// use `body`.
    #[serde(default)]
    pub raw: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PromptGetOutput {
    pub name: String,
    pub description: String,
    /// Raw markdown body (post-frontmatter) from disk.
    pub body: String,
    /// FULL on-disk file content (frontmatter included). Present only
    /// when the request set `raw: true` — the exact string to hand back
    /// to `directory::prompts::update`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    /// File mtime as RFC 3339.
    pub modified_at: String,
}

pub fn register(iii: &Arc<IIIClient>, cfg: &SharedConfig) {
    register_list_prompts(iii, cfg);
    register_get_prompt(iii, cfg);
}

fn register_list_prompts(iii: &Arc<IIIClient>, cfg: &SharedConfig) {
    let cfg_inner = cfg.clone();
    iii.register_function(
        "directory::prompts::list",
        RegisterFunction::new_async(move |_input: ListPromptsInput| {
            let cfg = cfg_inner.load_full();
            async move {
                let (prompts, _skipped) = fs_source::scan_prompts_merged(
                    &cfg.resolved_skills_folder(),
                    &cfg.local_skills_folder(),
                );
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
                Ok::<_, Error>(ListPromptsOutput { prompts: out })
            }
        })
        .description(
            "List filesystem-backed prompts (name, description, modified_at) from skills_folder.",
        ),
    );
}

fn register_get_prompt(iii: &Arc<IIIClient>, cfg: &SharedConfig) {
    let cfg_inner = cfg.clone();
    iii.register_function(
        "directory::prompts::get",
        RegisterFunction::new_async(move |req: PromptGetInput| {
            let cfg = cfg_inner.load_full();
            async move { get_prompt(&cfg, req).await.map_err(Error::Handler) }
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
    let (prompts, _skipped) =
        fs_source::scan_prompts_merged(&cfg.resolved_skills_folder(), &cfg.local_skills_folder());
    let Some(fs) = prompts.iter().find(|p| p.name == name).cloned() else {
        let names: Vec<String> = prompts.into_iter().map(|p| p.name).collect();
        let candidates = rank_prompt_names(&names, &name, 3);
        return Err(not_found_message(
            "D210",
            "prompt",
            &name,
            &candidates,
            PROMPT_NOT_FOUND_NEXT,
        ));
    };
    let body = fs_source::read_body(&fs.abs_path)?;
    let raw = if req.raw.unwrap_or(false) {
        Some(fs_source::read_raw(&fs.abs_path)?)
    } else {
        None
    };
    let modified_at = fs_modified_at(&fs.abs_path);
    Ok(PromptGetOutput {
        name: fs.name,
        description: fs.description,
        body,
        raw,
        modified_at,
    })
}

/// Rank prompt names by closeness to a missed name (lowercased Levenshtein,
/// reusing the skills ranker's distance fn), returning the closest `limit`.
/// Empty when there are no prompts on disk.
fn rank_prompt_names(names: &[String], missed: &str, limit: usize) -> Vec<String> {
    let missed_lc = missed.to_lowercase();
    let mut scored: Vec<(usize, &String)> = names
        .iter()
        .map(|n| {
            (
                crate::functions::skills::levenshtein(&missed_lc, &n.to_lowercase()),
                n,
            )
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, n)| n.clone())
        .collect()
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
