//! Filesystem-backed prompts reader + creator.
//!
//! Public API (reachable by any worker over `iii.trigger`):
//!
//!   * `directory::prompts::list` — metadata-only listing of every prompt
//!     in `<skills_folder>/<ns>/prompts/*.md`, sorted by name.
//!   * `directory::prompts::get`  — fetch one prompt's body + metadata.
//!   * `directory::prompts::save` — create a new prompt file under
//!     `<skills_folder>/user/prompts/<name>.md` (create-only).
//!
//! All responses are plain JSON shapes — no MCP envelope, no role/
//! messages wrapper — so this worker stays agnostic to MCP and any
//! other adapter. Adapters can shape the response on their own side.
//!
//! Prompts arrive on disk via `directory::skills::download`,
//! `directory::prompts::save`, or direct editing, and are re-read on
//! every list/get call. `save` deliberately fires no
//! `directory::prompts::on-change` fan-out: nothing subscribes to it
//! today and clients re-list on demand.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::{SharedConfig, SkillsConfig};
use crate::fs_source;
use crate::functions::error::{not_found_message, NextAction};

const NAME_MAX_LEN: usize = 64;

/// Namespace under the GLOBAL skills folder that holds prompts created
/// via `directory::prompts::save`. A plain directory — prompt scans have
/// no namespace-registration requirement. NB: a project-local
/// `./.iii/skills/user/` namespace shadows this one wholesale (existing
/// whole-namespace merge semantics).
const USER_PROMPTS_NAMESPACE: &str = "user";

/// How a prompt combines with the caller's built-in system prompt:
/// `enrich` appends to it (safe default), `override` replaces it.
/// Maps 1:1 onto `harness::send` `options.system_prompt_strategy`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PromptStrategy {
    #[default]
    Enrich,
    Override,
}

impl PromptStrategy {
    /// Lenient read-side parse: `"override"` maps to `Override`; anything
    /// else (absent, unknown, mixed case) falls back to `Enrich` so a bad
    /// frontmatter value never hides a prompt from the list.
    pub fn parse_lenient(s: Option<&str>) -> Self {
        match s.map(str::trim) {
            Some(v) if v.eq_ignore_ascii_case("override") => Self::Override,
            _ => Self::Enrich,
        }
    }
}

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
    /// System-prompt combination strategy declared in frontmatter.
    strategy: PromptStrategy,
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
    /// System-prompt combination strategy declared in frontmatter.
    pub strategy: PromptStrategy,
    /// Raw markdown body (post-frontmatter) from disk.
    pub body: String,
    /// File mtime as RFC 3339.
    pub modified_at: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PromptSaveInput {
    pub name: String,
    /// Markdown body (no frontmatter). Must be non-empty.
    pub body: String,
    #[serde(default)]
    pub strategy: PromptStrategy,
    /// Defaults to the first non-empty body line (truncated).
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PromptSaveOutput {
    pub name: String,
    /// Absolute path of the created file.
    pub path: String,
}

pub fn register(iii: &Arc<IIIClient>, cfg: &SharedConfig) {
    register_list_prompts(iii, cfg);
    register_get_prompt(iii, cfg);
    register_save_prompt(iii, cfg);
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
                            strategy: p.strategy,
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
             description, strategy, and modified_at — no envelope, no templating.",
        ),
    );
}

fn register_save_prompt(iii: &Arc<IIIClient>, cfg: &SharedConfig) {
    let cfg_inner = cfg.clone();
    iii.register_function(
        "directory::prompts::save",
        RegisterFunction::new_async(move |req: PromptSaveInput| {
            let cfg = cfg_inner.load_full();
            async move { save_prompt(&cfg, req).map_err(Error::Handler) }
        })
        .description(
            "Create a new filesystem-backed prompt at <skills_folder>/user/prompts/<name>.md \
             (create-only; fails if the name exists). Frontmatter carries description and \
             strategy: enrich appends to the caller's built-in system prompt, override \
             replaces it.",
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
    let modified_at = fs_modified_at(&fs.abs_path);
    Ok(PromptGetOutput {
        name: fs.name,
        description: fs.description,
        strategy: fs.strategy,
        body,
        modified_at,
    })
}

pub fn save_prompt(cfg: &SkillsConfig, req: PromptSaveInput) -> Result<PromptSaveOutput, String> {
    let name = req.name.trim().to_string();
    validate_name(&name)?;
    let body = req.body.trim();
    if body.is_empty() {
        return Err("body must be non-empty".into());
    }

    let (prompts, _skipped) =
        fs_source::scan_prompts_merged(&cfg.resolved_skills_folder(), &cfg.local_skills_folder());
    if let Some(existing) = prompts.iter().find(|p| p.name == name) {
        return Err(format!(
            "prompt {name:?} already exists at {}; pick another name",
            existing.abs_path.display()
        ));
    }

    let description = req
        .description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| default_description(body));

    #[derive(Serialize)]
    struct Frontmatter<'a> {
        name: &'a str,
        description: &'a str,
        strategy: PromptStrategy,
    }
    let fm_yaml = serde_yaml::to_string(&Frontmatter {
        name: &name,
        description: &description,
        strategy: req.strategy,
    })
    .map_err(|e| format!("frontmatter serialize: {e}"))?;
    let fm_yaml = fm_yaml.strip_prefix("---\n").unwrap_or(fm_yaml.as_str());
    let content = format!("---\n{fm_yaml}---\n\n{body}\n");

    let max = crate::functions::skills::SKILL_BODY_MAX_BYTES;
    if content.len() > max {
        return Err(format!(
            "prompt too large ({} bytes; max {max})",
            content.len()
        ));
    }

    let dest = cfg
        .resolved_skills_folder()
        .join(USER_PROMPTS_NAMESPACE)
        .join("prompts")
        .join(format!("{name}.md"));
    // A file invisible to the scan (e.g. broken frontmatter) must still
    // never be silently overwritten.
    if dest.exists() {
        return Err(format!(
            "prompt file already exists at {}; pick another name",
            dest.display()
        ));
    }
    crate::sources::write_file_atomic(&dest, content.as_bytes())?;
    Ok(PromptSaveOutput {
        name,
        path: dest.display().to_string(),
    })
}

/// First non-empty body line, char-safe-truncated to ~80 chars.
fn default_description(body: &str) -> String {
    let line = body
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() > 80 {
        let truncated: String = line.chars().take(79).collect();
        format!("{truncated}…")
    } else {
        line.to_string()
    }
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

    fn cfg_for(tmp: &std::path::Path) -> SkillsConfig {
        SkillsConfig {
            skills_folder: tmp.join("global").display().to_string(),
            local_skills_folder: tmp.join("local").display().to_string(),
            ..SkillsConfig::default()
        }
    }

    #[tokio::test]
    async fn save_then_get_roundtrips_body_and_strategy() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());
        let out = save_prompt(
            &cfg,
            PromptSaveInput {
                name: "code-review".into(),
                body: "Review the diff.\nBe strict.".into(),
                strategy: PromptStrategy::Override,
                description: None,
            },
        )
        .unwrap();
        assert!(
            out.path.ends_with("user/prompts/code-review.md"),
            "unexpected path: {}",
            out.path
        );

        let got = get_prompt(
            &cfg,
            PromptGetInput {
                name: "code-review".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(got.body.trim(), "Review the diff.\nBe strict.");
        assert_eq!(got.strategy, PromptStrategy::Override);
        // Derived description = first non-empty body line.
        assert_eq!(got.description, "Review the diff.");
    }

    #[test]
    fn save_rejects_duplicate_name() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());
        let input = || PromptSaveInput {
            name: "dupe".into(),
            body: "Body.".into(),
            strategy: PromptStrategy::Enrich,
            description: None,
        };
        save_prompt(&cfg, input()).unwrap();
        let err = save_prompt(&cfg, input()).unwrap_err();
        assert!(err.contains("already exists"), "got: {err}");
    }

    #[test]
    fn save_rejects_scan_invisible_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());
        // No frontmatter — invisible to scan_prompts, but must not be
        // silently overwritten.
        let dest = tmp.path().join("global/user/prompts/ghost.md");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, "no frontmatter\n").unwrap();
        let err = save_prompt(
            &cfg,
            PromptSaveInput {
                name: "ghost".into(),
                body: "Body.".into(),
                strategy: PromptStrategy::Enrich,
                description: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("already exists"), "got: {err}");
    }

    #[test]
    fn save_rejects_empty_body() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());
        let err = save_prompt(
            &cfg,
            PromptSaveInput {
                name: "empty".into(),
                body: "   \n".into(),
                strategy: PromptStrategy::Enrich,
                description: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("non-empty"), "got: {err}");
    }

    #[test]
    fn save_writes_wellformed_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());
        let out = save_prompt(
            &cfg,
            PromptSaveInput {
                name: "fm-check".into(),
                body: "Body line.".into(),
                strategy: PromptStrategy::Enrich,
                description: Some("A description: with colon.".into()),
            },
        )
        .unwrap();
        let raw = std::fs::read_to_string(&out.path).unwrap();
        assert!(raw.starts_with("---\n"), "got: {raw}");
        assert!(raw.contains("strategy: enrich"), "got: {raw}");
        // serde_yaml must quote/escape the colon-bearing description so
        // the scan can parse it back.
        let (prompts, skipped) = fs_source::scan_prompts(&cfg.resolved_skills_folder());
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");
        assert_eq!(prompts[0].description, "A description: with colon.");
    }

    #[test]
    fn default_description_truncates_long_first_line() {
        let long = "x".repeat(120);
        let d = default_description(&long);
        assert_eq!(d.chars().count(), 80);
        assert!(d.ends_with('…'));
        assert_eq!(default_description("\n\n  short  \nrest"), "short");
    }
}
