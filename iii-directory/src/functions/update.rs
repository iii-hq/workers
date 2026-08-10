//! Single-file write paths for skills and prompts.
//!
//! Public API (reachable by any worker over `iii.trigger`):
//!
//!   * `directory::skills::update`  — overwrite one EXISTING skill
//!     markdown file with new full-file content.
//!   * `directory::prompts::update` — overwrite one EXISTING prompt
//!     markdown file with new full-file content.
//!   * `directory::system-prompts::delete` — permanently remove one
//!     EXISTING system-prompt markdown file.
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

/// Recovery pointer for a missed system-prompt update target.
const SYSTEM_PROMPT_UPDATE_NOT_FOUND_NEXT: &[NextAction] = &[NextAction::new(
    "directory::system-prompts::list",
    "browse system prompt names",
)];

/// Recovery pointers for a create that hit an existing name/path.
const PROMPT_CREATE_CONFLICT_NEXT: &[NextAction] = &[
    NextAction::new("directory::prompts::update", "edit the existing prompt"),
    NextAction::new("directory::prompts::list", "browse prompt names"),
];

/// Recovery pointers for a system-prompt create that hit an existing name/path.
const SYSTEM_PROMPT_CREATE_CONFLICT_NEXT: &[NextAction] = &[
    NextAction::new(
        "directory::system-prompts::update",
        "edit the existing system prompt",
    ),
    NextAction::new(
        "directory::system-prompts::list",
        "browse system prompt names",
    ),
];

/// Select the in-family "browse" recovery pointer for a missed update
/// target, so a command-prompt miss never points an agent at the system
/// family's `list` (or vice versa).
fn update_not_found_next(kind: fs_source::PromptKind) -> &'static [NextAction] {
    match kind {
        fs_source::PromptKind::Command => PROMPT_UPDATE_NOT_FOUND_NEXT,
        fs_source::PromptKind::System => SYSTEM_PROMPT_UPDATE_NOT_FOUND_NEXT,
    }
}

/// Select the in-family recovery pointers for a create-time conflict.
fn create_conflict_next(kind: fs_source::PromptKind) -> &'static [NextAction] {
    match kind {
        fs_source::PromptKind::Command => PROMPT_CREATE_CONFLICT_NEXT,
        fs_source::PromptKind::System => SYSTEM_PROMPT_CREATE_CONFLICT_NEXT,
    }
}

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
    /// Prompt name to overwrite, as returned by the matching list for
    /// this kind (`directory::prompts::list` for command templates,
    /// `directory::system-prompts::list` for system prompts). The
    /// target file must already exist.
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PromptCreateInput {
    /// New prompt name — becomes the file stem
    /// (`<skills_folder>/prompts/<name>.md` for command templates,
    /// `<skills_folder>/system-prompts/<name>.md` for system prompts).
    /// Same charset rules as list/get names; must not collide with an
    /// existing prompt of the SAME kind (a command prompt and a system
    /// prompt may share a name).
    pub name: String,
    /// FULL file content, frontmatter block included — the same form
    /// `directory::prompts::update` takes. Frontmatter must carry a
    /// non-empty `description`; a declared `name` must match the
    /// requested name.
    pub content: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PromptCreateOutput {
    /// The created prompt's name (as requested).
    pub name: String,
    /// Description parsed from the frontmatter.
    pub description: String,
    /// Bytes written.
    pub bytes: usize,
    /// File mtime after the write, RFC 3339.
    pub modified_at: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PromptDeleteInput {
    /// Existing system-prompt name, as returned by
    /// `directory::system-prompts::list`.
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PromptDeleteOutput {
    /// Name of the system prompt whose file was removed.
    pub name: String,
}

pub fn register(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    subscribers: &super::Subscribers,
    cache: &Arc<RegisteredWorkersCache>,
) {
    register_update_skill(iii, cfg, &subscribers.skills, cache);
    register_update_prompt(
        iii,
        cfg,
        &subscribers.prompts,
        fs_source::PromptKind::Command,
        "directory::prompts::update",
    );
    register_update_prompt(
        iii,
        cfg,
        &subscribers.system_prompts,
        fs_source::PromptKind::System,
        "directory::system-prompts::update",
    );
    register_create_prompt(
        iii,
        cfg,
        &subscribers.prompts,
        fs_source::PromptKind::Command,
        "directory::prompts::create",
    );
    register_create_prompt(
        iii,
        cfg,
        &subscribers.system_prompts,
        fs_source::PromptKind::System,
        "directory::system-prompts::create",
    );
    register_delete_system_prompt(iii, cfg, &subscribers.system_prompts);
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
    kind: fs_source::PromptKind,
    function_id: &str,
) {
    let cfg_inner = cfg.clone();
    let iii_inner = iii.clone();
    let subs_inner = subs.clone();
    let (description, label): (&str, &str) = match kind {
        fs_source::PromptKind::Command => (
            "Overwrite one EXISTING filesystem-backed prompt with new full-file markdown \
             content. The frontmatter must keep a non-empty `description` (and a valid \
             `name` when it declares one) — the same rules the scanner enforces, so an \
             update can never produce a file the next directory::prompts::list would \
             skip. The write is atomic and fans out directory::prompts::on-change with \
             { op: \"update\" }. Returns the prompt's effective name after the write \
             (frontmatter `name:` wins over the file stem). Command templates only — \
             system prompts have their own directory::system-prompts::* family.",
            "Update prompt",
        ),
        fs_source::PromptKind::System => (
            "Overwrite one EXISTING filesystem-backed system prompt with new full-file \
             markdown content. The frontmatter must keep a non-empty `description` (and \
             a valid `name` when it declares one) — the same rules the scanner enforces, \
             so an update can never produce a file the next \
             directory::system-prompts::list would skip. The write is atomic and fans \
             out directory::system-prompts::on-change with { op: \"update\" }. Returns \
             the system prompt's effective name after the write (frontmatter `name:` \
             wins over the file stem). System prompts only — command templates have \
             their own directory::prompts::* family.",
            "Update system prompt",
        ),
    };
    iii.register_function(
        function_id,
        RegisterFunction::new_async(move |req: PromptUpdateInput| {
            let cfg = cfg_inner.load_full();
            let iii = iii_inner.clone();
            let subs = subs_inner.clone();
            async move {
                let out = update_prompt(&cfg, &req, kind).map_err(Error::Handler)?;
                trigger_types::dispatch(&iii, &subs, json!({ "op": "update", "name": out.name }))
                    .await;
                Ok::<_, Error>(out)
            }
        })
        .description(description)
        .metadata(json!({"tool": {"label": label}})),
    );
}

fn register_create_prompt(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    subs: &trigger_types::SubscriberSet,
    kind: fs_source::PromptKind,
    function_id: &str,
) {
    let cfg_inner = cfg.clone();
    let iii_inner = iii.clone();
    let subs_inner = subs.clone();
    let (description, label): (&str, &str) = match kind {
        fs_source::PromptKind::Command => (
            "Create a NEW command-template prompt at <skills_folder>/prompts/<name>.md \
             from full-file markdown content (frontmatter block included; a non-empty \
             `description` is required, and a declared frontmatter `name` must match the \
             requested name). Rejects names that already exist anywhere in the merged \
             command-prompt scan, or a target path that already exists on disk (even one \
             the scanner would skip). The write is atomic and fans out \
             directory::prompts::on-change with { op: \"create\" }. Use \
             directory::prompts::update to edit existing prompts.",
            "Create prompt",
        ),
        fs_source::PromptKind::System => (
            "Create a NEW system prompt at <skills_folder>/system-prompts/<name>.md from \
             full-file markdown content (frontmatter block included; a non-empty \
             `description` is required, and a declared frontmatter `name` must match the \
             requested name). Rejects names that already exist anywhere in the merged \
             system-prompt scan, or a target path that already exists on disk (even one \
             the scanner would skip). The write is atomic and fans out \
             directory::system-prompts::on-change with { op: \"create\" }. Use \
             directory::system-prompts::update to edit existing system prompts.",
            "Create system prompt",
        ),
    };
    iii.register_function(
        function_id,
        RegisterFunction::new_async(move |req: PromptCreateInput| {
            let cfg = cfg_inner.load_full();
            let iii = iii_inner.clone();
            let subs = subs_inner.clone();
            async move {
                let out = create_prompt(&cfg, &req, kind).map_err(Error::Handler)?;
                trigger_types::dispatch(&iii, &subs, json!({ "op": "create", "name": out.name }))
                    .await;
                Ok::<_, Error>(out)
            }
        })
        .description(description)
        .metadata(json!({"tool": {"label": label}})),
    );
}

fn register_delete_system_prompt(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    subs: &trigger_types::SubscriberSet,
) {
    let cfg_inner = cfg.clone();
    let iii_inner = iii.clone();
    let subs_inner = subs.clone();
    iii.register_function(
        "directory::system-prompts::delete",
        RegisterFunction::new_async(move |req: PromptDeleteInput| {
            let cfg = cfg_inner.load_full();
            let iii = iii_inner.clone();
            let subs = subs_inner.clone();
            async move {
                let out = delete_prompt(&cfg, &req, fs_source::PromptKind::System)
                    .map_err(Error::Handler)?;
                trigger_types::dispatch(&iii, &subs, json!({ "op": "delete", "name": out.name }))
                    .await;
                Ok::<_, Error>(out)
            }
        })
        .description(
            "Permanently delete one EXISTING filesystem-backed system prompt by name. \
             Resolves against the same merged scan as directory::system-prompts::list, \
             removes only that prompt's markdown file, and fans out \
             directory::system-prompts::on-change with { op: \"delete\" }.",
        )
        .metadata(json!({"tool": {"label": "Delete system prompt"}})),
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

/// Prompt update against the merged (global + local) prompt view for
/// `kind` — the same view `directory::prompts::list` /
/// `directory::system-prompts::list` serves.
pub fn update_prompt(
    cfg: &SkillsConfig,
    req: &PromptUpdateInput,
    kind: fs_source::PromptKind,
) -> Result<PromptUpdateOutput, String> {
    validate_name(&req.name)?;
    let (prompts, _skipped) = fs_source::scan_prompts_merged(
        &cfg.resolved_skills_folder(),
        &cfg.local_skills_folder(),
        kind,
    );
    let Some(fs) = prompts.iter().find(|p| p.name == req.name) else {
        let names: Vec<String> = prompts.iter().map(|p| p.name.clone()).collect();
        let candidates = closest_ids(&names, &req.name, 3);
        return Err(not_found_message(
            "D210",
            kind.noun(),
            &req.name,
            &candidates,
            update_not_found_next(kind),
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

/// Create a NEW prompt file under `<skills_folder>/<kind.segment()>/<name>.md`
/// (`prompts/` for command templates, `system-prompts/` for system
/// prompts). Rejects names already visible in the merged (global +
/// local) scan for THIS kind — prompt names are a flat space per kind
/// (the same name may exist as both a command prompt and a system
/// prompt) and `get` resolves by name within a kind only — and target
/// paths that already exist on disk even when the scanner skips them,
/// so create can never clobber a file.
pub fn create_prompt(
    cfg: &SkillsConfig,
    req: &PromptCreateInput,
    kind: fs_source::PromptKind,
) -> Result<PromptCreateOutput, String> {
    validate_name(&req.name)?;
    let (prompts, _skipped) = fs_source::scan_prompts_merged(
        &cfg.resolved_skills_folder(),
        &cfg.local_skills_folder(),
        kind,
    );
    if prompts.iter().any(|p| p.name == req.name) {
        return Err(invalid_input_message(
            "D214",
            &format!("{} {:?} already exists.", kind.noun(), req.name),
            create_conflict_next(kind),
        ));
    }
    let dest = cfg
        .resolved_skills_folder()
        .join(kind.segment())
        .join(format!("{}.md", req.name));
    if dest.exists() {
        return Err(invalid_input_message(
            "D214",
            &format!(
                "a file already exists at {} (currently skipped by the scanner); edit or \
                 remove it on disk.",
                dest.display()
            ),
            create_conflict_next(kind),
        ));
    }
    let description = validate_new_prompt_content(&req.name, &req.content, kind)?;

    write_file_atomic(&dest, req.content.as_bytes())?;

    Ok(PromptCreateOutput {
        name: req.name.clone(),
        description,
        bytes: req.content.len(),
        modified_at: fs_modified_at(&dest),
    })
}

/// Delete one prompt resolved through the same merged view as list/get.
pub fn delete_prompt(
    cfg: &SkillsConfig,
    req: &PromptDeleteInput,
    kind: fs_source::PromptKind,
) -> Result<PromptDeleteOutput, String> {
    validate_name(&req.name)?;
    let (prompts, _skipped) = fs_source::scan_prompts_merged(
        &cfg.resolved_skills_folder(),
        &cfg.local_skills_folder(),
        kind,
    );
    let Some(fs) = prompts.iter().find(|prompt| prompt.name == req.name) else {
        let names: Vec<String> = prompts.iter().map(|prompt| prompt.name.clone()).collect();
        let candidates = closest_ids(&names, &req.name, 3);
        return Err(not_found_message(
            "D210",
            kind.noun(),
            &req.name,
            &candidates,
            update_not_found_next(kind),
        ));
    };

    std::fs::remove_file(&fs.abs_path)
        .map_err(|error| format!("delete {}: {error}", fs.abs_path.display()))?;
    Ok(PromptDeleteOutput {
        name: req.name.clone(),
    })
}

/// Create-variant of [`validate_prompt_content`]: no file exists yet, so
/// the stem IS the requested name — and a declared frontmatter `name`
/// must equal it, so create can never materialise a file `get(name)`
/// would not find. Returns the description.
fn validate_new_prompt_content(
    name: &str,
    content: &str,
    kind: fs_source::PromptKind,
) -> Result<String, String> {
    let (fm, description) = validate_prompt_frontmatter(content)?;
    if let Some(declared) = fm.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if declared != name {
            return Err(invalid_input_message(
                "D213",
                &format!(
                    "frontmatter `name` ({declared:?}) must match the requested {} name \
                     ({name:?}).",
                    kind.noun()
                ),
                &[],
            ));
        }
    }
    Ok(description)
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

/// Shared prefix for both prompt-content validators: the raw content rules
/// via [`validate_skill_content`], then the REQUIRED frontmatter parse with
/// its non-empty `description`. Returns the parsed frontmatter (so callers
/// can layer their own name handling on top — `update` resolves an
/// effective/renameable name, `create` requires a declared name to match
/// the request exactly) plus the trimmed description.
fn validate_prompt_frontmatter(
    content: &str,
) -> Result<(fs_source::PromptFrontmatter, String), String> {
    validate_skill_content(content)?;
    let fm = fs_source::parse_prompt_frontmatter(content)
        .map_err(|e| invalid_input_message("D213", &format!("{e}."), &[]))?;
    let description = match fm.description.as_deref().map(str::trim) {
        Some(d) if !d.is_empty() => d.to_string(),
        _ => {
            return Err(invalid_input_message(
                "D213",
                "frontmatter is missing a non-empty `description`; the next scan would skip \
                 this prompt.",
                &[],
            ));
        }
    };
    Ok((fm, description))
}

/// Prompt-specific content rules on top of [`validate_skill_content`]:
/// the frontmatter the scanner requires must survive the write. Returns
/// the effective `(name, description)` the next scan will report.
fn validate_prompt_content(fs: &FsPrompt, content: &str) -> Result<(String, String), String> {
    let (fm, description) = validate_prompt_frontmatter(content)?;
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
            fs_source::PromptKind::Command,
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
                fs_source::PromptKind::Command,
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
            fs_source::PromptKind::Command,
        )
        .unwrap_err();
        assert!(err.starts_with("D210 not_found:"), "got: {err}");
        assert!(err.contains("real"), "got: {err}");
    }

    // ── prompt create ────────────────────────────────────────────────

    #[test]
    fn create_prompt_writes_file_visible_to_next_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());

        let content = "---\ndescription: Pirate identity.\n---\nTalk like a pirate.\n";
        let out = create_prompt(
            &cfg,
            &PromptCreateInput {
                name: "pirate".into(),
                content: content.into(),
            },
            fs_source::PromptKind::System,
        )
        .unwrap();
        assert_eq!(out.name, "pirate");
        assert_eq!(out.description, "Pirate identity.");
        assert_eq!(out.bytes, content.len());
        assert!(!out.modified_at.is_empty());

        let on_disk = std::fs::read_to_string(tmp.path().join("system-prompts/pirate.md")).unwrap();
        assert_eq!(on_disk, content);

        // The next merged scan (what list/get serve) sees it.
        let (prompts, skipped) = fs_source::scan_prompts_merged(
            &cfg.resolved_skills_folder(),
            &cfg.local_skills_folder(),
            fs_source::PromptKind::System,
        );
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");
        assert!(prompts.iter().any(|p| p.name == "pirate"));
    }

    #[test]
    fn create_prompt_rejects_existing_name_anywhere_in_merged_scan() {
        let tmp = tempfile::tempdir().unwrap();
        // Same name in a DIFFERENT namespace, SAME kind — names are flat
        // within a kind.
        write_fixture(
            tmp.path(),
            "ns/system-prompts/taken.md",
            "---\ndescription: x\n---\nBody.\n",
        );
        let cfg = cfg_for(tmp.path());

        let err = create_prompt(
            &cfg,
            &PromptCreateInput {
                name: "taken".into(),
                content: "---\ndescription: y\n---\nB.\n".into(),
            },
            fs_source::PromptKind::System,
        )
        .unwrap_err();
        assert!(err.starts_with("D214 invalid_input:"), "got: {err}");
        assert!(!tmp.path().join("system-prompts/taken.md").exists());
    }

    #[test]
    fn create_prompt_rejects_scanner_skipped_file_at_target_path() {
        let tmp = tempfile::tempdir().unwrap();
        // Present on disk but invisible to the scan (no frontmatter):
        // create must refuse rather than clobber it.
        write_fixture(tmp.path(), "system-prompts/ghost.md", "no frontmatter\n");
        let cfg = cfg_for(tmp.path());

        let err = create_prompt(
            &cfg,
            &PromptCreateInput {
                name: "ghost".into(),
                content: "---\ndescription: y\n---\nB.\n".into(),
            },
            fs_source::PromptKind::System,
        )
        .unwrap_err();
        assert!(err.starts_with("D214 invalid_input:"), "got: {err}");
        let on_disk = std::fs::read_to_string(tmp.path().join("system-prompts/ghost.md")).unwrap();
        assert_eq!(on_disk, "no frontmatter\n", "existing file untouched");
    }

    #[test]
    fn create_prompt_rejects_invalid_content_and_mismatched_name() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());

        for bad in [
            "no frontmatter at all\n",
            "---\nname: pirate\n---\nno description\n",
            // Declared frontmatter name must match the requested name.
            "---\nname: other-name\ndescription: x\n---\nbody\n",
        ] {
            let err = create_prompt(
                &cfg,
                &PromptCreateInput {
                    name: "pirate".into(),
                    content: bad.into(),
                },
                fs_source::PromptKind::System,
            )
            .unwrap_err();
            assert!(err.starts_with("D213 invalid_input:"), "got: {err}");
        }

        // Bad requested name never reaches the filesystem.
        assert!(create_prompt(
            &cfg,
            &PromptCreateInput {
                name: "Bad Name".into(),
                content: "---\ndescription: x\n---\nbody\n".into(),
            },
            fs_source::PromptKind::System,
        )
        .is_err());
        assert!(!tmp.path().join("system-prompts/pirate.md").exists());
    }

    #[test]
    fn create_prompt_kind_selects_target_dir_and_scope() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());
        let content = "---\ndescription: X.\n---\nBody.\n";

        let sys = create_prompt(
            &cfg,
            &PromptCreateInput {
                name: "dual".into(),
                content: content.into(),
            },
            fs_source::PromptKind::System,
        )
        .unwrap();
        assert_eq!(sys.name, "dual");
        assert!(tmp.path().join("system-prompts/dual.md").exists());

        // Same name as a COMMAND prompt: legal, lands in prompts/.
        let cmd = create_prompt(
            &cfg,
            &PromptCreateInput {
                name: "dual".into(),
                content: content.into(),
            },
            fs_source::PromptKind::Command,
        )
        .unwrap();
        assert_eq!(cmd.name, "dual");
        assert!(tmp.path().join("prompts/dual.md").exists());

        // But a second create of the same kind is a conflict.
        let err = create_prompt(
            &cfg,
            &PromptCreateInput {
                name: "dual".into(),
                content: content.into(),
            },
            fs_source::PromptKind::System,
        )
        .unwrap_err();
        assert!(err.starts_with("D214 invalid_input:"), "got: {err}");
        assert!(err.contains("system prompt"), "kind noun missing: {err}");
        assert!(
            err.contains("directory::system-prompts::update"),
            "next-action must stay in-family: {err}"
        );
    }

    #[test]
    fn update_prompt_is_kind_scoped() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "system-prompts/solo.md",
            "---\ndescription: old\n---\nOld.\n",
        );
        let cfg = cfg_for(tmp.path());
        // The command family cannot see (or update) a system prompt.
        let err = update_prompt(
            &cfg,
            &PromptUpdateInput {
                name: "solo".into(),
                content: "---\ndescription: new\n---\nNew.\n".into(),
            },
            fs_source::PromptKind::Command,
        )
        .unwrap_err();
        assert!(err.starts_with("D210 not_found: prompt"), "got: {err}");
        // The system family updates it.
        let out = update_prompt(
            &cfg,
            &PromptUpdateInput {
                name: "solo".into(),
                content: "---\ndescription: new\n---\nNew.\n".into(),
            },
            fs_source::PromptKind::System,
        )
        .unwrap();
        assert_eq!(out.description, "new");
    }

    #[test]
    fn delete_prompt_removes_only_the_resolved_kind() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "system-prompts/on-disk.md",
            "---\nname: dual\ndescription: system\n---\nSystem.\n",
        );
        write_fixture(
            tmp.path(),
            "prompts/dual.md",
            "---\ndescription: command\n---\nCommand.\n",
        );
        let cfg = cfg_for(tmp.path());

        let out = delete_prompt(
            &cfg,
            &PromptDeleteInput {
                name: "dual".into(),
            },
            fs_source::PromptKind::System,
        )
        .unwrap();

        assert_eq!(out.name, "dual");
        assert!(!tmp.path().join("system-prompts/on-disk.md").exists());
        assert!(tmp.path().join("prompts/dual.md").exists());

        let err = delete_prompt(
            &cfg,
            &PromptDeleteInput {
                name: "dual".into(),
            },
            fs_source::PromptKind::System,
        )
        .unwrap_err();
        assert!(
            err.starts_with("D210 not_found: system prompt"),
            "got: {err}"
        );
    }
}
