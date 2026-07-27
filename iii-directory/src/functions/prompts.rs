//! Filesystem-backed prompts reader + the user prompt library.
//!
//! Public API (reachable by any worker over `iii.trigger`):
//!
//!   * `directory::prompts::list` — metadata-only listing of every prompt:
//!     worker-shipped templates from `<skills_folder>/<ns>/prompts/*.md`
//!     (`source: worker`) merged with user-library entries from
//!     `prompts_folder` / `local_prompts_folder` (`source: user`), sorted
//!     by name. Filterable by `kind` and `source`.
//!   * `directory::prompts::get`  — fetch one prompt's body + metadata.
//!   * `directory::prompts::save` — write (or overwrite / fork) a user
//!     library entry; agent-callable so an orchestrator can author a
//!     small system prompt for a sub-agent and spawn with it.
//!   * `directory::prompts::delete` — remove a user library entry.
//!
//! Two kinds flow through the same store: `command` (slash-style
//! templates injected into the MESSAGE context, the default) and
//! `system` (full system prompts applied via the router override or the
//! send/spawn `system_prompt` options). The `kind` frontmatter field
//! separates them; pickers filter on it.
//!
//! Precedence on name collision: `local_prompts_folder` shadows
//! `prompts_folder`, which shadows worker-shipped templates.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::{SharedConfig, SkillsConfig};
use crate::fs_source::{self, FsPrompt};
use crate::functions::error::{not_found_message, NextAction};
use crate::sources::write_file_atomic;
use crate::trigger_types::{self, SubscriberSet};

const NAME_MAX_LEN: usize = 64;

/// The library kinds `save` accepts. Worker-shipped files may carry any
/// `kind`, but entries written through the API stay within the two the
/// pickers understand.
const SAVE_KINDS: &[&str] = &["command", "system"];

const DEFAULT_KIND: &str = "command";

/// Recovery pointer attached to a `directory::prompts::get` miss.
const PROMPT_NOT_FOUND_NEXT: &[NextAction] = &[NextAction::new(
    "directory::prompts::list",
    "browse prompt names",
)];

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct ListPromptsInput {
    /// Only entries of this `kind` (e.g. `system` or `command`).
    #[serde(default)]
    kind: Option<String>,
    /// Only entries from this source: `worker` or `user`.
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct PromptEntry {
    name: String,
    description: String,
    /// `command` (default) or `system`, from frontmatter.
    kind: String,
    /// `worker` (shipped template) or `user` (library entry).
    source: String,
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
    /// `command` (default) or `system`, from frontmatter.
    pub kind: String,
    /// `worker` (shipped template) or `user` (library entry).
    pub source: String,
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PromptSaveInput {
    /// Library entry name (lowercase ASCII, digits, `-`, `_`).
    pub name: String,
    /// Non-empty teaser shown by `list`.
    pub description: String,
    /// The prompt text. Omit it and set `from` to fork an existing
    /// prompt's body under the new name.
    #[serde(default)]
    pub body: Option<String>,
    /// `command` (default) or `system`.
    #[serde(default)]
    pub kind: Option<String>,
    /// Name of an existing prompt whose body seeds this one when `body`
    /// is omitted (fork).
    #[serde(default)]
    pub from: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PromptSaveOutput {
    pub name: String,
    pub kind: String,
    /// Absolute path written.
    pub path: String,
    /// `true` when an existing library file of the same name was replaced.
    pub overwrote: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PromptDeleteInput {
    pub name: String,
    /// Must be exactly `true` — same confirmation convention as the
    /// worker lifecycle ops.
    #[serde(default)]
    pub yes: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PromptDeleteOutput {
    pub name: String,
    pub deleted: bool,
}

pub fn register(iii: &Arc<IIIClient>, cfg: &SharedConfig, prompts_subs: &SubscriberSet) {
    register_list_prompts(iii, cfg);
    register_get_prompt(iii, cfg);
    register_save_prompt(iii, cfg, prompts_subs);
    register_delete_prompt(iii, cfg, prompts_subs);
}

fn register_list_prompts(iii: &Arc<IIIClient>, cfg: &SharedConfig) {
    let cfg_inner = cfg.clone();
    iii.register_function(
        "directory::prompts::list",
        RegisterFunction::new_async(move |input: ListPromptsInput| {
            let cfg = cfg_inner.load_full();
            async move {
                let out: Vec<PromptEntry> = collect_all(&cfg)
                    .into_iter()
                    .filter(|(p, source)| {
                        input.kind.as_deref().is_none_or(|k| k == effective_kind(p))
                            && input.source.as_deref().is_none_or(|s| s == *source)
                    })
                    .map(|(p, source)| {
                        let modified_at = fs_modified_at(&p.abs_path);
                        PromptEntry {
                            kind: effective_kind(&p).to_string(),
                            source: source.to_string(),
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
            "List prompts: worker-shipped templates (source `worker`) merged with user \
             library entries (source `user`), each `command` or `system` kind. Filter with \
             `kind` and/or `source`.",
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
            "Fetch one prompt by name (user library entries shadow worker templates). \
             Returns the raw markdown body plus name, description, kind, source, and \
             modified_at — no envelope, no templating.",
        ),
    );
}

fn register_save_prompt(iii: &Arc<IIIClient>, cfg: &SharedConfig, subs: &SubscriberSet) {
    let cfg_inner = cfg.clone();
    let subs = subs.clone();
    let engine = iii.clone();
    iii.register_function(
        "directory::prompts::save",
        RegisterFunction::new_async(move |req: PromptSaveInput| {
            let cfg = cfg_inner.load_full();
            let subs = subs.clone();
            let engine = engine.clone();
            async move {
                let out = save_prompt(&cfg, req).map_err(Error::Handler)?;
                notify_change(&engine, &subs, "save", &out.name).await;
                Ok::<_, Error>(out)
            }
        })
        .description(
            "Save a prompt to the user library (`prompts_folder`): name, description, \
             body, kind `command` or `system` (default `command`). Omit `body` and set \
             `from` to fork an existing prompt under the new name. Overwrites a \
             same-named library entry; fires directory::prompts::on-change.",
        ),
    );
}

fn register_delete_prompt(iii: &Arc<IIIClient>, cfg: &SharedConfig, subs: &SubscriberSet) {
    let cfg_inner = cfg.clone();
    let subs = subs.clone();
    let engine = iii.clone();
    iii.register_function(
        "directory::prompts::delete",
        RegisterFunction::new_async(move |req: PromptDeleteInput| {
            let cfg = cfg_inner.load_full();
            let subs = subs.clone();
            let engine = engine.clone();
            async move {
                let out = delete_prompt(&cfg, req).map_err(Error::Handler)?;
                notify_change(&engine, &subs, "delete", &out.name).await;
                Ok::<_, Error>(out)
            }
        })
        .description(
            "Delete a user library prompt from `prompts_folder`. Requires exactly \
             `yes: true`. Worker-shipped templates and project-local files are refused. \
             Fires directory::prompts::on-change.",
        ),
    );
}

// ---------- core helpers (reusable in tests) ----------

/// Fire `directory::prompts::on-change` for a library write, matching the
/// download fan-out's payload shape.
async fn notify_change(engine: &IIIClient, subs: &SubscriberSet, op: &str, name: &str) {
    trigger_types::dispatch(
        engine,
        subs,
        json!({ "op": op, "name": name, "source": "user" }),
    )
    .await;
}

/// `command` unless the frontmatter says otherwise.
fn effective_kind(p: &FsPrompt) -> &str {
    p.kind.as_deref().unwrap_or(DEFAULT_KIND)
}

/// Every visible prompt with its source, name-shadowed in precedence
/// order: local library, then global library, then worker-shipped.
pub fn collect_all(cfg: &SkillsConfig) -> Vec<(FsPrompt, &'static str)> {
    let mut seen: Vec<(FsPrompt, &'static str)> = Vec::new();
    let mut push_new = |batch: Vec<FsPrompt>, source: &'static str| {
        for p in batch {
            if !seen.iter().any(|(existing, _)| existing.name == p.name) {
                seen.push((p, source));
            }
        }
    };
    let (local_user, _) = fs_source::scan_user_prompts(&cfg.resolved_local_prompts_folder());
    push_new(local_user, "user");
    let (global_user, _) = fs_source::scan_user_prompts(&cfg.resolved_prompts_folder());
    push_new(global_user, "user");
    let (shipped, _) =
        fs_source::scan_prompts_merged(&cfg.resolved_skills_folder(), &cfg.local_skills_folder());
    push_new(shipped, "worker");
    seen.sort_by(|a, b| a.0.name.cmp(&b.0.name));
    seen
}

pub async fn get_prompt(
    cfg: &SkillsConfig,
    req: PromptGetInput,
) -> Result<PromptGetOutput, String> {
    let name = req.name;
    validate_name(&name)?;
    let all = collect_all(cfg);
    let Some((fs, source)) = all.iter().find(|(p, _)| p.name == name).cloned() else {
        let names: Vec<String> = all.into_iter().map(|(p, _)| p.name).collect();
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
        kind: effective_kind(&fs).to_string(),
        source: source.to_string(),
        name: fs.name,
        description: fs.description,
        body,
        raw,
        modified_at,
    })
}

pub fn save_prompt(cfg: &SkillsConfig, req: PromptSaveInput) -> Result<PromptSaveOutput, String> {
    validate_name(&req.name)?;
    let description = req.description.trim();
    if description.is_empty() {
        return Err("description must be non-empty".into());
    }
    let kind = req.kind.as_deref().unwrap_or(DEFAULT_KIND);
    if !SAVE_KINDS.contains(&kind) {
        return Err(format!("kind must be one of {SAVE_KINDS:?}, got {kind:?}"));
    }
    let body = match (req.body, req.from.as_deref()) {
        (Some(b), _) if !b.trim().is_empty() => b,
        (_, Some(from)) => {
            validate_name(from)?;
            let all = collect_all(cfg);
            let Some((src, _)) = all.iter().find(|(p, _)| p.name == from) else {
                return Err(format!("fork source prompt {from:?} not found"));
            };
            fs_source::read_body(&src.abs_path)?
        }
        _ => return Err("provide a non-empty `body`, or `from` to fork an existing prompt".into()),
    };

    let dest = cfg
        .resolved_prompts_folder()
        .join(format!("{}.md", req.name));
    let overwrote = dest.exists();
    // Serialized with the same YAML machinery the scanner parses it back
    // with, so writer and reader can never drift on escaping. The name is
    // deliberately NOT written: the scanner derives it from the file stem,
    // and a stored copy would go stale on a hand-rename.
    let fm_yaml = serde_yaml::to_string(&WrittenFrontmatter { description, kind })
        .map_err(|e| format!("serialize frontmatter: {e}"))?;
    let contents = format!("---\n{fm_yaml}---\n\n{}", body.trim_start_matches('\n'));
    write_file_atomic(&dest, contents.as_bytes())?;
    Ok(PromptSaveOutput {
        name: req.name,
        kind: kind.to_string(),
        path: dest.display().to_string(),
        overwrote,
    })
}

pub fn delete_prompt(
    cfg: &SkillsConfig,
    req: PromptDeleteInput,
) -> Result<PromptDeleteOutput, String> {
    if !req.yes {
        return Err("pass exactly `yes: true` to confirm the delete".into());
    }
    validate_name(&req.name)?;
    let dest = cfg
        .resolved_prompts_folder()
        .join(format!("{}.md", req.name));
    if !dest.exists() {
        let all = collect_all(cfg);
        if all.iter().any(|(p, _)| p.name == req.name) {
            return Err(format!(
                "{:?} is not a user library entry in prompts_folder — worker-shipped \
                 templates and project-local files are not deletable through this function",
                req.name
            ));
        }
        return Err(format!("no user library prompt named {:?}", req.name));
    }
    std::fs::remove_file(&dest).map_err(|e| format!("remove {}: {e}", dest.display()))?;
    Ok(PromptDeleteOutput {
        name: req.name,
        deleted: true,
    })
}

/// The frontmatter `save` writes; parsed back by `PromptFrontmatter` in
/// `fs_source`.
#[derive(Serialize)]
struct WrittenFrontmatter<'a> {
    description: &'a str,
    kind: &'a str,
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

    fn temp_cfg() -> (SkillsConfig, tempfile::TempDir) {
        let root = tempfile::tempdir().expect("tempdir");
        let cfg = SkillsConfig {
            skills_folder: root.path().join("skills").display().to_string(),
            local_skills_folder: root.path().join("local-skills").display().to_string(),
            prompts_folder: root.path().join("prompts").display().to_string(),
            local_prompts_folder: root.path().join("local-prompts").display().to_string(),
            ..SkillsConfig::default()
        };
        (cfg, root)
    }

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

    #[test]
    fn save_list_get_roundtrip_with_kind_and_source() {
        let (cfg, root) = temp_cfg();
        let out = save_prompt(
            &cfg,
            PromptSaveInput {
                name: "blog-writer".into(),
                description: "Writes blog posts: direct, punchy".into(),
                body: Some("You write blog posts. Use the web worker only.".into()),
                kind: Some("system".into()),
                from: None,
            },
        )
        .expect("save");
        assert!(!out.overwrote);
        assert_eq!(out.kind, "system");

        let all = collect_all(&cfg);
        let (entry, source) = all.iter().find(|(p, _)| p.name == "blog-writer").unwrap();
        assert_eq!(*source, "user");
        assert_eq!(entry.kind.as_deref(), Some("system"));

        let got = futures::executor::block_on(get_prompt(
            &cfg,
            PromptGetInput {
                name: "blog-writer".into(),
                raw: None,
            },
        ))
        .expect("get");
        assert_eq!(got.kind, "system");
        assert_eq!(got.source, "user");
        assert!(got.body.contains("web worker"));

        drop(root);
    }

    #[test]
    fn fork_seeds_body_from_an_existing_prompt() {
        let (cfg, root) = temp_cfg();
        save_prompt(
            &cfg,
            PromptSaveInput {
                name: "base".into(),
                description: "base prompt".into(),
                body: Some("ORIGINAL BODY".into()),
                kind: Some("system".into()),
                from: None,
            },
        )
        .unwrap();
        let forked = save_prompt(
            &cfg,
            PromptSaveInput {
                name: "base-fork".into(),
                description: "forked".into(),
                body: None,
                kind: Some("system".into()),
                from: Some("base".into()),
            },
        )
        .expect("fork");
        let got = futures::executor::block_on(get_prompt(
            &cfg,
            PromptGetInput {
                name: "base-fork".into(),
                raw: None,
            },
        ))
        .unwrap();
        assert!(got.body.contains("ORIGINAL BODY"));
        assert!(!forked.overwrote);
        drop(root);
    }

    #[test]
    fn save_rejects_unknown_kind_and_empty_body() {
        let (cfg, root) = temp_cfg();
        assert!(save_prompt(
            &cfg,
            PromptSaveInput {
                name: "x".into(),
                description: "d".into(),
                body: Some("b".into()),
                kind: Some("vibe".into()),
                from: None,
            },
        )
        .is_err());
        assert!(save_prompt(
            &cfg,
            PromptSaveInput {
                name: "x".into(),
                description: "d".into(),
                body: None,
                kind: None,
                from: None,
            },
        )
        .is_err());
        drop(root);
    }

    #[test]
    fn delete_requires_confirmation_and_only_touches_the_library() {
        let (cfg, root) = temp_cfg();
        save_prompt(
            &cfg,
            PromptSaveInput {
                name: "victim".into(),
                description: "d".into(),
                body: Some("b".into()),
                kind: None,
                from: None,
            },
        )
        .unwrap();
        assert!(delete_prompt(
            &cfg,
            PromptDeleteInput {
                name: "victim".into(),
                yes: false,
            },
        )
        .is_err());
        let out = delete_prompt(
            &cfg,
            PromptDeleteInput {
                name: "victim".into(),
                yes: true,
            },
        )
        .expect("delete");
        assert!(out.deleted);
        assert!(delete_prompt(
            &cfg,
            PromptDeleteInput {
                name: "victim".into(),
                yes: true,
            },
        )
        .is_err());
        drop(root);
    }

    #[test]
    fn local_library_shadows_global_shadows_worker() {
        let (cfg, root) = temp_cfg();
        std::fs::create_dir_all(cfg.resolved_local_prompts_folder()).unwrap();
        std::fs::write(
            cfg.resolved_local_prompts_folder().join("dup.md"),
            "---\ndescription: local\n---\nLOCAL",
        )
        .unwrap();
        save_prompt(
            &cfg,
            PromptSaveInput {
                name: "dup".into(),
                description: "global".into(),
                body: Some("GLOBAL".into()),
                kind: None,
                from: None,
            },
        )
        .unwrap();
        let all = collect_all(&cfg);
        let hits: Vec<_> = all.iter().filter(|(p, _)| p.name == "dup").collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0.description, "local");
        drop(root);
    }
}
