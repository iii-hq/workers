//! Single-file write paths for skills and prompts.
//!
//! Public API (reachable by any worker over `iii.trigger`):
//!
//!   * `directory::skills::update`  — overwrite one EXISTING skill
//!     markdown file with new full-file content.
//!   * `directory::prompts::update` — overwrite one EXISTING prompt
//!     markdown file with new full-file content.
//!
//! Both take the FULL raw file (frontmatter block included) — the same
//! string `directory::skills::get { raw: true }` / `directory::prompts::get
//! { raw: true }` return — so an editor round-trips the exact on-disk
//! bytes. Updates never create files: downloads (and direct disk edits)
//! are how skills arrive; update only mutates what a read can already
//! see. Resolution mirrors the read path (merged global+local scan, the
//! `<id>` → `<id>/index` overview alias, the installed-worker visibility
//! filter) so update can never write a file `list`/`get` would hide.
//!
//! Content is validated against the READ invariants before the write —
//! anything this module accepts, the scanners and readers will serve:
//!
//!   * both: size cap ([`SKILL_BODY_MAX_BYTES`] on the raw file), body
//!     non-empty after frontmatter strip;
//!   * prompts additionally: required YAML frontmatter with a non-empty
//!     `description`, and a valid `name` when the frontmatter declares
//!     one (the scanner would otherwise skip the file on next scan).
//!
//! Writes are atomic (tmp + rename, same as download) and fan out the
//! same `directory::skills::on-change` / `directory::prompts::on-change`
//! triggers with `{ op: "update", ... }` payloads.

use std::path::Path;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::{SharedConfig, SkillsConfig};
use crate::fs_source::{self, FsPrompt, FsSkill, SkillFrontmatter};
use crate::functions::error::{invalid_input_message, not_found_message, NextAction};
use crate::functions::prompts::validate_name;
use crate::functions::skills::{
    find_fs_skill_in, normalize_get_id, reject_function_id_shaped, resolve_title,
    resolve_visible_skills, validate_id, RegisteredWorkersCache, SKILL_BODY_MAX_BYTES,
};
use crate::sources::write_file_atomic;
use crate::trigger_types;

/// Recovery pointers for a missed skill update target.
const SKILL_UPDATE_NOT_FOUND_NEXT: &[NextAction] = &[
    NextAction::new("directory::skills::list", "browse skill ids"),
    NextAction::new(
        "directory::skills::download",
        "materialize a missing bundle first",
    ),
];

/// Recovery pointer for a missed prompt update target.
const PROMPT_UPDATE_NOT_FOUND_NEXT: &[NextAction] = &[NextAction::new(
    "directory::prompts::list",
    "browse prompt names",
)];

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SkillUpdateInput {
    /// Skill id to overwrite — the same forms `directory::skills::get`
    /// accepts (bare id, `<id>.md`, `SKILL(S).md`, `iii://<id>`). The
    /// target file must already exist; update never creates skills.
    pub id: String,
    /// FULL new file content, frontmatter block included — the string
    /// `directory::skills::get { raw: true }` returns, edited.
    pub content: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SkillUpdateOutput {
    /// The id that was updated (normalized form of the input id).
    pub id: String,
    /// Title resolved from the NEW content (frontmatter `title:`, then
    /// body H1, then the id) — what `list` rows will now show.
    pub title: String,
    /// Frontmatter `type:` of the new content, `null` when absent.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Frontmatter `function_id:` of the new content, `null` when absent.
    pub function_id: Option<String>,
    /// Bytes written.
    pub bytes: usize,
    /// File mtime after the write, RFC 3339.
    pub modified_at: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PromptUpdateInput {
    /// Prompt name to overwrite (as returned by
    /// `directory::prompts::list`). The target file must already exist.
    pub name: String,
    /// FULL new file content, frontmatter block included. The
    /// frontmatter must keep a non-empty `description` — a prompt
    /// without one would be skipped by the next scan.
    pub content: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PromptUpdateOutput {
    /// The prompt's EFFECTIVE name after the write: the new
    /// frontmatter `name:` when declared, otherwise the file stem.
    /// Differs from the input name when the update renames the prompt.
    pub name: String,
    /// Description parsed from the new frontmatter.
    pub description: String,
    /// Bytes written.
    pub bytes: usize,
    /// File mtime after the write, RFC 3339.
    pub modified_at: String,
}

pub fn register(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    subscribers: &super::Subscribers,
    cache: &Arc<RegisteredWorkersCache>,
) {
    register_update_skill(iii, cfg, &subscribers.skills, cache);
    register_update_prompt(iii, cfg, &subscribers.prompts);
}

fn register_update_skill(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    subs: &trigger_types::SubscriberSet,
    cache: &Arc<RegisteredWorkersCache>,
) {
    let cfg_inner = cfg.clone();
    let iii_inner = iii.clone();
    let subs_inner = subs.clone();
    let cache_inner = cache.clone();
    iii.register_function(
        "directory::skills::update",
        RegisterFunction::new_async(move |req: SkillUpdateInput| {
            let cfg = cfg_inner.load_full();
            let iii = iii_inner.clone();
            let subs = subs_inner.clone();
            let cache = cache_inner.clone();
            async move {
                let visible = resolve_visible_skills(&cfg, &cache, &iii, false).await;
                let out = update_skill_in(&visible, &req).map_err(Error::Handler)?;
                let namespace = out.id.split('/').next().unwrap_or("").to_string();
                trigger_types::dispatch(
                    &iii,
                    &subs,
                    json!({ "op": "update", "namespace": namespace, "id": out.id }),
                )
                .await;
                Ok::<_, Error>(out)
            }
        })
        .description(
            "Overwrite one EXISTING filesystem-backed skill with new full-file markdown \
             content (frontmatter block included — pass back the `raw` field from \
             directory::skills::get { raw: true }, edited). Accepts the same id forms as \
             `get`. Never creates files (use directory::skills::download to materialize a \
             bundle). Content is validated against the read invariants (size cap, \
             non-empty body after frontmatter); the write is atomic and fans out \
             directory::skills::on-change with { op: \"update\" }.",
        )
        .metadata(json!({"tool": {"label": "Update skill"}})),
    );
}

fn register_update_prompt(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    subs: &trigger_types::SubscriberSet,
) {
    let cfg_inner = cfg.clone();
    let iii_inner = iii.clone();
    let subs_inner = subs.clone();
    iii.register_function(
        "directory::prompts::update",
        RegisterFunction::new_async(move |req: PromptUpdateInput| {
            let cfg = cfg_inner.load_full();
            let iii = iii_inner.clone();
            let subs = subs_inner.clone();
            async move {
                let out = update_prompt(&cfg, &req).map_err(Error::Handler)?;
                trigger_types::dispatch(&iii, &subs, json!({ "op": "update", "name": out.name }))
                    .await;
                Ok::<_, Error>(out)
            }
        })
        .description(
            "Overwrite one EXISTING filesystem-backed prompt with new full-file markdown \
             content. The frontmatter must keep a non-empty `description` (and a valid \
             `name` when it declares one) — the same rules the scanner enforces, so an \
             update can never produce a file the next directory::prompts::list would \
             skip. The write is atomic and fans out directory::prompts::on-change with \
             { op: \"update\" }. Returns the prompt's effective name after the write \
             (frontmatter `name:` wins over the file stem).",
        )
        .metadata(json!({"tool": {"label": "Update prompt"}})),
    );
}

// ---------- core helpers (engine-free, reusable in tests) ----------

/// Resolve `req.id` against an already-resolved visible set and write the
/// new content. Pure with respect to the engine: callers hand in the
/// visible skills, so tests drive it straight off a tempdir scan.
pub fn update_skill_in(
    visible: &[FsSkill],
    req: &SkillUpdateInput,
) -> Result<SkillUpdateOutput, String> {
    let id = normalize_get_id(&req.id)?;
    reject_function_id_shaped(&id)?;
    validate_id(&id)?;
    validate_skill_content(&req.content)?;

    let Some(fs) = find_fs_skill_in(visible, &id) else {
        let ids: Vec<String> = visible.iter().map(|s| s.id.clone()).collect();
        let candidates = closest_ids(&ids, &id, 3);
        return Err(not_found_message(
            "D110",
            "skill",
            &id,
            &candidates,
            SKILL_UPDATE_NOT_FOUND_NEXT,
        ));
    };

    write_file_atomic(&fs.abs_path, req.content.as_bytes())?;

    let (fm_text, body) = fs_source::split_frontmatter(&req.content);
    let fm: SkillFrontmatter = fm_text
        .and_then(|t| serde_yaml::from_str(t).ok())
        .unwrap_or_default();
    let title = resolve_title(&fm, body, &id);
    Ok(SkillUpdateOutput {
        id,
        title,
        kind: crate::functions::skills::clean_optional(fm.kind),
        function_id: crate::functions::skills::clean_optional(fm.function_id),
        bytes: req.content.len(),
        modified_at: fs_modified_at(&fs.abs_path),
    })
}

/// Prompt update against the merged (global + local) prompt view — the
/// same view `directory::prompts::list` serves.
pub fn update_prompt(
    cfg: &SkillsConfig,
    req: &PromptUpdateInput,
) -> Result<PromptUpdateOutput, String> {
    validate_name(&req.name)?;
    let (prompts, _skipped) =
        fs_source::scan_prompts_merged(&cfg.resolved_skills_folder(), &cfg.local_skills_folder());
    let Some(fs) = prompts.iter().find(|p| p.name == req.name) else {
        let names: Vec<String> = prompts.iter().map(|p| p.name.clone()).collect();
        let candidates = closest_ids(&names, &req.name, 3);
        return Err(not_found_message(
            "D210",
            "prompt",
            &req.name,
            &candidates,
            PROMPT_UPDATE_NOT_FOUND_NEXT,
        ));
    };
    let (effective_name, description) = validate_prompt_content(fs, &req.content)?;

    write_file_atomic(&fs.abs_path, req.content.as_bytes())?;

    Ok(PromptUpdateOutput {
        name: effective_name,
        description,
        bytes: req.content.len(),
        modified_at: fs_modified_at(&fs.abs_path),
    })
}

/// Shared content rules for both kinds: raw size cap and a non-empty
/// body after frontmatter strip, mirroring `read_skill_with_frontmatter`
/// so an accepted write is always a servable read.
fn validate_skill_content(content: &str) -> Result<(), String> {
    if content.len() > SKILL_BODY_MAX_BYTES {
        return Err(invalid_input_message(
            "D113",
            &format!(
                "content is too large ({} bytes; max {SKILL_BODY_MAX_BYTES}).",
                content.len()
            ),
            &[],
        ));
    }
    let (_, body) = fs_source::split_frontmatter(content);
    if body.trim_matches('\n').trim().is_empty() {
        return Err(invalid_input_message(
            "D113",
            "content has an empty body after the frontmatter block; readers reject empty bodies.",
            &[],
        ));
    }
    Ok(())
}

/// Prompt-specific content rules on top of [`validate_skill_content`]:
/// the frontmatter the scanner requires must survive the write. Returns
/// the effective `(name, description)` the next scan will report.
fn validate_prompt_content(fs: &FsPrompt, content: &str) -> Result<(String, String), String> {
    validate_skill_content(content)?;
    let fm = fs_source::parse_prompt_frontmatter(content)
        .map_err(|e| invalid_input_message("D213", &format!("{e}."), &[]))?;
    let description = match fm.description {
        Some(d) if !d.trim().is_empty() => d.trim().to_string(),
        _ => {
            return Err(invalid_input_message(
                "D213",
                "frontmatter is missing a non-empty `description`; the next scan would skip \
                 this prompt.",
                &[],
            ));
        }
    };
    let stem = fs
        .abs_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&fs.name)
        .to_string();
    let effective_name = match fm.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(declared) => {
            validate_name(declared).map_err(|e| {
                invalid_input_message(
                    "D213",
                    &format!(
                        "frontmatter `name` is invalid ({e}); the next scan would skip \
                     this prompt."
                    ),
                    &[],
                )
            })?;
            declared.to_string()
        }
        None => stem,
    };
    Ok((effective_name, description))
}

/// Closest candidates for a missed update target, by lowercased
/// Levenshtein distance (same ranker the prompt reader uses).
fn closest_ids(ids: &[String], missed: &str, limit: usize) -> Vec<String> {
    let missed_lc = missed.to_lowercase();
    let mut scored: Vec<(usize, &String)> = ids
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

fn fs_modified_at(path: &Path) -> String {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn write_fixture(dir: &Path, rel: &str, contents: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    fn scan(dir: &Path) -> Vec<FsSkill> {
        let (skills, skipped) = fs_source::scan_skills(dir);
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");
        skills
    }

    fn cfg_for(dir: &Path) -> SkillsConfig {
        SkillsConfig {
            skills_folder: dir.to_string_lossy().into_owned(),
            // Point the local root INSIDE the tempdir so a `.iii/skills`
            // in the test process CWD can't shadow the fixtures.
            local_skills_folder: dir.join("local-empty").to_string_lossy().into_owned(),
            ..SkillsConfig::default()
        }
    }

    // ── skills ───────────────────────────────────────────────────────

    #[test]
    fn update_skill_overwrites_file_and_reports_new_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "ns/how-to.md", "# Old\n\nold body\n");
        let visible = scan(tmp.path());

        let new_content = "---\ntitle: New title\ntype: how-to\n---\n# H1\n\nnew body\n";
        let out = update_skill_in(
            &visible,
            &SkillUpdateInput {
                id: "ns/how-to".into(),
                content: new_content.into(),
            },
        )
        .unwrap();

        assert_eq!(out.id, "ns/how-to");
        assert_eq!(out.title, "New title");
        assert_eq!(out.kind.as_deref(), Some("how-to"));
        assert_eq!(out.bytes, new_content.len());
        assert!(!out.modified_at.is_empty());
        let on_disk = std::fs::read_to_string(tmp.path().join("ns/how-to.md")).unwrap();
        assert_eq!(on_disk, new_content);
    }

    #[test]
    fn update_skill_resolves_bare_worker_alias_to_index() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "ns/index.md", "# Overview\n\nold\n");
        let visible = scan(tmp.path());

        let out = update_skill_in(
            &visible,
            &SkillUpdateInput {
                id: "ns".into(),
                content: "# Overview\n\nnew\n".into(),
            },
        )
        .unwrap();
        assert_eq!(out.id, "ns");
        let on_disk = std::fs::read_to_string(tmp.path().join("ns/index.md")).unwrap();
        assert!(on_disk.contains("new"));
    }

    #[test]
    fn update_skill_missing_target_is_not_found_with_candidates() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "ns/real.md", "# Real\n\nbody\n");
        let visible = scan(tmp.path());

        let err = update_skill_in(
            &visible,
            &SkillUpdateInput {
                id: "ns/reel".into(),
                content: "# X\n\nbody\n".into(),
            },
        )
        .unwrap_err();
        assert!(err.starts_with("D110 not_found:"), "got: {err}");
        assert!(err.contains("ns/real"), "got: {err}");
        // Nothing was created.
        assert!(!tmp.path().join("ns/reel.md").exists());
    }

    #[test]
    fn update_skill_rejects_empty_body_and_oversize() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "ns/a.md", "# A\n\nbody\n");
        let visible = scan(tmp.path());

        let empty = update_skill_in(
            &visible,
            &SkillUpdateInput {
                id: "ns/a".into(),
                content: "---\ntitle: x\n---\n".into(),
            },
        )
        .unwrap_err();
        assert!(empty.starts_with("D113 invalid_input:"), "got: {empty}");

        let huge = update_skill_in(
            &visible,
            &SkillUpdateInput {
                id: "ns/a".into(),
                content: "x".repeat(SKILL_BODY_MAX_BYTES + 1),
            },
        )
        .unwrap_err();
        assert!(huge.contains("too large"), "got: {huge}");

        // Original untouched by rejected writes.
        let on_disk = std::fs::read_to_string(tmp.path().join("ns/a.md")).unwrap();
        assert_eq!(on_disk, "# A\n\nbody\n");
    }

    #[test]
    fn update_skill_rejects_function_id_shaped_input() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "ns/a.md", "# A\n\nbody\n");
        let visible = scan(tmp.path());
        let err = update_skill_in(
            &visible,
            &SkillUpdateInput {
                id: "sandbox::create".into(),
                content: "# X\n\nbody\n".into(),
            },
        )
        .unwrap_err();
        assert!(err.contains("FUNCTION id"), "got: {err}");
    }

    // ── prompts ──────────────────────────────────────────────────────

    #[test]
    fn update_prompt_overwrites_and_returns_effective_name() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns/prompts/open-pr.md",
            "---\ndescription: Old.\n---\nOld body.\n",
        );
        let cfg = cfg_for(tmp.path());

        let new_content = "---\nname: open-pr-v2\ndescription: New.\n---\nNew body.\n";
        let out = update_prompt(
            &cfg,
            &PromptUpdateInput {
                name: "open-pr".into(),
                content: new_content.into(),
            },
        )
        .unwrap();
        assert_eq!(out.name, "open-pr-v2", "frontmatter name wins");
        assert_eq!(out.description, "New.");
        let on_disk = std::fs::read_to_string(tmp.path().join("ns/prompts/open-pr.md")).unwrap();
        assert_eq!(on_disk, new_content);
    }

    #[test]
    fn update_prompt_rejects_content_the_scanner_would_skip() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns/prompts/keep.md",
            "---\ndescription: Fine.\n---\nBody.\n",
        );
        let cfg = cfg_for(tmp.path());

        for bad in [
            "no frontmatter at all\n",
            "---\nname: keep\n---\nno description\n",
            "---\nname: Bad Name\ndescription: x\n---\nbody\n",
        ] {
            let err = update_prompt(
                &cfg,
                &PromptUpdateInput {
                    name: "keep".into(),
                    content: bad.into(),
                },
            )
            .unwrap_err();
            assert!(err.starts_with("D213 invalid_input:"), "got: {err}");
        }
        // Original untouched.
        let on_disk = std::fs::read_to_string(tmp.path().join("ns/prompts/keep.md")).unwrap();
        assert_eq!(on_disk, "---\ndescription: Fine.\n---\nBody.\n");
    }

    #[test]
    fn update_prompt_missing_target_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns/prompts/real.md",
            "---\ndescription: x\n---\nBody.\n",
        );
        let cfg = cfg_for(tmp.path());
        let err = update_prompt(
            &cfg,
            &PromptUpdateInput {
                name: "reel".into(),
                content: "---\ndescription: y\n---\nBody.\n".into(),
            },
        )
        .unwrap_err();
        assert!(err.starts_with("D210 not_found:"), "got: {err}");
        assert!(err.contains("real"), "got: {err}");
    }
}
