//! Filesystem-backed agent profiles (`directory::agents::*`).
//!
//! An agent profile is a reusable identity selected for a session: a system prompt
//! (the file body), a display name, a description, an emoji logo, and a
//! skill filter — one direct `<agents_folder>/<id>.md` file with required
//! YAML frontmatter (see `docs/architecture/agent-profile-storage.md`).
//!
//! Profiles compose: `extends: <id>` makes a profile's resolved system
//! prompt its parent's resolved prompt followed by its own body, with
//! `skills` / `model` / `reasoning_effort` falling back up the chain when
//! omitted (display fields never inherit). The chain is resolved here, on
//! every read, so the harness always receives a finished prompt. The base
//! of most chains is a bundled profile embedded in this binary — `iii` (the
//! harness default identity) or `iii-minimal` (the minimal directory-first
//! identity): always listed, `builtin: true` until a local file with the
//! same id shadows it. Five filesystem-backed verbs:
//!
//!   * `directory::agents::list`   — metadata-only listing, chain-resolved.
//!   * `directory::agents::get`    — one agent profile's resolved system
//!     prompt + metadata, plus `unknown_skills` (ids that resolve to nothing
//!     — warnings, never load failures) and `inheritance_error` (a chain that
//!     does not resolve — the profile still serves from its own file so it
//!     can be fixed; the harness refuses to run it).
//!   * `directory::agents::create` / `update` / `delete` — full-file
//!     writes, atomic, fanning out `directory::agents::on-change`.
//!
//! Not to be confused with the read-only `agents_skills_folder` config
//! root (`~/.agents/skills`) — that is an external tool's *skills*
//! convention; agent profiles live in the dedicated `agents_folder`.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::{SharedConfig, SkillsConfig};
use crate::fs_source::{self, FsAgent, FsSkill};
use crate::functions::error::{invalid_input_message, not_found_message, NextAction};
use crate::functions::prompts::validate_name;
use crate::functions::skills::{
    find_fs_skill_in, resolve_visible_skills, RegisteredWorkersCache, SKILL_BODY_MAX_BYTES,
};
use crate::sources::{mark_self_write, write_file_atomic};
use crate::trigger_types;

/// Recovery pointer attached to a `directory::agents::get` / write miss.
const AGENT_NOT_FOUND_NEXT: &[NextAction] = &[NextAction::new(
    "directory::agents::list",
    "browse agent profile ids",
)];

const AGENT_CREATE_CONFLICT_NEXT: &[NextAction] = &[
    NextAction::new(
        "directory::agents::update",
        "edit the existing agent profile",
    ),
    NextAction::new("directory::agents::list", "browse agent profile ids"),
];

// ---------- wire shapes ----------

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct ListAgentsInput {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AgentEntry {
    /// Flat agent profile id (the file stem) — what `harness::send { options:
    /// { agent } }` will take.
    pub id: String,
    /// Display name from frontmatter (`name:`).
    pub name: String,
    pub description: String,
    /// Emoji logo, `null` when the agent profile has none.
    pub logo: Option<String>,
    /// Length of the agent profile's skill filter, resolved through
    /// `extends`; `null` = no filter (every skill).
    pub skill_count: Option<usize>,
    /// Model id for sessions using this profile, resolved through
    /// `extends`; `null` = the send decides.
    pub model: Option<String>,
    /// Provider-native reasoning effort paired with `model`, resolved
    /// through `extends`; `null` = the model/provider default.
    pub reasoning_effort: Option<String>,
    /// Harness subagent icon token for spawn display identities;
    /// `null` = caller picks.
    pub icon: Option<String>,
    /// Harness subagent color token for display identities; `null` =
    /// neutral.
    pub color: Option<String>,
    /// Parent profile id (`extends:`), as declared; `null` = none.
    pub extends: Option<String>,
    /// Bundled with the worker, no file behind it: editing it creates the
    /// local file (which then shadows this entry); there is nothing to
    /// delete.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub builtin: bool,
    /// Set when the `extends` chain does not resolve (unknown parent, loop,
    /// too deep): the row carries the profile's own fields only, and the
    /// harness refuses to run it until the chain is fixed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inheritance_error: Option<String>,
    /// File mtime as RFC 3339; empty for a bundled profile.
    pub modified_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListAgentsOutput {
    pub agents: Vec<AgentEntry>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentGetInput {
    pub id: String,
    /// When `true`, the response includes the FULL on-disk file content
    /// (frontmatter block included) as `raw` — the exact string to hand
    /// back to `directory::agents::update`.
    #[serde(default)]
    pub raw: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AgentGetOutput {
    pub id: String,
    pub name: String,
    pub description: String,
    pub logo: Option<String>,
    /// The RESOLVED identity: every ancestor's body root-first, then this
    /// file's body, frontmatter stripped, joined by a blank line. A profile
    /// without `extends` serves its own body verbatim.
    pub system_prompt: String,
    /// The skill filter, resolved through `extends` (the nearest profile
    /// with a non-empty filter). Empty = every skill.
    pub skills: Vec<String>,
    /// Filter entries that resolve to no currently visible skill.
    /// Warnings — the agent profile still loads.
    pub unknown_skills: Vec<String>,
    /// Model id for sessions using this profile, resolved through `extends`;
    /// `null` = the send decides. Served verbatim — resolution against the
    /// live model catalog happens where it is used.
    pub model: Option<String>,
    /// Provider-native reasoning effort paired with `model`, resolved
    /// through `extends`; `null` = the model/provider default. Served
    /// verbatim and validated at use time.
    pub reasoning_effort: Option<String>,
    /// Harness subagent icon token (closed set, validated at write
    /// time); `null` = caller picks.
    pub icon: Option<String>,
    /// Harness subagent color token (closed set, validated at write
    /// time); `null` = neutral.
    pub color: Option<String>,
    /// Parent profile id (`extends:`), as declared; `null` = none.
    pub extends: Option<String>,
    /// Bundled with the worker, no file behind it (see `list`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub builtin: bool,
    /// Set when the `extends` chain does not resolve (unknown parent, loop,
    /// deeper than 8): `system_prompt` and the inherited fields then come
    /// from this file alone, `raw` still round-trips so the chain can be
    /// fixed, and the harness refuses to run the profile meanwhile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inheritance_error: Option<String>,
    /// FULL on-disk file content (this profile's own file, ancestors
    /// excluded). Present only when the request set `raw: true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    /// File mtime as RFC 3339; empty for a bundled profile.
    pub modified_at: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentCreateInput {
    /// New agent profile id — becomes the file stem
    /// (`<agents_folder>/<id>.md`). Lowercase ASCII, digits,
    /// `-` and `_` only. Must not collide with an existing agent profile or an
    /// on-disk file at the target path.
    pub id: String,
    /// FULL file content, frontmatter block included. Frontmatter must
    /// carry a non-empty `name`; the body is the system prompt and must
    /// be non-empty. An `extends:` that does not resolve is reported by
    /// `list`/`get` as `inheritance_error`, never a write error.
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentUpdateInput {
    /// Existing agent profile id, as returned by `directory::agents::list`.
    pub id: String,
    /// FULL new file content, frontmatter block included — the string
    /// `directory::agents::get { raw: true }` returns, edited.
    pub content: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AgentWriteOutput {
    pub id: String,
    /// Display name parsed from the (new) frontmatter.
    pub name: String,
    pub description: String,
    pub logo: Option<String>,
    /// Bytes written.
    pub bytes: usize,
    /// File mtime after the write, RFC 3339.
    pub modified_at: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentDeleteInput {
    /// Existing agent profile id, as returned by `directory::agents::list`.
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AgentDeleteOutput {
    pub id: String,
}

// ---------- registration ----------

pub fn register(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    subs: &super::Subscribers,
    cache: &Arc<RegisteredWorkersCache>,
) {
    register_list(iii, cfg);
    register_get(iii, cfg, cache);
    register_create(iii, cfg, &subs.agents);
    register_update(iii, cfg, &subs.agents);
    register_delete(iii, cfg, &subs.agents);
}

fn register_list(iii: &Arc<IIIClient>, cfg: &SharedConfig) {
    let cfg_inner = cfg.clone();
    iii.register_function(
        "directory::agents::list",
        RegisterFunction::new_async(move |_input: ListAgentsInput| {
            let cfg = cfg_inner.load_full();
            async move { Ok::<_, Error>(list_agents(&cfg)) }
        })
        .description(
            "List agent profiles (id, display name, description, emoji logo, icon, color, \
             model, reasoning_effort, skill_count, extends, modified_at) from the configured \
             agents folder plus the profiles bundled with this worker (`builtin: true` until a \
             local file shadows one — `iii` is the base identity most profiles extend). An \
             agent profile is a reusable session identity: its file body is the system \
             prompt. model / reasoning_effort / skill_count are resolved through `extends` \
             (a profile inherits what it omits from its parent chain); skill_count null = \
             sessions using the profile can use every skill. A row whose `extends` chain \
             does not resolve carries `inheritance_error`.",
        ),
    );
}

fn register_get(iii: &Arc<IIIClient>, cfg: &SharedConfig, cache: &Arc<RegisteredWorkersCache>) {
    let cfg_inner = cfg.clone();
    let iii_inner = iii.clone();
    let cache_inner = cache.clone();
    iii.register_function(
        "directory::agents::get",
        RegisterFunction::new_async(move |req: AgentGetInput| {
            let cfg = cfg_inner.load_full();
            let iii = iii_inner.clone();
            let cache = cache_inner.clone();
            async move {
                let visible = resolve_visible_skills(&cfg, &cache, &iii, false).await;
                get_agent(&cfg, req, &visible).map_err(Error::Handler)
            }
        })
        .description(
            "Fetch one agent profile by id. Returns the RESOLVED system prompt (each \
             ancestor's body root-first, then this file's body), display name, \
             description, emoji logo, the skill filter resolved \
             through `extends` plus unknown_skills (filter entries matching no visible \
             skill — warnings, the profile still loads), model, reasoning_effort, icon, \
             color, extends, builtin, and modified_at. `inheritance_error` is set when the \
             `extends` chain names an unknown profile, loops, or is deeper than 8 levels: \
             the profile then serves from its own file only and the harness refuses to \
             run it. Pass raw: true to also get the exact on-disk file (this profile's \
             own, ancestors excluded) for editing with directory::agents::update.",
        ),
    );
}

fn register_create(iii: &Arc<IIIClient>, cfg: &SharedConfig, subs: &trigger_types::SubscriberSet) {
    let cfg_inner = cfg.clone();
    let iii_inner = iii.clone();
    let subs_inner = subs.clone();
    iii.register_function(
        "directory::agents::create",
        RegisterFunction::new_async(move |req: AgentCreateInput| {
            let cfg = cfg_inner.load_full();
            let iii = iii_inner.clone();
            let subs = subs_inner.clone();
            async move {
                let out = create_agent(&cfg, &req).map_err(Error::Handler)?;
                trigger_types::dispatch(&iii, &subs, json!({ "op": "create", "name": out.id }))
                    .await;
                Ok::<_, Error>(out)
            }
        })
        .description(
            "Create a NEW agent profile at <agents_folder>/<id>.md from full-file \
             markdown content (frontmatter block included; a non-empty `name` is \
             required, `logo` is emoji-only, and the body — the system prompt — must be \
             non-empty; an `extends: <id>` that does not resolve is reported by list/get \
             as `inheritance_error`). Rejects ids that already exist in the \
             configured agents folder, or a target path that already exists on disk; \
             creating a bundled id shadows the bundled copy. The write is atomic and \
             fans out directory::agents::on-change with { op: \"create\" }.",
        )
        .metadata(json!({"tool": {"label": "Create agent profile"}})),
    );
}

fn register_update(iii: &Arc<IIIClient>, cfg: &SharedConfig, subs: &trigger_types::SubscriberSet) {
    let cfg_inner = cfg.clone();
    let iii_inner = iii.clone();
    let subs_inner = subs.clone();
    iii.register_function(
        "directory::agents::update",
        RegisterFunction::new_async(move |req: AgentUpdateInput| {
            let cfg = cfg_inner.load_full();
            let iii = iii_inner.clone();
            let subs = subs_inner.clone();
            async move {
                let out = update_agent(&cfg, &req).map_err(Error::Handler)?;
                trigger_types::dispatch(&iii, &subs, json!({ "op": "update", "name": out.id }))
                    .await;
                Ok::<_, Error>(out)
            }
        })
        .description(
            "Overwrite one EXISTING agent profile with new full-file markdown content — \
             the same rules the scanner enforces (required frontmatter with a non-empty \
             `name`, emoji-only `logo`, non-empty body), so an \
             update can never produce a file the next directory::agents::list would skip. \
             The agent profile id stays the file stem; frontmatter `name` is only the \
             display name. Updating a bundled profile (`builtin: true`) creates the local \
             file, which then shadows the bundled copy. The write is atomic and fans out \
             directory::agents::on-change with { op: \"update\" }.",
        )
        .metadata(json!({"tool": {"label": "Update agent profile"}})),
    );
}

fn register_delete(iii: &Arc<IIIClient>, cfg: &SharedConfig, subs: &trigger_types::SubscriberSet) {
    let cfg_inner = cfg.clone();
    let iii_inner = iii.clone();
    let subs_inner = subs.clone();
    iii.register_function(
        "directory::agents::delete",
        RegisterFunction::new_async(move |req: AgentDeleteInput| {
            let cfg = cfg_inner.load_full();
            let iii = iii_inner.clone();
            let subs = subs_inner.clone();
            async move {
                let out = delete_agent(&cfg, &req).map_err(Error::Handler)?;
                trigger_types::dispatch(&iii, &subs, json!({ "op": "delete", "name": out.id }))
                    .await;
                Ok::<_, Error>(out)
            }
        })
        .description(
            "Permanently delete one EXISTING agent profile by id. Resolves against the \
             same merged scan as directory::agents::list, removes only that profile's \
             markdown file, and fans out directory::agents::on-change with \
             { op: \"delete\" }. Deleting the local shadow of a bundled profile falls \
             back to the bundled copy; a bundled profile with no local file has nothing \
             to delete. Sessions already using this profile are not affected; profiles \
             that extend it stop resolving until their `extends` is fixed.",
        )
        .metadata(json!({"tool": {"label": "Delete agent profile"}})),
    );
}

// ---------- core helpers (engine-free, reusable in tests) ----------

/// Max `extends` hops. Enough for any sane hierarchy; small enough that a
/// runaway chain is an error instead of a slow read.
const MAX_EXTENDS_DEPTH: usize = 8;

/// Filesystem-only scan (both roots). The write verbs resolve FILES through
/// this; everything served goes through [`catalog`].
fn scan_profiles(cfg: &SkillsConfig) -> Vec<FsAgent> {
    fs_source::scan_agents_merged(&cfg.resolved_agents_roots()).0
}

/// The merged view every read serves: the scanned roots plus each bundled
/// profile no root shadows (`builtin: true`), sorted by id — the contract
/// the bundled system prompts already follow.
fn catalog(cfg: &SkillsConfig) -> Vec<FsAgent> {
    let mut agents = scan_profiles(cfg);
    for bundled in crate::bundled::bundled_agents() {
        if !agents.iter().any(|a| a.name == bundled.name) {
            agents.push(bundled);
        }
    }
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    agents
}

// Unlike the skills roots (`.agents/skills` is owned by external agent
// tooling and stays read-only), `~/.iii/agents` is iii's own directory:
// update and delete resolve to whichever root holds the profile and write it
// IN PLACE. Only create is anchored — it always writes `agents_folder`, and
// never materializes the global root. A bundled profile has no file until an
// update copy-on-writes the local shadow into `agents_folder`.

/// D415: `child`'s `extends` chain does not resolve. Carried by `list`/`get`
/// as `inheritance_error`; the harness refuses to run the profile on it.
fn inheritance_error(child: &str, problem: &str) -> String {
    invalid_input_message(
        "D415",
        &format!("agent profile {child:?} {problem}"),
        AGENT_NOT_FOUND_NEXT,
    )
}

/// The `extends` chain for `child`, nearest first: `[child, parent, …]`.
/// A chain that does not resolve — unknown parent, loop, more than
/// [`MAX_EXTENDS_DEPTH`] hops — yields `([child], Some(D415))`: the profile
/// still serves from its own file (the editor must be able to open it to
/// fix the chain), the error rides along, and the harness refuses to run it.
fn resolve_chain<'a>(
    catalog: &'a [FsAgent],
    child: &'a FsAgent,
) -> (Vec<&'a FsAgent>, Option<String>) {
    let mut chain = vec![child];
    while let Some(parent_id) = chain.last().and_then(|a| a.extends.as_deref()) {
        let trail = chain
            .iter()
            .map(|a| a.name.as_str())
            .chain([parent_id])
            .collect::<Vec<_>>()
            .join(" → ");
        let problem = if chain.iter().any(|a| a.name == parent_id) {
            format!("has an extends loop: {trail}.")
        } else if chain.len() > MAX_EXTENDS_DEPTH {
            format!("extends chain is deeper than {MAX_EXTENDS_DEPTH} levels: {trail}.")
        } else if let Some(parent) = catalog.iter().find(|a| a.name == parent_id) {
            chain.push(parent);
            continue;
        } else {
            format!("extends unknown agent profile {parent_id:?}.")
        };
        return (vec![child], Some(inheritance_error(&child.name, &problem)));
    }
    (chain, None)
}

/// What a profile inherits when it omits a field: the nearest chain member
/// that sets it wins. `skills` is the first NON-EMPTY filter — an empty
/// list means "not narrowed here", never "no skills".
struct Inherited {
    skills: Vec<String>,
    model: Option<String>,
    reasoning_effort: Option<String>,
}

fn inherit(chain: &[&FsAgent]) -> Inherited {
    Inherited {
        skills: chain
            .iter()
            .find(|a| !a.skills.is_empty())
            .map(|a| a.skills.clone())
            .unwrap_or_default(),
        model: chain.iter().find_map(|a| a.model.clone()),
        reasoning_effort: chain.iter().find_map(|a| a.reasoning_effort.clone()),
    }
}

fn bundled_raw(agent: &FsAgent) -> Result<&'static str, String> {
    crate::bundled::bundled_agent_raw(&agent.name).ok_or_else(|| {
        format!(
            "bundled agent profile {:?} has no embedded copy",
            agent.name
        )
    })
}

/// A profile's OWN body: the embedded copy for a bundled row, the file
/// otherwise.
fn read_agent_body(agent: &FsAgent) -> Result<String, String> {
    if agent.builtin {
        return bundled_raw(agent).map(|raw| fs_source::split_frontmatter(raw).1.to_string());
    }
    fs_source::read_body(&agent.abs_path)
}

fn read_agent_raw(agent: &FsAgent) -> Result<String, String> {
    if agent.builtin {
        return bundled_raw(agent).map(str::to_string);
    }
    fs_source::read_raw(&agent.abs_path)
}

/// The resolved system prompt, root first: each ancestor's body with its
/// trailing newlines trimmed, a blank line, then the next body — the
/// profile's own body last and verbatim, so a profile without `extends`
/// serves byte-identical to a plain read.
fn compose_prompt(chain: &[&FsAgent]) -> Result<String, String> {
    let mut prompt: Option<String> = None;
    for agent in chain.iter().rev() {
        let body = read_agent_body(agent)?;
        prompt = Some(match prompt {
            None => body,
            Some(parent) => format!("{}\n\n{body}", parent.trim_end_matches('\n')),
        });
    }
    Ok(prompt.unwrap_or_default())
}

pub fn list_agents(cfg: &SkillsConfig) -> ListAgentsOutput {
    let catalog = catalog(cfg);
    let agents = catalog
        .iter()
        .map(|a| {
            let (chain, inheritance_error) = resolve_chain(&catalog, a);
            let inherited = inherit(&chain);
            AgentEntry {
                modified_at: fs_modified_at(&a.abs_path),
                skill_count: (!inherited.skills.is_empty()).then_some(inherited.skills.len()),
                model: inherited.model,
                reasoning_effort: inherited.reasoning_effort,
                icon: a.icon.clone(),
                color: a.color.clone(),
                id: a.name.clone(),
                name: a.display_name.clone(),
                description: a.description.clone(),
                logo: a.logo.clone(),
                extends: a.extends.clone(),
                builtin: a.builtin,
                inheritance_error,
            }
        })
        .collect();
    ListAgentsOutput { agents }
}

/// `visible_skills` is the same set `directory::skills::list` serves
/// (`resolve_visible_skills`); the registered handler supplies it, tests
/// hand-build one.
pub fn get_agent(
    cfg: &SkillsConfig,
    req: AgentGetInput,
    visible_skills: &[FsSkill],
) -> Result<AgentGetOutput, String> {
    validate_name(&req.id)?;
    let catalog = catalog(cfg);
    let Some(agent) = catalog.iter().find(|a| a.name == req.id) else {
        return Err(agent_not_found(&catalog, &req.id));
    };
    let (chain, inheritance_error) = resolve_chain(&catalog, agent);
    let inherited = inherit(&chain);
    let system_prompt = compose_prompt(&chain)?;
    let raw = if req.raw.unwrap_or(false) {
        Some(read_agent_raw(agent)?)
    } else {
        None
    };
    // `find_fs_skill_in` resolves the `<ns>` ↔ `<ns>/index` overview
    // alias, so both id forms an author might write count as known.
    let unknown_skills = inherited
        .skills
        .iter()
        .filter(|id| find_fs_skill_in(visible_skills, id).is_none())
        .cloned()
        .collect();
    Ok(AgentGetOutput {
        modified_at: fs_modified_at(&agent.abs_path),
        id: agent.name.clone(),
        name: agent.display_name.clone(),
        description: agent.description.clone(),
        logo: agent.logo.clone(),
        system_prompt,
        skills: inherited.skills,
        unknown_skills,
        model: inherited.model,
        reasoning_effort: inherited.reasoning_effort,
        icon: agent.icon.clone(),
        color: agent.color.clone(),
        extends: agent.extends.clone(),
        builtin: agent.builtin,
        inheritance_error,
        raw,
    })
}

pub fn create_agent(
    cfg: &SkillsConfig,
    req: &AgentCreateInput,
) -> Result<AgentWriteOutput, String> {
    validate_name(&req.id)?;
    let catalog = catalog(cfg);
    // Only a FILE collides: creating a bundled id writes the local shadow,
    // exactly like creating a bundled system prompt.
    if let Some(existing) = catalog.iter().find(|a| a.name == req.id && !a.builtin) {
        // Name the root for global collisions: "already exists" alone sends
        // the caller hunting agents_folder for a file that is not there.
        let origin = if existing.abs_path.starts_with(cfg.resolved_agents_folder()) {
            String::new()
        } else {
            format!(
                " as a user-global profile ({})",
                existing.abs_path.display()
            )
        };
        return Err(invalid_input_message(
            "D414",
            &format!("agent profile {:?} already exists{origin}.", req.id),
            AGENT_CREATE_CONFLICT_NEXT,
        ));
    }
    let dest = cfg.resolved_agents_folder().join(format!("{}.md", req.id));
    if dest.exists() {
        return Err(skipped_file_conflict(&dest));
    }
    let agent = validate_agent_content(&req.id, &req.content, &dest)?;
    write_file_atomic(&dest, req.content.as_bytes())?;
    Ok(write_output(&agent, req.content.len()))
}

pub fn update_agent(
    cfg: &SkillsConfig,
    req: &AgentUpdateInput,
) -> Result<AgentWriteOutput, String> {
    validate_name(&req.id)?;
    let catalog = catalog(cfg);
    let Some(existing) = catalog.iter().find(|a| a.name == req.id) else {
        return Err(agent_not_found(&catalog, &req.id));
    };
    let dest = if existing.builtin {
        // A bundled profile has no file until first edited: updating it
        // copy-on-writes the local file, which shadows the bundled copy from
        // then on (deleting that file falls back to it again).
        let dest = cfg.resolved_agents_folder().join(format!("{}.md", req.id));
        if dest.exists() {
            return Err(skipped_file_conflict(&dest));
        }
        dest
    } else {
        existing.abs_path.clone()
    };
    let agent = validate_agent_content(&req.id, &req.content, &dest)?;
    write_file_atomic(&dest, req.content.as_bytes())?;
    Ok(write_output(&agent, req.content.len()))
}

pub fn delete_agent(
    cfg: &SkillsConfig,
    req: &AgentDeleteInput,
) -> Result<AgentDeleteOutput, String> {
    validate_name(&req.id)?;
    let catalog = catalog(cfg);
    let Some(agent) = catalog.iter().find(|a| a.name == req.id) else {
        return Err(agent_not_found(&catalog, &req.id));
    };
    if agent.builtin {
        return Err(invalid_input_message(
            "D414",
            &format!(
                "agent profile {:?} is bundled with the worker and has no local file to \
                 delete.",
                req.id
            ),
            AGENT_CREATE_CONFLICT_NEXT,
        ));
    }
    // Suppress the watcher's `{ op: "external" }` for our own delete —
    // the precise `{ op: "delete" }` fan-out already covers it.
    mark_self_write(&agent.abs_path);
    std::fs::remove_file(&agent.abs_path)
        .map_err(|e| format!("delete {}: {e}", agent.abs_path.display()))?;
    Ok(AgentDeleteOutput { id: req.id.clone() })
}

// ---------- validation ----------

/// Write-time content check, byte-identical to what the scanner
/// enforces: size cap on the raw file, required valid frontmatter
/// ([`fs_source::parse_agent_frontmatter`]), non-empty body. Returns the
/// row the next scan will serve for `dest` (the write receipt reads from
/// it).
fn validate_agent_content(
    id: &str,
    content: &str,
    dest: &std::path::Path,
) -> Result<FsAgent, String> {
    if content.len() > SKILL_BODY_MAX_BYTES {
        return Err(format!(
            "content too large ({} bytes; max {SKILL_BODY_MAX_BYTES})",
            content.len()
        ));
    }
    let fm = fs_source::parse_agent_frontmatter(content)?;
    let (_, body) = fs_source::split_frontmatter(content);
    if body.trim().is_empty() {
        return Err("body (the system prompt) must be non-empty".into());
    }
    Ok(fs_source::agent_from_frontmatter(
        id.to_string(),
        fm,
        dest.to_path_buf(),
        false,
    ))
}

fn skipped_file_conflict(dest: &std::path::Path) -> String {
    invalid_input_message(
        "D414",
        &format!(
            "a file already exists at {} (currently skipped by the scanner); edit or \
             remove it on disk.",
            dest.display()
        ),
        AGENT_CREATE_CONFLICT_NEXT,
    )
}

fn agent_not_found(agents: &[FsAgent], missed: &str) -> String {
    let names: Vec<String> = agents.iter().map(|a| a.name.clone()).collect();
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
    let candidates: Vec<String> = scored.into_iter().take(3).map(|(_, n)| n.clone()).collect();
    not_found_message(
        "D410",
        "agent profile",
        missed,
        &candidates,
        AGENT_NOT_FOUND_NEXT,
    )
}

fn write_output(agent: &FsAgent, bytes: usize) -> AgentWriteOutput {
    AgentWriteOutput {
        modified_at: fs_modified_at(&agent.abs_path),
        id: agent.name.clone(),
        name: agent.display_name.clone(),
        description: agent.description.clone(),
        logo: agent.logo.clone(),
        bytes,
    }
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
    use std::path::{Path, PathBuf};

    const CAPTAIN: &str = "---\nname: Release Captain\ndescription: Cuts releases.\nlogo: \"🚢\"\nskills:\n  - iii-sandbox\n  - agent-memory/observe\nmodel: codex/gpt-5.4-mini\nreasoning_effort: high\nicon: search\ncolor: purple\n---\nYou are the release captain.\n";

    fn write_fixture(dir: &Path, rel: &str, contents: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    fn cfg_for(dir: &Path) -> SkillsConfig {
        SkillsConfig {
            skills_folder: dir.join("skills").to_string_lossy().into_owned(),
            local_skills_folder: dir.join("local-empty").to_string_lossy().into_owned(),
            agents_folder: dir.join("agents").to_string_lossy().into_owned(),
            // Pinned inside the tempdir: the default resolves to the
            // developer's REAL ~/.iii/agents, which would leak
            // machine-dependent profiles into these tests.
            global_agents_folder: dir
                .join("global-agents-empty")
                .to_string_lossy()
                .into_owned(),
            ..SkillsConfig::default()
        }
    }

    fn skill(id: &str) -> FsSkill {
        FsSkill {
            id: id.into(),
            abs_path: PathBuf::from(format!("/x/{id}.md")),
        }
    }

    #[test]
    fn list_and_get_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "agents/release-captain.md", CAPTAIN);
        let cfg = cfg_for(tmp.path());

        let listed = list_agents(&cfg);
        // The bundled `iii` base is always listed alongside the files.
        let rows: Vec<&AgentEntry> = listed.agents.iter().filter(|r| !r.builtin).collect();
        assert_eq!(rows.len(), 1);
        let row = rows[0];
        assert_eq!(row.id, "release-captain");
        assert!(row.extends.is_none());
        assert!(row.inheritance_error.is_none());
        assert_eq!(row.name, "Release Captain");
        assert_eq!(row.logo.as_deref(), Some("🚢"));
        assert_eq!(row.skill_count, Some(2));
        assert_eq!(row.model.as_deref(), Some("codex/gpt-5.4-mini"));
        assert_eq!(row.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(row.icon.as_deref(), Some("search"));
        assert_eq!(row.color.as_deref(), Some("purple"));

        // `iii-sandbox` is known via the `<ns>/index` alias; the other id
        // matches nothing.
        let visible = vec![skill("iii-sandbox/index")];
        let got = get_agent(
            &cfg,
            AgentGetInput {
                id: "release-captain".into(),
                raw: Some(true),
            },
            &visible,
        )
        .unwrap();
        assert_eq!(got.system_prompt.trim(), "You are the release captain.");
        assert_eq!(got.unknown_skills, vec!["agent-memory/observe".to_string()]);
        assert_eq!(got.model.as_deref(), Some("codex/gpt-5.4-mini"));
        assert_eq!(got.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(got.icon.as_deref(), Some("search"));
        assert_eq!(got.color.as_deref(), Some("purple"));
        assert_eq!(got.raw.as_deref(), Some(CAPTAIN));
        assert!(got.extends.is_none());
        assert!(!got.builtin);
        assert!(got.inheritance_error.is_none());
    }

    #[test]
    fn get_reports_absent_lists() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "agents/frontend-design.md",
            "---\nname: Frontend\ndescription: UI work.\n---\nYou do frontend.\n",
        );
        let cfg = cfg_for(tmp.path());
        // Absent skills on a minimal agent.
        let plain = get_agent(
            &cfg,
            AgentGetInput {
                id: "frontend-design".into(),
                raw: None,
            },
            &[],
        )
        .unwrap();
        assert!(plain.skills.is_empty());
        assert!(plain.unknown_skills.is_empty());
    }

    #[test]
    fn get_miss_names_the_family_and_its_list() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "agents/real.md", CAPTAIN);
        let cfg = cfg_for(tmp.path());
        let err = get_agent(
            &cfg,
            AgentGetInput {
                id: "reel".into(),
                raw: None,
            },
            &[],
        )
        .unwrap_err();
        assert!(
            err.starts_with("D410 not_found: agent profile"),
            "got: {err}"
        );
        assert!(err.contains("real"), "candidate missing: {err}");
        assert!(err.contains("directory::agents::list"), "got: {err}");
    }

    #[test]
    fn list_does_not_fall_back_to_profiles_under_skills() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "skills/ns/agents/legacy.md", CAPTAIN);
        write_fixture(tmp.path(), "agents/current.md", CAPTAIN);
        let cfg = cfg_for(tmp.path());

        let ids = list_agents(&cfg)
            .agents
            .into_iter()
            .map(|agent| agent.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["current", "iii", "iii-minimal"]);
    }

    #[test]
    fn create_update_delete_lifecycle() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());

        let out = create_agent(
            &cfg,
            &AgentCreateInput {
                id: "captain".into(),
                content: CAPTAIN.into(),
            },
        )
        .unwrap();
        assert_eq!(out.name, "Release Captain");
        let dest = tmp.path().join("agents/captain.md");
        assert!(dest.is_file());

        // Duplicate create conflicts.
        let err = create_agent(
            &cfg,
            &AgentCreateInput {
                id: "captain".into(),
                content: CAPTAIN.into(),
            },
        )
        .unwrap_err();
        assert!(err.starts_with("D414 invalid_input:"), "got: {err}");
        assert!(err.contains("directory::agents::update"), "got: {err}");

        let updated = update_agent(
            &cfg,
            &AgentUpdateInput {
                id: "captain".into(),
                content: "---\nname: Captain v2\ndescription: New.\n---\nNew prompt.\n".into(),
            },
        )
        .unwrap();
        assert_eq!(updated.name, "Captain v2");
        assert!(std::fs::read_to_string(&dest)
            .unwrap()
            .contains("New prompt."));

        delete_agent(
            &cfg,
            &AgentDeleteInput {
                id: "captain".into(),
            },
        )
        .unwrap();
        assert!(!dest.exists());

        let err = update_agent(
            &cfg,
            &AgentUpdateInput {
                id: "captain".into(),
                content: CAPTAIN.into(),
            },
        )
        .unwrap_err();
        assert!(
            err.starts_with("D410 not_found: agent profile"),
            "got: {err}"
        );
    }

    #[test]
    fn write_rejects_what_the_scanner_would_skip() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());
        for (content, needle) in [
            ("no frontmatter\n", "missing YAML frontmatter"),
            ("---\ndescription: x\n---\nbody\n", "non-empty `name`"),
            ("---\nname: X\nlogo: ./x.png\n---\nbody\n", "emoji only"),
            (
                "---\nname: X\nicon: rocket\n---\nbody\n",
                "`icon` must be one of",
            ),
            (
                "---\nname: X\ncolor: ultraviolet\n---\nbody\n",
                "`color` must be one of",
            ),
            ("---\nname: X\ndescription: y\n---\n", "non-empty"),
        ] {
            let err = create_agent(
                &cfg,
                &AgentCreateInput {
                    id: "bad".into(),
                    content: content.into(),
                },
            )
            .unwrap_err();
            assert!(err.contains(needle), "content {content:?} → {err}");
        }
    }

    #[test]
    fn create_refuses_on_disk_but_skipped_file() {
        let tmp = tempfile::tempdir().unwrap();
        // On disk but frontmatter-less → invisible to the scan, still a
        // create conflict (create never clobbers).
        write_fixture(tmp.path(), "agents/ghost.md", "no frontmatter\n");
        let cfg = cfg_for(tmp.path());
        let err = create_agent(
            &cfg,
            &AgentCreateInput {
                id: "ghost".into(),
                content: CAPTAIN.into(),
            },
        )
        .unwrap_err();
        assert!(err.contains("already exists at"), "got: {err}");
    }

    /// A profile that only exists under the user-global root is listed and
    /// gettable; the same id under the project root shadows it.
    #[test]
    fn global_profiles_list_and_project_shadows() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());
        write_fixture(tmp.path(), "global-agents-empty/captain.md", CAPTAIN);
        write_fixture(
            tmp.path(),
            "global-agents-empty/scout.md",
            "---\nname: Scout\ndescription: Scouts.\n---\nYou scout.\n",
        );
        // Project copy of `captain` wins over the global one.
        write_fixture(
            tmp.path(),
            "agents/captain.md",
            "---\nname: Local Captain\ndescription: Local.\n---\nYou are local.\n",
        );

        let out = list_agents(&cfg);
        let names: Vec<(&str, &str)> = out
            .agents
            .iter()
            .map(|a| (a.id.as_str(), a.name.as_str()))
            .collect();
        assert_eq!(
            names,
            vec![
                ("captain", "Local Captain"),
                ("iii", "iii"),
                ("iii-minimal", "iii-minimal"),
                ("scout", "Scout")
            ]
        );

        let got = get_agent(
            &cfg,
            AgentGetInput {
                id: "scout".into(),
                raw: None,
            },
            &[],
        )
        .unwrap();
        assert_eq!(got.system_prompt.trim(), "You scout.");
    }

    /// Global profiles are EDITABLE in place (`~/.iii/agents` is iii's own
    /// directory): update rewrites the global file, create on the same id
    /// still reports the collision naming the global file (create only ever
    /// writes `agents_folder`), and delete removes the global file.
    #[test]
    fn global_profiles_are_editable_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());
        let global_file = tmp.path().join("global-agents-empty/captain.md");
        write_fixture(tmp.path(), "global-agents-empty/captain.md", CAPTAIN);

        let updated = CAPTAIN.replace("Release Captain", "Fleet Captain");
        let out = update_agent(
            &cfg,
            &AgentUpdateInput {
                id: "captain".into(),
                content: updated.clone(),
            },
        )
        .unwrap();
        assert_eq!(out.name, "Fleet Captain");
        assert_eq!(std::fs::read_to_string(&global_file).unwrap(), updated);
        // The project root stays untouched — the write landed in place.
        assert!(!tmp.path().join("agents/captain.md").exists());

        let err = create_agent(
            &cfg,
            &AgentCreateInput {
                id: "captain".into(),
                content: CAPTAIN.into(),
            },
        )
        .unwrap_err();
        assert!(err.starts_with("D414 invalid_input:"), "got: {err}");
        assert!(err.contains("user-global"), "got: {err}");

        delete_agent(
            &cfg,
            &AgentDeleteInput {
                id: "captain".into(),
            },
        )
        .unwrap();
        assert!(!global_file.exists());
    }

    const III_DOCTRINE_OPENER: &str = "You are an iii agent worker.";

    fn get(cfg: &SkillsConfig, id: &str, raw: bool) -> Result<AgentGetOutput, String> {
        get_agent(
            cfg,
            AgentGetInput {
                id: id.into(),
                raw: Some(raw),
            },
            &[],
        )
    }

    #[test]
    fn extends_composes_parent_first_and_inherits_omitted_fields() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "agents/base.md",
            "---\nname: Base\nskills:\n  - a\n  - b\nmodel: m1\nreasoning_effort: high\nicon: code\ncolor: blue\n---\nBase body.\n\n",
        );
        write_fixture(
            tmp.path(),
            "agents/child.md",
            "---\nname: Child\ndescription: Adds.\nextends: base\n---\nChild body.\n",
        );
        write_fixture(
            tmp.path(),
            "agents/narrow.md",
            "---\nname: Narrow\nextends: child\nskills:\n  - c\nmodel: m2\n---\nNarrow body.\n",
        );
        let cfg = cfg_for(tmp.path());

        let child = get(&cfg, "child", true).unwrap();
        // Parent trailing newlines trimmed, one blank line, own body verbatim.
        assert_eq!(child.system_prompt, "Base body.\n\nChild body.\n");
        assert_eq!(child.extends.as_deref(), Some("base"));
        assert_eq!(child.skills, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(child.model.as_deref(), Some("m1"));
        assert_eq!(child.reasoning_effort.as_deref(), Some("high"));
        // Display fields never inherit.
        assert_eq!(child.name, "Child");
        assert_eq!(child.description, "Adds.");
        assert!(child.icon.is_none() && child.color.is_none());
        assert!(child.inheritance_error.is_none());
        assert!(
            child.raw.unwrap().starts_with("---\nname: Child"),
            "raw is the profile's own file"
        );

        let narrow = get(&cfg, "narrow", false).unwrap();
        assert_eq!(
            narrow.system_prompt,
            "Base body.\n\nChild body.\n\nNarrow body.\n"
        );
        assert_eq!(
            narrow.skills,
            vec!["c".to_string()],
            "a non-empty filter replaces, no union"
        );
        assert_eq!(narrow.model.as_deref(), Some("m2"));
        assert_eq!(
            narrow.reasoning_effort.as_deref(),
            Some("high"),
            "effort inherits independently of model"
        );

        let rows = list_agents(&cfg).agents;
        let row = rows.iter().find(|r| r.id == "child").unwrap();
        assert_eq!(row.model.as_deref(), Some("m1"));
        assert_eq!(row.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(row.skill_count, Some(2));
        assert_eq!(row.extends.as_deref(), Some("base"));
        assert!(row.inheritance_error.is_none());
    }

    #[test]
    fn extends_bundled_iii_without_shadow() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "agents/lead.md",
            "---\nname: Lead\nextends: iii\n---\nYou lead.\n",
        );
        let cfg = cfg_for(tmp.path());
        let lead = get(&cfg, "lead", false).unwrap();
        assert!(lead.system_prompt.starts_with(III_DOCTRINE_OPENER));
        assert!(lead.system_prompt.ends_with("\n\nYou lead.\n"));
        assert!(!lead.builtin);
        assert!(lead.inheritance_error.is_none());
    }

    #[test]
    fn broken_chain_is_soft_on_read_and_writes_do_not_gate_it() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "agents/orphan.md",
            "---\nname: Orphan\nmodel: own\nextends: nope\n---\nOwn body.\n",
        );
        let cfg = cfg_for(tmp.path());

        let got = get(&cfg, "orphan", true).unwrap();
        let err = got
            .inheritance_error
            .clone()
            .expect("broken chain reported");
        assert!(
            err.starts_with(
                "D415 invalid_input: agent profile \"orphan\" extends unknown agent profile \"nope\"."
            ),
            "got: {err}"
        );
        assert!(err.contains("directory::agents::list"), "got: {err}");
        assert_eq!(got.system_prompt, "Own body.\n");
        assert_eq!(got.model.as_deref(), Some("own"));
        assert!(got.raw.is_some(), "the editor can still open it");
        let row = list_agents(&cfg)
            .agents
            .into_iter()
            .find(|r| r.id == "orphan")
            .unwrap();
        assert_eq!(row.inheritance_error.as_deref(), Some(err.as_str()));

        // Writes do not gate the chain: the file lands, the next read reports it.
        let content = std::fs::read_to_string(tmp.path().join("agents/orphan.md")).unwrap();
        create_agent(
            &cfg,
            &AgentCreateInput {
                id: "orphan2".into(),
                content,
            },
        )
        .unwrap();
        assert!(get(&cfg, "orphan2", false)
            .unwrap()
            .inheritance_error
            .is_some());
    }

    #[test]
    fn extends_loops_and_self_are_detected() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "agents/a.md",
            "---\nname: A\nextends: b\n---\nA.\n",
        );
        write_fixture(
            tmp.path(),
            "agents/b.md",
            "---\nname: B\nextends: a\n---\nB.\n",
        );
        let cfg = cfg_for(tmp.path());

        let a = get(&cfg, "a", false).unwrap();
        let err = a.inheritance_error.as_deref().unwrap();
        assert!(
            err.contains("has an extends loop: a → b → a."),
            "got: {err}"
        );
        assert_eq!(a.system_prompt, "A.\n");
        let b = get(&cfg, "b", false).unwrap();
        assert!(
            b.inheritance_error
                .as_deref()
                .unwrap()
                .contains("b → a → b"),
            "{:?}",
            b.inheritance_error
        );

        // Self-extends, including a local `iii` "extending" the bundled `iii`
        // it shadows (the shadowed copy is not in the catalog to extend).
        write_fixture(
            tmp.path(),
            "agents/c.md",
            "---\nname: C\nextends: c\n---\nC.\n",
        );
        write_fixture(
            tmp.path(),
            "agents/iii.md",
            "---\nname: iii\nextends: iii\n---\nMine.\n",
        );
        for (id, trail) in [("c", "c → c"), ("iii", "iii → iii")] {
            let got = get(&cfg, id, false).unwrap();
            let err = got.inheritance_error.as_deref().unwrap();
            assert!(
                err.contains(&format!("has an extends loop: {trail}.")),
                "got: {err}"
            );
        }
    }

    #[test]
    fn extends_depth_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());
        // p0 ← p1 ← … ← p9: p8 is eight hops from the root, p9 nine.
        write_fixture(tmp.path(), "agents/p0.md", "---\nname: P0\n---\nP0.\n");
        for i in 1..=9 {
            write_fixture(
                tmp.path(),
                &format!("agents/p{i}.md"),
                &format!("---\nname: P{i}\nextends: p{}\n---\nP{i}.\n", i - 1),
            );
        }
        let eight = get(&cfg, "p8", false).unwrap();
        assert!(
            eight.inheritance_error.is_none(),
            "{:?}",
            eight.inheritance_error
        );
        assert!(eight.system_prompt.starts_with("P0.\n\nP1.\n"));
        assert!(eight.system_prompt.ends_with("P7.\n\nP8.\n"));
        let nine = get(&cfg, "p9", false).unwrap();
        assert!(
            nine.inheritance_error
                .as_deref()
                .unwrap()
                .contains("deeper than 8 levels"),
            "{:?}",
            nine.inheritance_error
        );
        assert_eq!(nine.system_prompt, "P9.\n");
    }

    #[test]
    fn bundled_iii_lists_gets_copy_on_writes_and_falls_back() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_for(tmp.path());

        let rows = list_agents(&cfg).agents;
        let row = rows
            .iter()
            .find(|r| r.id == "iii")
            .expect("bundled base listed");
        assert!(row.builtin);
        assert_eq!(row.modified_at, "");
        assert!(row.inheritance_error.is_none());

        let got = get(&cfg, "iii", true).unwrap();
        assert!(got.builtin);
        assert!(got.system_prompt.starts_with(III_DOCTRINE_OPENER));
        assert_eq!(got.raw.as_deref(), crate::bundled::bundled_agent_raw("iii"));
        assert_eq!(got.modified_at, "");

        // Nothing on disk to delete yet.
        let err = delete_agent(&cfg, &AgentDeleteInput { id: "iii".into() }).unwrap_err();
        assert!(
            err.starts_with("D414 invalid_input:") && err.contains("bundled"),
            "got: {err}"
        );

        // Update copy-on-writes the local shadow …
        let local = tmp.path().join("agents/iii.md");
        let out = update_agent(
            &cfg,
            &AgentUpdateInput {
                id: "iii".into(),
                content: "---\nname: iii\ndescription: Mine.\n---\nMy own base.\n".into(),
            },
        )
        .unwrap();
        assert_eq!(out.description, "Mine.");
        assert!(local.is_file());
        let got = get(&cfg, "iii", false).unwrap();
        assert!(!got.builtin);
        assert_eq!(got.system_prompt, "My own base.\n");
        assert!(
            !list_agents(&cfg)
                .agents
                .iter()
                .find(|r| r.id == "iii")
                .unwrap()
                .builtin
        );

        // … and deleting the shadow falls back to the bundled copy.
        delete_agent(&cfg, &AgentDeleteInput { id: "iii".into() }).unwrap();
        assert!(!local.exists());
        assert!(get(&cfg, "iii", false).unwrap().builtin);
    }

    #[test]
    fn unknown_skills_checks_the_inherited_filter() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "agents/base.md",
            "---\nname: Base\nskills:\n  - known/index\n  - ghost\n---\nBase.\n",
        );
        write_fixture(
            tmp.path(),
            "agents/kid.md",
            "---\nname: Kid\nextends: base\n---\nKid.\n",
        );
        let cfg = cfg_for(tmp.path());
        let kid = get_agent(
            &cfg,
            AgentGetInput {
                id: "kid".into(),
                raw: None,
            },
            &[skill("known/index")],
        )
        .unwrap();
        assert_eq!(
            kid.skills,
            vec!["known/index".to_string(), "ghost".to_string()]
        );
        assert_eq!(kid.unknown_skills, vec!["ghost".to_string()]);
    }
}
