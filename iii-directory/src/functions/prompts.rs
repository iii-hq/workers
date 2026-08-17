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

/// Recovery pointer attached to a `directory::system-prompts::get` miss.
const SYSTEM_PROMPT_NOT_FOUND_NEXT: &[NextAction] = &[NextAction::new(
    "directory::system-prompts::list",
    "browse system prompt names",
)];

fn not_found_next(kind: fs_source::PromptKind) -> &'static [NextAction] {
    match kind {
        fs_source::PromptKind::Command => PROMPT_NOT_FOUND_NEXT,
        fs_source::PromptKind::System => SYSTEM_PROMPT_NOT_FOUND_NEXT,
    }
}

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
    register_list(
        iii,
        cfg,
        fs_source::PromptKind::Command,
        "directory::prompts::list",
        "List filesystem-backed command-template prompts (name, description, modified_at) \
         from skills_folder (`prompts/` path segment).",
    );
    register_list(
        iii,
        cfg,
        fs_source::PromptKind::System,
        "directory::system-prompts::list",
        "List filesystem-backed system prompts (name, description, modified_at) from \
         skills_folder (`system-prompts/` path segment).",
    );
    register_get(
        iii,
        cfg,
        fs_source::PromptKind::Command,
        "directory::prompts::get",
        "Fetch one filesystem-backed command-template prompt by name. Returns the raw \
         markdown body plus name, description, and modified_at — no envelope, no templating.",
    );
    register_get(
        iii,
        cfg,
        fs_source::PromptKind::System,
        "directory::system-prompts::get",
        "Fetch one filesystem-backed system prompt by name. Returns the raw markdown body \
         plus name, description, and modified_at — no envelope, no templating.",
    );
}

fn register_list(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    kind: fs_source::PromptKind,
    function_id: &str,
    description: &str,
) {
    let cfg_inner = cfg.clone();
    iii.register_function(
        function_id,
        RegisterFunction::new_async(move |_input: ListPromptsInput| {
            let cfg = cfg_inner.load_full();
            async move {
                let (prompts, _skipped) = fs_source::scan_prompts_merged(
                    &cfg.resolved_skills_folder(),
                    &cfg.local_skills_folder(),
                    kind,
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
        .description(description),
    );
}

fn register_get(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    kind: fs_source::PromptKind,
    function_id: &str,
    description: &str,
) {
    let cfg_inner = cfg.clone();
    iii.register_function(
        function_id,
        RegisterFunction::new_async(move |req: PromptGetInput| {
            let cfg = cfg_inner.load_full();
            async move { get_prompt(&cfg, req, kind).await.map_err(Error::Handler) }
        })
        .description(description),
    );
}

// ---------- core helpers (reusable in tests) ----------

pub async fn get_prompt(
    cfg: &SkillsConfig,
    req: PromptGetInput,
    kind: fs_source::PromptKind,
) -> Result<PromptGetOutput, String> {
    let name = req.name;
    validate_name(&name)?;
    let (prompts, _skipped) = fs_source::scan_prompts_merged(
        &cfg.resolved_skills_folder(),
        &cfg.local_skills_folder(),
        kind,
    );
    let Some(fs) = prompts.iter().find(|p| p.name == name).cloned() else {
        let names: Vec<String> = prompts.into_iter().map(|p| p.name).collect();
        let candidates = rank_prompt_names(&names, &name, 3);
        return Err(not_found_message(
            "D210",
            kind.noun(),
            &name,
            &candidates,
            not_found_next(kind),
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

    fn write_fixture(dir: &std::path::Path, rel: &str, contents: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    fn cfg_for(dir: &std::path::Path) -> SkillsConfig {
        SkillsConfig {
            skills_folder: dir.to_string_lossy().into_owned(),
            local_skills_folder: dir.join("local-empty").to_string_lossy().into_owned(),
            ..SkillsConfig::default()
        }
    }

    #[tokio::test]
    async fn get_prompt_is_kind_scoped() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns/prompts/hello.md",
            "---\ndescription: cmd\n---\nCommand body.\n",
        );
        write_fixture(
            tmp.path(),
            "system-prompts/hello.md",
            "---\ndescription: sys\n---\nSystem body.\n",
        );
        let cfg = cfg_for(tmp.path());

        let cmd = get_prompt(
            &cfg,
            PromptGetInput {
                name: "hello".into(),
                raw: None,
            },
            fs_source::PromptKind::Command,
        )
        .await
        .unwrap();
        assert_eq!(cmd.body.trim(), "Command body.");

        let sys = get_prompt(
            &cfg,
            PromptGetInput {
                name: "hello".into(),
                raw: None,
            },
            fs_source::PromptKind::System,
        )
        .await
        .unwrap();
        assert_eq!(sys.body.trim(), "System body.");
    }

    #[tokio::test]
    async fn get_system_prompt_miss_names_the_kind_and_its_list() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "system-prompts/real.md",
            "---\ndescription: s\n---\nB.\n",
        );
        let cfg = cfg_for(tmp.path());
        let err = get_prompt(
            &cfg,
            PromptGetInput {
                name: "reel".into(),
                raw: None,
            },
            fs_source::PromptKind::System,
        )
        .await
        .unwrap_err();
        assert!(
            err.starts_with("D210 not_found: system prompt"),
            "got: {err}"
        );
        assert!(err.contains("real"), "candidate missing: {err}");
        assert!(
            err.contains("directory::system-prompts::list"),
            "next-action must point at the system family: {err}"
        );
    }
}
