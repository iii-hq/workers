//! Single-file write paths for skills and system prompts.
//!
//! Public API (reachable by any worker over `iii.trigger`):
//!
//!   * `directory::skills::{update,create,delete}` — overwrite one
//!     EXISTING skill markdown file, author a NEW one under the global
//!     `skills_folder`, or permanently remove one.
//!   * `directory::system-prompts::{update,create,delete}` — the same
//!     trio for system prompts.
//!
//! Updates take the FULL raw file (frontmatter block included) — the same
//! string `directory::skills::get { raw: true }` / `directory::system-prompts::get
//! { raw: true }` return — so an editor round-trips the exact on-disk
//! bytes. Update never creates files — `create` is the explicit authoring
//! path; update only mutates what a read can already see. Resolution
//! mirrors the read path (merged global+local scan, the `<id>` →
//! `<id>/index` overview alias, the installed-worker visibility filter)
//! so update can never write a file `list`/`get` would hide. Skills
//! resolved to the read-only `agents_skills_folder` are refused by
//! update/delete (D116) — those are owned by external agent tooling —
//! and create never writes there.
//!
//! Content is validated against the READ invariants before the write —
//! anything this module accepts, the scanners and readers will serve:
//!
//!   * both: size cap ([`SKILL_BODY_MAX_BYTES`] on the raw file), body
//!     non-empty after frontmatter strip;
//!   * system prompts additionally: required YAML frontmatter with a non-empty
//!     `description`, and a valid `name` when the frontmatter declares
//!     one (the scanner would otherwise skip the file on next scan).
//!
//! Writes are atomic (tmp + rename, same as download) and fan out the
//! same `directory::skills::on-change` / `directory::system-prompts::on-change`
//! triggers with `{ op: "update", ... }` payloads.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
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
    filter_to_registered, find_fs_skill_in, normalize_get_id, reject_function_id_shaped,
    resolve_title, resolve_visible_skills, validate_id, RegisteredWorkersCache,
    SKILL_BODY_MAX_BYTES,
};
use crate::sources::{mark_self_write, write_file_atomic};
use crate::trigger_types;

/// Recovery pointers for a missed skill update/delete target.
const SKILL_UPDATE_NOT_FOUND_NEXT: &[NextAction] = &[
    NextAction::new("directory::skills::list", "browse skill ids"),
    NextAction::new(
        "directory::skills::download",
        "materialize a missing bundle first",
    ),
    NextAction::new("directory::skills::create", "author a new skill"),
];

/// Recovery pointers for a skill create that hit an existing id/path.
const SKILL_CREATE_CONFLICT_NEXT: &[NextAction] = &[
    NextAction::new("directory::skills::update", "edit the existing skill"),
    NextAction::new("directory::skills::list", "browse skill ids"),
];

/// Recovery pointer for a missed system-prompt update target.
const SYSTEM_PROMPT_UPDATE_NOT_FOUND_NEXT: &[NextAction] = &[NextAction::new(
    "directory::system-prompts::list",
    "browse system prompt names",
)];

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
pub struct SkillCreateInput {
    /// New skill id — becomes the file path
    /// (`<skills_folder>/<id>.md`), so it may contain `/`-separated
    /// segments (`[a-z0-9_-]` each). Must not collide with an existing
    /// visible skill (including system-installed agents skills) or an
    /// on-disk file at the target path.
    pub id: String,
    /// FULL file content, frontmatter block included — the same form
    /// `directory::skills::update` takes. Frontmatter is optional for
    /// skills; only the size cap and a non-empty body are enforced.
    pub content: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SkillCreateOutput {
    /// The created skill's id (normalized form of the input id).
    pub id: String,
    /// Title resolved from the content (frontmatter `title:`, then
    /// `name:`, then body H1, then the id) — what `list` rows will show.
    pub title: String,
    /// Frontmatter `type:` of the content, `null` when absent.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Frontmatter `function_id:` of the content, `null` when absent.
    pub function_id: Option<String>,
    /// Bytes written.
    pub bytes: usize,
    /// File mtime after the write, RFC 3339.
    pub modified_at: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SkillDeleteInput {
    /// Existing skill id — the same forms `directory::skills::get`
    /// accepts (bare id, `<id>.md`, `SKILL(S).md`, `iii://<id>`).
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SkillDeleteOutput {
    /// Id of the skill whose file was removed — the RESOLVED on-disk id
    /// (e.g. `ns/index` for input `ns`), not the input form.
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SystemPromptUpdateInput {
    /// System prompt name to overwrite, as returned by
    /// `directory::system-prompts::list`. The
    /// target file must already exist.
    pub name: String,
    /// FULL new file content, frontmatter block included. The
    /// frontmatter must keep a non-empty `description` — a system prompt
    /// without one would be skipped by the next scan.
    pub content: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SystemPromptUpdateOutput {
    /// The system prompt's EFFECTIVE name after the write: the new
    /// frontmatter `name:` when declared, otherwise the file stem.
    /// Differs from the input name when the update renames the system prompt.
    pub name: String,
    /// Description parsed from the new frontmatter.
    pub description: String,
    /// Bytes written.
    pub bytes: usize,
    /// File mtime after the write, RFC 3339.
    pub modified_at: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SystemPromptCreateInput {
    /// New system prompt name — becomes the file stem
    /// `<skills_folder>/system-prompts/<name>.md`. Same charset rules as
    /// list/get names; must not collide with an existing system prompt.
    pub name: String,
    /// FULL file content, frontmatter block included — the same form
    /// `directory::system-prompts::update` takes. Frontmatter must carry a
    /// non-empty `description`; a declared `name` must match the
    /// requested name.
    pub content: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SystemPromptCreateOutput {
    /// The created system prompt's name (as requested).
    pub name: String,
    /// Description parsed from the frontmatter.
    pub description: String,
    /// Bytes written.
    pub bytes: usize,
    /// File mtime after the write, RFC 3339.
    pub modified_at: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SystemPromptDeleteInput {
    /// Existing system prompt name, as returned by
    /// `directory::system-prompts::list`.
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SystemPromptDeleteOutput {
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
    register_create_skill(iii, cfg, &subscribers.skills, cache);
    register_delete_skill(iii, cfg, &subscribers.skills, cache);
    register_update_system_prompt(iii, cfg, &subscribers.system_prompts);
    register_create_system_prompt(iii, cfg, &subscribers.system_prompts);
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
                let out = update_skill_in(&visible, &req, &cfg.resolved_agents_skills_roots())
                    .map_err(Error::Handler)?;
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
             `get`. Never creates files (use directory::skills::create to author one, or \
             directory::skills::download to materialize a bundle). Refuses read-only \
             system-installed skills under agents_skills_folder. Content is validated \
             against the read invariants (size cap, non-empty body after frontmatter); \
             the write is atomic and fans out directory::skills::on-change with \
             { op: \"update\" }.",
        )
        .metadata(json!({"tool": {"label": "Update skill"}})),
    );
}

fn register_create_skill(
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
        "directory::skills::create",
        RegisterFunction::new_async(move |req: SkillCreateInput| {
            let cfg = cfg_inner.load_full();
            let iii = iii_inner.clone();
            let subs = subs_inner.clone();
            let cache = cache_inner.clone();
            async move {
                let visible = resolve_visible_skills(&cfg, &cache, &iii, false).await;
                // Mirror the read path: the filter guard only applies when
                // the filter is on AND the registered set is known — on
                // daemon-down reads serve unfiltered, so create does too.
                let registered = if cfg.filter_unregistered {
                    cache.get_or_fetch(&iii).await
                } else {
                    None
                };
                let out = create_skill_in(&cfg, &visible, registered.as_ref(), &req)
                    .map_err(Error::Handler)?;
                let namespace = out.id.split('/').next().unwrap_or("").to_string();
                trigger_types::dispatch(
                    &iii,
                    &subs,
                    json!({ "op": "create", "namespace": namespace, "id": out.id }),
                )
                .await;
                Ok::<_, Error>(out)
            }
        })
        .description(
            "Create a NEW filesystem-backed skill at <skills_folder>/<id>.md from \
             full-file markdown content (frontmatter optional; only the size cap and a \
             non-empty body are enforced, same as directory::skills::update). Rejects ids \
             that already exist anywhere in the visible skill set (including \
             system-installed agents skills), a target path that already exists on disk \
             (even one the scanner would skip), ids in a namespace reserved by a \
             system-installed agents skill, and — when filter_unregistered is on — ids \
             the visibility filter would immediately hide. The write is atomic and fans \
             out directory::skills::on-change with { op: \"create\" }. Use \
             directory::skills::update to edit existing skills.",
        )
        .metadata(json!({"tool": {"label": "Create skill"}})),
    );
}

fn register_delete_skill(
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
        "directory::skills::delete",
        RegisterFunction::new_async(move |req: SkillDeleteInput| {
            let cfg = cfg_inner.load_full();
            let iii = iii_inner.clone();
            let subs = subs_inner.clone();
            let cache = cache_inner.clone();
            async move {
                let visible = resolve_visible_skills(&cfg, &cache, &iii, false).await;
                let out = delete_skill_in(&cfg, &visible, &req).map_err(Error::Handler)?;
                let namespace = out.id.split('/').next().unwrap_or("").to_string();
                trigger_types::dispatch(
                    &iii,
                    &subs,
                    json!({ "op": "delete", "namespace": namespace, "id": out.id }),
                )
                .await;
                Ok::<_, Error>(out)
            }
        })
        .description(
            "Permanently delete one EXISTING filesystem-backed skill by id. Accepts the \
             same id forms as directory::skills::get, resolves against the same visible \
             set as directory::skills::list, refuses read-only system-installed skills \
             under agents_skills_folder, removes only that skill's markdown file (plus \
             any parent directories the removal left empty), and fans out \
             directory::skills::on-change with { op: \"delete\" }.",
        )
        .metadata(json!({"tool": {"label": "Delete skill"}})),
    );
}

fn register_update_system_prompt(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    subs: &trigger_types::SubscriberSet,
) {
    let cfg_inner = cfg.clone();
    let iii_inner = iii.clone();
    let subs_inner = subs.clone();
    iii.register_function(
        "directory::system-prompts::update",
        RegisterFunction::new_async(move |req: SystemPromptUpdateInput| {
            let cfg = cfg_inner.load_full();
            let iii = iii_inner.clone();
            let subs = subs_inner.clone();
            async move {
                let out = update_system_prompt(&cfg, &req).map_err(Error::Handler)?;
                trigger_types::dispatch(&iii, &subs, json!({ "op": "update", "name": out.name }))
                    .await;
                Ok::<_, Error>(out)
            }
        })
        .description(
            "Overwrite one EXISTING filesystem-backed system prompt with new full-file \
             markdown content. The frontmatter must keep a non-empty `description` (and \
             a valid `name` when it declares one) — the same rules the scanner enforces, \
             so an update can never produce a file the next \
             directory::system-prompts::list would skip. The write is atomic and fans \
             out directory::system-prompts::on-change with { op: \"update\" }. Returns \
             the system prompt's effective name after the write (frontmatter `name:` \
             wins over the file stem).",
        )
        .metadata(json!({"tool": {"label": "Update system prompt"}})),
    );
}

fn register_create_system_prompt(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    subs: &trigger_types::SubscriberSet,
) {
    let cfg_inner = cfg.clone();
    let iii_inner = iii.clone();
    let subs_inner = subs.clone();
    iii.register_function(
        "directory::system-prompts::create",
        RegisterFunction::new_async(move |req: SystemPromptCreateInput| {
            let cfg = cfg_inner.load_full();
            let iii = iii_inner.clone();
            let subs = subs_inner.clone();
            async move {
                let out = create_system_prompt(&cfg, &req).map_err(Error::Handler)?;
                trigger_types::dispatch(&iii, &subs, json!({ "op": "create", "name": out.name }))
                    .await;
                Ok::<_, Error>(out)
            }
        })
        .description(
            "Create a NEW system prompt at <skills_folder>/system-prompts/<name>.md from \
             full-file markdown content (frontmatter block included; a non-empty \
             `description` is required, and a declared frontmatter `name` must match the \
             requested name). Rejects names that already exist anywhere in the merged \
             system-prompt scan, or a target path that already exists on disk (even one \
             the scanner would skip). The write is atomic and fans out \
             directory::system-prompts::on-change with { op: \"create\" }. Use \
             directory::system-prompts::update to edit existing system prompts.",
        )
        .metadata(json!({"tool": {"label": "Create system prompt"}})),
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
        RegisterFunction::new_async(move |req: SystemPromptDeleteInput| {
            let cfg = cfg_inner.load_full();
            let iii = iii_inner.clone();
            let subs = subs_inner.clone();
            async move {
                let out = delete_system_prompt(&cfg, &req).map_err(Error::Handler)?;
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

/// Refuse writes to a skill resolved into a read-only agents root.
/// Checked on the RESOLVED `abs_path` (not the input id) so every input
/// alias (`bare`, `<id>/index`, `SKILL.md`, `iii://…`) hits the guard.
fn ensure_writable(fs: &FsSkill, agents_roots: &[PathBuf]) -> Result<(), String> {
    if let Some(agents_root) = agents_roots
        .iter()
        .find(|root| fs.abs_path.starts_with(root))
    {
        return Err(invalid_input_message(
            "D116",
            &format!(
                "skill {:?} is system-installed under an agents skills root ({}) and \
                 read-only; edit it with its owning tool, or copy it into skills_folder \
                 on disk to fork it.",
                fs.id,
                agents_root.display()
            ),
            &[],
        ));
    }
    Ok(())
}

/// Resolve `req.id` against an already-resolved visible set and write the
/// new content. Pure with respect to the engine: callers hand in the
/// visible skills, so tests drive it straight off a tempdir scan.
pub fn update_skill_in(
    visible: &[FsSkill],
    req: &SkillUpdateInput,
    agents_roots: &[PathBuf],
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
    ensure_writable(&fs, agents_roots)?;

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

/// Create a NEW skill file at `<skills_folder>/<id>.md`. Mirrors
/// [`create_system_prompt`]'s two-layer conflict check — the id must not resolve
/// in the visible set (the `<id>` → `<id>/index` alias catches a bare id
/// colliding with an existing overview doc, agents skills included), and
/// the target path must not exist on disk even when the scanner skips it.
///
/// Two guards keep the module invariant ("anything this module accepts,
/// the readers will serve") under the visibility filter:
///
/// * ids whose top segment is a system-installed agents namespace are
///   reserved — creating `<agents-skill>/…` under the global root would
///   shadow the whole read-only namespace;
/// * when `registered` is `Some` (the handler passes it only while
///   `cfg.filter_unregistered` is on), ids that [`filter_to_registered`]
///   would immediately hide are rejected. `None` skips the guard,
///   mirroring the read path's unfiltered daemon-down fallback.
pub fn create_skill_in(
    cfg: &SkillsConfig,
    visible: &[FsSkill],
    registered: Option<&HashSet<String>>,
    req: &SkillCreateInput,
) -> Result<SkillCreateOutput, String> {
    let id = normalize_get_id(&req.id)?;
    reject_function_id_shaped(&id)?;
    validate_id(&id)?;
    // A `prompts` / `system-prompts` segment would make the written file
    // invisible to every skills scan, breaking the module invariant that an
    // accepted write is a servable read.
    let classified = fs_source::classify_rel_path(Path::new(&format!("{id}.md")));
    if classified.is_none() {
        return Err(invalid_input_message(
            "D115",
            &format!(
                "id {id:?} contains the reserved `prompts` path segment; rename that \
                 segment before creating the skill.",
            ),
            &[],
        ));
    }
    if classified == Some(fs_source::SourceKind::SystemPrompt) {
        return Err(invalid_input_message(
            "D115",
            &format!(
                "id {id:?} contains a `system-prompts` path segment, which the scanner \
                 classifies as a prompt — the created file would be invisible to \
                 directory::skills::list. Use directory::system-prompts::create for \
                 prompt files.",
            ),
            &[],
        ));
    }
    if classified == Some(fs_source::SourceKind::Agent) {
        return Err(invalid_input_message(
            "D115",
            &format!(
                "id {id:?} contains an `agents` path segment, which the scanner \
                 classifies as an agent — the created file would be invisible to \
                 directory::skills::list. Use directory::agents::create for agent profiles.",
            ),
            &[],
        ));
    }
    if classified != Some(fs_source::SourceKind::Skill) {
        unreachable!("non-skill source kinds are handled above");
    }

    let agents_roots = cfg.resolved_agents_skills_roots();
    let top_seg = id.split('/').next().unwrap_or("");
    let mut agents_ns: Vec<String> = Vec::new();
    for agents_root in &agents_roots {
        let ns = fs_source::agents_namespaces(agents_root);
        if id.contains('/') && ns.iter().any(|ns| ns == top_seg) {
            return Err(invalid_input_message(
                "D115",
                &format!(
                    "namespace {top_seg:?} is reserved by a system-installed skill under \
                     an agents skills root ({}); edit it with its owning tool, or copy it \
                     into skills_folder on disk to fork it.",
                    agents_root.display()
                ),
                &[],
            ));
        }
        agents_ns.extend(ns);
    }
    if let Some(registered) = registered {
        let candidate = FsSkill {
            id: id.clone(),
            abs_path: std::path::PathBuf::new(),
        };
        if filter_to_registered(vec![candidate], registered, &agents_ns).is_empty() {
            return Err(invalid_input_message(
                "D115",
                &format!(
                    "id {id:?} would be hidden by filter_unregistered: its top namespace \
                     segment {top_seg:?} is not an installed worker. Use a single-segment \
                     id, install the worker first, or set filter_unregistered: false.",
                ),
                &[],
            ));
        }
    }

    if find_fs_skill_in(visible, &id).is_some() {
        return Err(invalid_input_message(
            "D114",
            &format!("skill {id:?} already exists."),
            SKILL_CREATE_CONFLICT_NEXT,
        ));
    }
    let dest = cfg.resolved_skills_folder().join(format!("{id}.md"));
    if dest.exists() {
        return Err(invalid_input_message(
            "D114",
            &format!(
                "a file already exists at {} (currently skipped by the scanner); edit or \
                 remove it on disk.",
                dest.display()
            ),
            SKILL_CREATE_CONFLICT_NEXT,
        ));
    }
    validate_skill_content(&req.content)?;

    write_file_atomic(&dest, req.content.as_bytes())?;

    let (fm_text, body) = fs_source::split_frontmatter(&req.content);
    let fm: SkillFrontmatter = fm_text
        .and_then(|t| serde_yaml::from_str(t).ok())
        .unwrap_or_default();
    let title = resolve_title(&fm, body, &id);
    Ok(SkillCreateOutput {
        id,
        title,
        kind: crate::functions::skills::clean_optional(fm.kind),
        function_id: crate::functions::skills::clean_optional(fm.function_id),
        bytes: req.content.len(),
        modified_at: fs_modified_at(&dest),
    })
}

/// Delete one skill resolved through the same visible view as list/get.
/// Refuses read-only agents-root skills, then removes the file plus any
/// parent directories the removal left empty — a leftover empty namespace
/// dir would otherwise keep shadowing the same namespace in a
/// lower-precedence root (agents) forever, so delete must undo what
/// create did.
pub fn delete_skill_in(
    cfg: &SkillsConfig,
    visible: &[FsSkill],
    req: &SkillDeleteInput,
) -> Result<SkillDeleteOutput, String> {
    let id = normalize_get_id(&req.id)?;
    reject_function_id_shaped(&id)?;
    validate_id(&id)?;

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
    ensure_writable(&fs, &cfg.resolved_agents_skills_roots())?;

    // Deletes have no atomic write to piggyback the self-write mark on;
    // mark explicitly so the watcher doesn't fire a spurious
    // `{ op: "external" }` on top of the precise `{ op: "delete" }`.
    mark_self_write(&fs.abs_path);
    std::fs::remove_file(&fs.abs_path)
        .map_err(|error| format!("delete {}: {error}", fs.abs_path.display()))?;
    remove_empty_parents(
        &fs.abs_path,
        &[&cfg.resolved_skills_folder(), &cfg.local_skills_folder()],
    );
    Ok(SkillDeleteOutput { id: fs.id.clone() })
}

/// Remove now-empty ancestor directories of `path`, stopping at (never
/// removing) any of `roots` and at the first non-empty directory —
/// `remove_dir` refuses non-empty dirs, which bounds the walk naturally.
fn remove_empty_parents(path: &Path, roots: &[&Path]) {
    let mut cur = path.parent();
    while let Some(dir) = cur {
        // A root is never removed, even when it is nested inside another
        // root (e.g. a local_skills_folder configured under the global
        // one) — check equality against ALL roots before the inside test.
        if roots.contains(&dir) {
            break;
        }
        let inside_a_root = roots.iter().any(|r| dir.starts_with(r));
        if !inside_a_root || std::fs::remove_dir(dir).is_err() {
            break;
        }
        cur = dir.parent();
    }
}

/// Update against the merged (global + local) system-prompt view served by
/// `directory::system-prompts::list`.
pub fn update_system_prompt(
    cfg: &SkillsConfig,
    req: &SystemPromptUpdateInput,
) -> Result<SystemPromptUpdateOutput, String> {
    validate_name(&req.name)?;
    let (prompts, _skipped) = fs_source::scan_system_prompts_merged(
        &cfg.resolved_skills_folder(),
        &cfg.local_skills_folder(),
    );
    let Some(fs) = prompts.iter().find(|p| p.name == req.name) else {
        // A bundled prompt has no file until first edited: updating it
        // copy-on-writes the local file, which shadows the bundled copy
        // from then on (deleting that file falls back to it again).
        if crate::bundled::bundled_system_prompt(&req.name).is_some() {
            let dest = cfg
                .resolved_skills_folder()
                .join("system-prompts")
                .join(format!("{}.md", req.name));
            if dest.exists() {
                return Err(invalid_input_message(
                    "D214",
                    &format!(
                        "a file already exists at {} (currently skipped by the scanner); \
                         edit or remove it on disk.",
                        dest.display()
                    ),
                    SYSTEM_PROMPT_CREATE_CONFLICT_NEXT,
                ));
            }
            let description = validate_new_prompt_content(&req.name, &req.content)?;
            write_file_atomic(&dest, req.content.as_bytes())?;
            return Ok(SystemPromptUpdateOutput {
                name: req.name.clone(),
                description,
                bytes: req.content.len(),
                modified_at: fs_modified_at(&dest),
            });
        }
        let names: Vec<String> = prompts.iter().map(|p| p.name.clone()).collect();
        let candidates = closest_ids(&names, &req.name, 3);
        return Err(not_found_message(
            "D210",
            "system prompt",
            &req.name,
            &candidates,
            SYSTEM_PROMPT_UPDATE_NOT_FOUND_NEXT,
        ));
    };
    let (effective_name, description) = validate_prompt_content(fs, &req.content)?;

    write_file_atomic(&fs.abs_path, req.content.as_bytes())?;

    Ok(SystemPromptUpdateOutput {
        name: effective_name,
        description,
        bytes: req.content.len(),
        modified_at: fs_modified_at(&fs.abs_path),
    })
}

/// Create a NEW system prompt under `<skills_folder>/system-prompts/<name>.md`.
/// Rejects names already visible in the merged (global + local) scan and target
/// paths that already exist on disk even when the scanner skips them, so create
/// can never clobber a file.
pub fn create_system_prompt(
    cfg: &SkillsConfig,
    req: &SystemPromptCreateInput,
) -> Result<SystemPromptCreateOutput, String> {
    validate_name(&req.name)?;
    let (prompts, _skipped) = fs_source::scan_system_prompts_merged(
        &cfg.resolved_skills_folder(),
        &cfg.local_skills_folder(),
    );
    if prompts.iter().any(|p| p.name == req.name) {
        return Err(invalid_input_message(
            "D214",
            &format!("system prompt {:?} already exists.", req.name),
            SYSTEM_PROMPT_CREATE_CONFLICT_NEXT,
        ));
    }
    let dest = cfg
        .resolved_skills_folder()
        .join("system-prompts")
        .join(format!("{}.md", req.name));
    if dest.exists() {
        return Err(invalid_input_message(
            "D214",
            &format!(
                "a file already exists at {} (currently skipped by the scanner); edit or \
                 remove it on disk.",
                dest.display()
            ),
            SYSTEM_PROMPT_CREATE_CONFLICT_NEXT,
        ));
    }
    let description = validate_new_prompt_content(&req.name, &req.content)?;

    write_file_atomic(&dest, req.content.as_bytes())?;

    Ok(SystemPromptCreateOutput {
        name: req.name.clone(),
        description,
        bytes: req.content.len(),
        modified_at: fs_modified_at(&dest),
    })
}

/// Delete one system prompt resolved through the same merged view as list/get.
pub fn delete_system_prompt(
    cfg: &SkillsConfig,
    req: &SystemPromptDeleteInput,
) -> Result<SystemPromptDeleteOutput, String> {
    validate_name(&req.name)?;
    let (prompts, _skipped) = fs_source::scan_system_prompts_merged(
        &cfg.resolved_skills_folder(),
        &cfg.local_skills_folder(),
    );
    let Some(fs) = prompts.iter().find(|prompt| prompt.name == req.name) else {
        let names: Vec<String> = prompts.iter().map(|prompt| prompt.name.clone()).collect();
        let candidates = closest_ids(&names, &req.name, 3);
        return Err(not_found_message(
            "D210",
            "system prompt",
            &req.name,
            &candidates,
            SYSTEM_PROMPT_UPDATE_NOT_FOUND_NEXT,
        ));
    };

    // Suppress the watcher's `{ op: "external" }` for our own delete —
    // the precise `{ op: "delete" }` fan-out already covers it.
    mark_self_write(&fs.abs_path);
    std::fs::remove_file(&fs.abs_path)
        .map_err(|error| format!("delete {}: {error}", fs.abs_path.display()))?;
    Ok(SystemPromptDeleteOutput {
        name: req.name.clone(),
    })
}

/// Create-variant of [`validate_prompt_content`]: no file exists yet, so
/// the stem IS the requested name — and a declared frontmatter `name`
/// must equal it, so create can never materialise a file `get(name)`
/// would not find. Returns the description.
fn validate_new_prompt_content(name: &str, content: &str) -> Result<String, String> {
    let (fm, description) = validate_prompt_frontmatter(content)?;
    if let Some(declared) = fm.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if declared != name {
            return Err(invalid_input_message(
                "D213",
                &format!(
                    "frontmatter `name` ({declared:?}) must match the requested system prompt name \
                     ({name:?})."
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

/// Shared prefix for the system-prompt content validators: the raw content rules
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

/// System-prompt content rules on top of [`validate_skill_content`]:
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
/// Levenshtein distance (same ranker the system-prompt reader uses).
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
            // in the test process CWD can't shadow the fixtures — and the
            // agents root too: its default resolves to the developer's
            // REAL ~/.agents/skills, which would leak into these tests.
            local_skills_folder: dir.join("local-empty").to_string_lossy().into_owned(),
            agents_skills_folder: dir.join("agents-empty").to_string_lossy().into_owned(),
            global_agents_skills_folder: dir
                .join("global-agents-empty")
                .to_string_lossy()
                .into_owned(),
            ..SkillsConfig::default()
        }
    }

    /// Agents roots that match nothing — for tests that only need
    /// `update_skill_in`'s read-only guard to stay out of the way.
    fn no_agents() -> Vec<std::path::PathBuf> {
        vec![std::path::PathBuf::from("/nonexistent-agents-root")]
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
            &no_agents(),
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
            &no_agents(),
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
            &no_agents(),
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
            &no_agents(),
        )
        .unwrap_err();
        assert!(empty.starts_with("D113 invalid_input:"), "got: {empty}");

        let huge = update_skill_in(
            &visible,
            &SkillUpdateInput {
                id: "ns/a".into(),
                content: "x".repeat(SKILL_BODY_MAX_BYTES + 1),
            },
            &no_agents(),
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
            &no_agents(),
        )
        .unwrap_err();
        assert!(err.contains("FUNCTION id"), "got: {err}");
    }

    // ── prompts ──────────────────────────────────────────────────────

    #[test]
    fn update_system_prompt_overwrites_and_returns_effective_name() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns/system-prompts/open-pr.md",
            "---\ndescription: Old.\n---\nOld body.\n",
        );
        let cfg = cfg_for(tmp.path());

        let new_content = "---\nname: open-pr-v2\ndescription: New.\n---\nNew body.\n";
        let out = update_system_prompt(
            &cfg,
            &SystemPromptUpdateInput {
                name: "open-pr".into(),
                content: new_content.into(),
            },
        )
        .unwrap();
        assert_eq!(out.name, "open-pr-v2", "frontmatter name wins");
        assert_eq!(out.description, "New.");
        let on_disk =
            std::fs::read_to_string(tmp.path().join("ns/system-prompts/open-pr.md")).unwrap();
        assert_eq!(on_disk, new_content);
    }

    #[test]
    fn update_system_prompt_rejects_content_the_scanner_would_skip() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns/system-prompts/keep.md",
            "---\ndescription: Fine.\n---\nBody.\n",
        );
        let cfg = cfg_for(tmp.path());

        for bad in [
            "no frontmatter at all\n",
            "---\nname: keep\n---\nno description\n",
            "---\nname: Bad Name\ndescription: x\n---\nbody\n",
        ] {
            let err = update_system_prompt(
                &cfg,
                &SystemPromptUpdateInput {
                    name: "keep".into(),
                    content: bad.into(),
                },
            )
            .unwrap_err();
            assert!(err.starts_with("D213 invalid_input:"), "got: {err}");
        }
        // Original untouched.
        let on_disk =
            std::fs::read_to_string(tmp.path().join("ns/system-prompts/keep.md")).unwrap();
        assert_eq!(on_disk, "---\ndescription: Fine.\n---\nBody.\n");
    }

    #[test]
    fn update_system_prompt_missing_target_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns/system-prompts/real.md",
            "---\ndescription: x\n---\nBody.\n",
        );
        let cfg = cfg_for(tmp.path());
        let err = update_system_prompt(
            &cfg,
            &SystemPromptUpdateInput {
                name: "reel".into(),
                content: "---\ndescription: y\n---\nBody.\n".into(),
            },
        )
        .unwrap_err();
        assert!(err.starts_with("D210 not_found:"), "got: {err}");
        assert!(err.contains("real"), "got: {err}");
    }

    /// Editing a bundled prompt that has no file yet copy-on-writes the
    /// local file; the next update goes through the ordinary path.
    #[test]
    fn update_bundled_prompt_creates_the_local_override() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());
        let out = update_system_prompt(
            &cfg,
            &SystemPromptUpdateInput {
                name: "iii-minimal".into(),
                content: "---\nname: iii-minimal\ndescription: mine\n---\nEdited.\n".into(),
            },
        )
        .unwrap();
        assert_eq!(out.name, "iii-minimal");
        let path = tmp.path().join("system-prompts/iii-minimal.md");
        assert!(path.exists());
        assert!(std::fs::read_to_string(&path).unwrap().contains("Edited."));

        // Now a real file exists: the ordinary update path overwrites it.
        update_system_prompt(
            &cfg,
            &SystemPromptUpdateInput {
                name: "iii-minimal".into(),
                content: "---\nname: iii-minimal\ndescription: mine\n---\nEdited twice.\n".into(),
            },
        )
        .unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("Edited twice."));
    }

    // ── prompt create ────────────────────────────────────────────────

    #[test]
    fn create_system_prompt_writes_file_visible_to_next_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());

        let content = "---\ndescription: Pirate identity.\n---\nTalk like a pirate.\n";
        let out = create_system_prompt(
            &cfg,
            &SystemPromptCreateInput {
                name: "pirate".into(),
                content: content.into(),
            },
        )
        .unwrap();
        assert_eq!(out.name, "pirate");
        assert_eq!(out.description, "Pirate identity.");
        assert_eq!(out.bytes, content.len());
        assert!(!out.modified_at.is_empty());

        let on_disk = std::fs::read_to_string(tmp.path().join("system-prompts/pirate.md")).unwrap();
        assert_eq!(on_disk, content);

        // The next merged scan (what list/get serve) sees it.
        let (prompts, skipped) = fs_source::scan_system_prompts_merged(
            &cfg.resolved_skills_folder(),
            &cfg.local_skills_folder(),
        );
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");
        assert!(prompts.iter().any(|p| p.name == "pirate"));
    }

    #[test]
    fn create_system_prompt_rejects_existing_name_anywhere_in_merged_scan() {
        let tmp = tempfile::tempdir().unwrap();
        // Same name in a DIFFERENT namespace, SAME kind — names are flat
        // within a kind.
        write_fixture(
            tmp.path(),
            "ns/system-prompts/taken.md",
            "---\ndescription: x\n---\nBody.\n",
        );
        let cfg = cfg_for(tmp.path());

        let err = create_system_prompt(
            &cfg,
            &SystemPromptCreateInput {
                name: "taken".into(),
                content: "---\ndescription: y\n---\nB.\n".into(),
            },
        )
        .unwrap_err();
        assert!(err.starts_with("D214 invalid_input:"), "got: {err}");
        assert!(!tmp.path().join("system-prompts/taken.md").exists());
    }

    #[test]
    fn create_system_prompt_rejects_scanner_skipped_file_at_target_path() {
        let tmp = tempfile::tempdir().unwrap();
        // Present on disk but invisible to the scan (no frontmatter):
        // create must refuse rather than clobber it.
        write_fixture(tmp.path(), "system-prompts/ghost.md", "no frontmatter\n");
        let cfg = cfg_for(tmp.path());

        let err = create_system_prompt(
            &cfg,
            &SystemPromptCreateInput {
                name: "ghost".into(),
                content: "---\ndescription: y\n---\nB.\n".into(),
            },
        )
        .unwrap_err();
        assert!(err.starts_with("D214 invalid_input:"), "got: {err}");
        let on_disk = std::fs::read_to_string(tmp.path().join("system-prompts/ghost.md")).unwrap();
        assert_eq!(on_disk, "no frontmatter\n", "existing file untouched");
    }

    #[test]
    fn create_system_prompt_rejects_invalid_content_and_mismatched_name() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());

        for bad in [
            "no frontmatter at all\n",
            "---\nname: pirate\n---\nno description\n",
            // Declared frontmatter name must match the requested name.
            "---\nname: other-name\ndescription: x\n---\nbody\n",
        ] {
            let err = create_system_prompt(
                &cfg,
                &SystemPromptCreateInput {
                    name: "pirate".into(),
                    content: bad.into(),
                },
            )
            .unwrap_err();
            assert!(err.starts_with("D213 invalid_input:"), "got: {err}");
        }

        // Bad requested name never reaches the filesystem.
        assert!(create_system_prompt(
            &cfg,
            &SystemPromptCreateInput {
                name: "Bad Name".into(),
                content: "---\ndescription: x\n---\nbody\n".into(),
            },
        )
        .is_err());
        assert!(!tmp.path().join("system-prompts/pirate.md").exists());
    }

    #[test]
    fn create_system_prompt_kind_selects_target_dir_and_scope() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());
        let content = "---\ndescription: X.\n---\nBody.\n";

        let sys = create_system_prompt(
            &cfg,
            &SystemPromptCreateInput {
                name: "dual".into(),
                content: content.into(),
            },
        )
        .unwrap();
        assert_eq!(sys.name, "dual");
        assert!(tmp.path().join("system-prompts/dual.md").exists());

        // A second create is a conflict.
        let err = create_system_prompt(
            &cfg,
            &SystemPromptCreateInput {
                name: "dual".into(),
                content: content.into(),
            },
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
    fn update_system_prompt_is_kind_scoped() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "system-prompts/solo.md",
            "---\ndescription: old\n---\nOld.\n",
        );
        let cfg = cfg_for(tmp.path());
        // The system family updates it.
        let out = update_system_prompt(
            &cfg,
            &SystemPromptUpdateInput {
                name: "solo".into(),
                content: "---\ndescription: new\n---\nNew.\n".into(),
            },
        )
        .unwrap();
        assert_eq!(out.description, "new");
    }

    #[test]
    fn delete_system_prompt_removes_only_the_resolved_kind() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "system-prompts/on-disk.md",
            "---\nname: dual\ndescription: system\n---\nSystem.\n",
        );
        let cfg = cfg_for(tmp.path());

        let out = delete_system_prompt(
            &cfg,
            &SystemPromptDeleteInput {
                name: "dual".into(),
            },
        )
        .unwrap();

        assert_eq!(out.name, "dual");
        assert!(!tmp.path().join("system-prompts/on-disk.md").exists());

        let err = delete_system_prompt(
            &cfg,
            &SystemPromptDeleteInput {
                name: "dual".into(),
            },
        )
        .unwrap_err();
        assert!(
            err.starts_with("D210 not_found: system prompt"),
            "got: {err}"
        );
    }

    // ── skill create / delete ────────────────────────────────────────

    #[test]
    fn create_skill_writes_file_visible_to_next_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());

        let content = "---\ntitle: Guide\ntype: how-to\n---\n# Guide\n\nBody.\n";
        let out = create_skill_in(
            &cfg,
            &[],
            None,
            &SkillCreateInput {
                id: "my-skill".into(),
                content: content.into(),
            },
        )
        .unwrap();
        assert_eq!(out.id, "my-skill");
        assert_eq!(out.title, "Guide");
        assert_eq!(out.kind.as_deref(), Some("how-to"));
        assert_eq!(out.bytes, content.len());
        assert!(!out.modified_at.is_empty());

        let on_disk = std::fs::read_to_string(tmp.path().join("my-skill.md")).unwrap();
        assert_eq!(on_disk, content);

        // The next scan (what list/get serve) sees it.
        let visible = scan(tmp.path());
        assert!(visible.iter().any(|s| s.id == "my-skill"));
    }

    #[test]
    fn create_skill_rejects_existing_id_including_skill_md_alias() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "ns/SKILL.md", "# Existing\n\nbody\n");
        let cfg = cfg_for(tmp.path());
        let visible = scan(tmp.path()); // ns/SKILL.md → id ns/index

        // Both the on-disk id and the bare alias collide.
        for id in ["ns/index", "ns"] {
            let err = create_skill_in(
                &cfg,
                &visible,
                None,
                &SkillCreateInput {
                    id: id.into(),
                    content: "# New\n\nbody\n".into(),
                },
            )
            .unwrap_err();
            assert!(err.starts_with("D114 invalid_input:"), "id {id}: {err}");
            assert!(
                err.contains("directory::skills::update"),
                "next-action missing for {id}: {err}"
            );
        }
    }

    #[test]
    fn create_skill_rejects_existing_file_at_target_path() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());
        // Present on disk but invisible to the scan (empty body): create
        // must refuse rather than clobber it.
        write_fixture(tmp.path(), "ghost.md", "");

        let err = create_skill_in(
            &cfg,
            &[],
            None,
            &SkillCreateInput {
                id: "ghost".into(),
                content: "# G\n\nbody\n".into(),
            },
        )
        .unwrap_err();
        assert!(err.starts_with("D114 invalid_input:"), "got: {err}");
        let on_disk = std::fs::read_to_string(tmp.path().join("ghost.md")).unwrap();
        assert_eq!(on_disk, "", "existing file untouched");
    }

    #[test]
    fn create_skill_rejects_empty_body_and_oversize() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());

        let empty = create_skill_in(
            &cfg,
            &[],
            None,
            &SkillCreateInput {
                id: "new-skill".into(),
                content: "---\ntitle: x\n---\n".into(),
            },
        )
        .unwrap_err();
        assert!(empty.starts_with("D113 invalid_input:"), "got: {empty}");

        let huge = create_skill_in(
            &cfg,
            &[],
            None,
            &SkillCreateInput {
                id: "new-skill".into(),
                content: "x".repeat(SKILL_BODY_MAX_BYTES + 1),
            },
        )
        .unwrap_err();
        assert!(huge.contains("too large"), "got: {huge}");
        assert!(!tmp.path().join("new-skill.md").exists());
    }

    #[test]
    fn create_skill_rejects_function_id_shaped_and_invalid_id() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());

        let err = create_skill_in(
            &cfg,
            &[],
            None,
            &SkillCreateInput {
                id: "sandbox::create".into(),
                content: "# X\n\nbody\n".into(),
            },
        )
        .unwrap_err();
        assert!(err.contains("FUNCTION id"), "got: {err}");

        assert!(create_skill_in(
            &cfg,
            &[],
            None,
            &SkillCreateInput {
                id: "Bad Name".into(),
                content: "# X\n\nbody\n".into(),
            },
        )
        .is_err());
    }

    #[test]
    fn create_skill_rejects_prompt_segment_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());

        for id in ["ns/prompts/foo", "prompts/foo"] {
            let err = create_skill_in(
                &cfg,
                &[],
                None,
                &SkillCreateInput {
                    id: id.into(),
                    content: "# X\n\nbody\n".into(),
                },
            )
            .unwrap_err();
            assert!(err.starts_with("D115 invalid_input:"), "id {id}: {err}");
            assert!(
                err.contains("reserved `prompts` path segment; rename that segment"),
                "id {id} should direct the caller to rename the reserved segment: {err}"
            );
            assert!(!err.contains("classified as a prompt"), "id {id}: {err}");
            assert!(!err.contains("directory::prompts::"), "id {id}: {err}");
        }

        let err = create_skill_in(
            &cfg,
            &[],
            None,
            &SkillCreateInput {
                id: "ns/system-prompts/foo".into(),
                content: "# X\n\nbody\n".into(),
            },
        )
        .unwrap_err();
        assert!(
            err.contains("directory::system-prompts::create"),
            "system prompt ids should keep their create guidance: {err}"
        );
        assert!(!tmp.path().join("ns").exists());
        assert!(!tmp.path().join("prompts").exists());
    }

    #[test]
    fn create_skill_rejects_agent_segment_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());

        for id in ["agents/foo", "ns/agents/foo"] {
            let err = create_skill_in(
                &cfg,
                &[],
                None,
                &SkillCreateInput {
                    id: id.into(),
                    content: "# X\n\nbody\n".into(),
                },
            )
            .unwrap_err();
            assert!(err.starts_with("D115 invalid_input:"), "id {id}: {err}");
            assert!(err.contains("directory::agents::create"), "id {id}: {err}");
        }

        assert!(!tmp.path().join("agents").exists());
        assert!(!tmp.path().join("ns").exists());
    }

    #[test]
    fn create_skill_ignores_skill_less_agents_dirs() {
        // A stray directory without a SKILL.md under the agents root
        // reserves nothing.
        let tmp = tempfile::tempdir().unwrap();
        let agents = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(agents.path().join("stray")).unwrap();
        let mut cfg = cfg_for(tmp.path());
        cfg.agents_skills_folder = agents.path().to_string_lossy().into_owned();

        create_skill_in(
            &cfg,
            &[],
            None,
            &SkillCreateInput {
                id: "stray/notes".into(),
                content: "# N\n\nbody\n".into(),
            },
        )
        .expect("skill-less agents dir must not reserve the namespace");
    }

    #[test]
    fn remove_empty_parents_never_removes_a_nested_root() {
        let tmp = tempfile::tempdir().unwrap();
        let global = tmp.path().to_path_buf();
        // Local root nested INSIDE the global root.
        let local = global.join("nested-local");
        let file = local.join("ns/doc.md");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "x").unwrap();
        std::fs::remove_file(&file).unwrap();

        remove_empty_parents(&file, &[&global, &local]);
        assert!(!local.join("ns").exists(), "empty ns dir is cleaned");
        assert!(
            local.exists(),
            "the nested local root itself must survive even when empty"
        );
    }

    #[test]
    fn create_skill_rejects_id_hidden_by_filter() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());
        let registered: HashSet<String> = HashSet::from(["other".to_string()]);

        // Multi-segment id under a non-installed namespace: would be
        // invisible to list/get the moment it was written.
        let err = create_skill_in(
            &cfg,
            &[],
            Some(&registered),
            &SkillCreateInput {
                id: "ghost/doc".into(),
                content: "# G\n\nbody\n".into(),
            },
        )
        .unwrap_err();
        assert!(err.starts_with("D115 invalid_input:"), "got: {err}");
        assert!(err.contains("filter_unregistered"), "got: {err}");
        assert!(!tmp.path().join("ghost/doc.md").exists());

        // Filter-exempt shapes pass: single-segment, directory/*, iii/*,
        // and installed-worker namespaces.
        for id in ["solo", "directory/note", "iii/note", "other/note"] {
            create_skill_in(
                &cfg,
                &[],
                Some(&registered),
                &SkillCreateInput {
                    id: id.into(),
                    content: "# OK\n\nbody\n".into(),
                },
            )
            .unwrap_or_else(|e| panic!("id {id} should pass the filter guard: {e}"));
        }
    }

    #[test]
    fn create_skill_rejects_agents_namespace() {
        let tmp = tempfile::tempdir().unwrap();
        let agents = tempfile::tempdir().unwrap();
        write_fixture(
            agents.path(),
            "impeccable/SKILL.md",
            "# Impeccable\n\nbody\n",
        );
        let mut cfg = cfg_for(tmp.path());
        cfg.agents_skills_folder = agents.path().to_string_lossy().into_owned();

        // A multi-segment id under the agents namespace would shadow the
        // whole read-only namespace — reserved, even with the filter off.
        let err = create_skill_in(
            &cfg,
            &[],
            None,
            &SkillCreateInput {
                id: "impeccable/notes".into(),
                content: "# N\n\nbody\n".into(),
            },
        )
        .unwrap_err();
        assert!(err.starts_with("D115 invalid_input:"), "got: {err}");
        assert!(err.contains("reserved"), "got: {err}");
        assert!(!tmp.path().join("impeccable").exists());

        // The bare id collides with the agents skill via the <id>/index
        // alias once the agents entry is in the visible set.
        let (merged, _) = fs_source::scan_skills_merged(
            &cfg.resolved_skills_folder(),
            &cfg.local_skills_folder(),
        );
        let (visible, _) = fs_source::merge_agents_roots(
            merged,
            &cfg.resolved_skills_folder(),
            &cfg.local_skills_folder(),
            &cfg.resolved_agents_skills_roots(),
        );
        let err = create_skill_in(
            &cfg,
            &visible,
            None,
            &SkillCreateInput {
                id: "impeccable".into(),
                content: "# I\n\nbody\n".into(),
            },
        )
        .unwrap_err();
        assert!(err.starts_with("D114 invalid_input:"), "got: {err}");
    }

    #[test]
    fn delete_skill_removes_file_and_cleans_empty_parents() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());
        write_fixture(tmp.path(), "ns/deep/doc.md", "# Doc\n\nbody\n");
        write_fixture(tmp.path(), "keep/other.md", "# Other\n\nbody\n");
        let visible = scan(tmp.path());

        let out = delete_skill_in(
            &cfg,
            &visible,
            &SkillDeleteInput {
                id: "ns/deep/doc".into(),
            },
        )
        .unwrap();
        assert_eq!(out.id, "ns/deep/doc");
        assert!(!tmp.path().join("ns/deep/doc.md").exists());
        // Empty parents are cleaned up so the namespace can't keep
        // shadowing a lower-precedence root…
        assert!(!tmp.path().join("ns").exists());
        // …but the root itself and non-empty siblings survive.
        assert!(tmp.path().exists());
        assert!(tmp.path().join("keep/other.md").exists());
    }

    #[test]
    fn delete_skill_resolves_bare_alias_to_index() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());
        write_fixture(tmp.path(), "ns/index.md", "# Overview\n\nbody\n");
        let visible = scan(tmp.path());

        let out = delete_skill_in(&cfg, &visible, &SkillDeleteInput { id: "ns".into() }).unwrap();
        assert_eq!(out.id, "ns/index", "returns the resolved on-disk id");
        assert!(!tmp.path().join("ns").exists());
    }

    #[test]
    fn delete_skill_missing_target_is_not_found_with_candidates() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());
        write_fixture(tmp.path(), "ns/real.md", "# Real\n\nbody\n");
        let visible = scan(tmp.path());

        let err = delete_skill_in(
            &cfg,
            &visible,
            &SkillDeleteInput {
                id: "ns/reel".into(),
            },
        )
        .unwrap_err();
        assert!(err.starts_with("D110 not_found:"), "got: {err}");
        assert!(err.contains("ns/real"), "got: {err}");
        assert!(tmp.path().join("ns/real.md").exists());
    }

    #[test]
    fn update_and_delete_refuse_agents_root_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let agents = tempfile::tempdir().unwrap();
        write_fixture(
            agents.path(),
            "impeccable/SKILL.md",
            "# Impeccable\n\nbody\n",
        );
        let mut cfg = cfg_for(tmp.path());
        cfg.agents_skills_folder = agents.path().to_string_lossy().into_owned();

        let (visible, skipped) = fs_source::scan_agents_skills(agents.path());
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");

        // Every input alias resolves to the same abs_path and is refused.
        for id in ["impeccable", "impeccable/index"] {
            let err = update_skill_in(
                &visible,
                &SkillUpdateInput {
                    id: id.into(),
                    content: "# Hacked\n\nbody\n".into(),
                },
                &cfg.resolved_agents_skills_roots(),
            )
            .unwrap_err();
            assert!(err.starts_with("D116 invalid_input:"), "id {id}: {err}");

            let err =
                delete_skill_in(&cfg, &visible, &SkillDeleteInput { id: id.into() }).unwrap_err();
            assert!(err.starts_with("D116 invalid_input:"), "id {id}: {err}");
        }
        // The file survives untouched.
        let on_disk = std::fs::read_to_string(agents.path().join("impeccable/SKILL.md")).unwrap();
        assert_eq!(on_disk, "# Impeccable\n\nbody\n");
    }
}
