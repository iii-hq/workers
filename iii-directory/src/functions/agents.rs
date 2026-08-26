//! Filesystem-backed agent profiles (`directory::agents::*`).
//!
//! An agent is a reusable identity a session runs as: a system prompt
//! (the file body), a display name, a description, an emoji logo, and a
//! skill filter — one direct `<agents_folder>/<id>.md` file with required
//! YAML frontmatter (see `docs/architecture/agent-profile-storage.md`).
//! Five filesystem-backed verbs:
//!
//!   * `directory::agents::list`   — metadata-only listing.
//!   * `directory::agents::get`    — one agent's system prompt + metadata,
//!     plus `unknown_skills` / `unknown_delegates` (ids that resolve to
//!     nothing — warnings, never load failures).
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
    "browse agent ids",
)];

const AGENT_CREATE_CONFLICT_NEXT: &[NextAction] = &[
    NextAction::new("directory::agents::update", "edit the existing agent"),
    NextAction::new("directory::agents::list", "browse agent ids"),
];

// ---------- wire shapes ----------

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct ListAgentsInput {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AgentEntry {
    /// Flat agent id (the file stem) — what `harness::send { options:
    /// { agent } }` will take.
    pub id: String,
    /// Display name from frontmatter (`name:`).
    pub name: String,
    pub description: String,
    /// Emoji logo, `null` when the agent has none.
    pub logo: Option<String>,
    /// Length of the agent's skill filter; `null` = no filter (every
    /// skill).
    pub skill_count: Option<usize>,
    /// Default model id for sessions running as this agent; `null` =
    /// the send decides.
    pub model: Option<String>,
    /// Harness subagent icon token for spawn display identities;
    /// `null` = caller picks.
    pub icon: Option<String>,
    /// `true` = this agent may not delegate — a specialist meant to be
    /// spawned by an orchestrator, not to front a session.
    pub leaf: bool,
    /// File mtime as RFC 3339.
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
    /// The file body, frontmatter stripped — the identity, verbatim.
    pub system_prompt: String,
    /// The frontmatter skill filter. Empty = every skill.
    pub skills: Vec<String>,
    /// Filter entries that resolve to no currently visible skill.
    /// Warnings — the agent still loads and runs.
    pub unknown_skills: Vec<String>,
    /// Agent ids this agent may delegate to; `null` = every agent.
    pub delegates_to: Option<Vec<String>>,
    /// `true` = this agent may not delegate at all.
    pub leaf: bool,
    /// `delegates_to` entries that name no existing agent. Warnings.
    pub unknown_delegates: Vec<String>,
    /// Default model id for sessions running as this agent; `null` =
    /// the send decides. Served verbatim — resolution against the live
    /// model catalog happens where it is used.
    pub model: Option<String>,
    /// Harness subagent icon token (closed set, validated at write
    /// time); `null` = caller picks.
    pub icon: Option<String>,
    /// FULL on-disk file content. Present only when the request set
    /// `raw: true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    /// File mtime as RFC 3339.
    pub modified_at: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentCreateInput {
    /// New agent id — becomes the file stem
    /// (`<agents_folder>/<id>.md`). Lowercase ASCII, digits,
    /// `-` and `_` only. Must not collide with an existing agent or an
    /// on-disk file at the target path.
    pub id: String,
    /// FULL file content, frontmatter block included. Frontmatter must
    /// carry a non-empty `name`; the body is the system prompt and must
    /// be non-empty.
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentUpdateInput {
    /// Existing agent id, as returned by `directory::agents::list`.
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
    /// Existing agent id, as returned by `directory::agents::list`.
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
            "List filesystem-backed agent profiles (id, display name, description, emoji \
             logo, skill_count, modified_at) from the configured agents folder. An agent \
             is a reusable session identity: its file body is the \
             system prompt. skill_count null = the agent uses every skill.",
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
            "Fetch one agent profile by id. Returns the system prompt (the file body, \
             frontmatter stripped), display name, description, emoji logo, the skill \
             filter plus unknown_skills (filter entries matching no visible skill — \
             warnings, the agent still runs), delegates_to/leaf plus unknown_delegates, \
             and modified_at. Pass raw: true to also get the exact on-disk file for \
             editing with directory::agents::update.",
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
             non-empty). Rejects ids that already exist in the configured agents folder, \
             or a target path that already exists on disk. The write is atomic and \
             fans out directory::agents::on-change with { op: \"create\" }.",
        )
        .metadata(json!({"tool": {"label": "Create agent"}})),
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
             `name`, emoji-only `logo`, non-empty body), so an update can never produce \
             a file the next directory::agents::list would skip. The agent id stays the \
             file stem; frontmatter `name` is only the display name. The write is atomic \
             and fans out directory::agents::on-change with { op: \"update\" }.",
        )
        .metadata(json!({"tool": {"label": "Update agent"}})),
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
             same merged scan as directory::agents::list, removes only that agent's \
             markdown file, and fans out directory::agents::on-change with \
             { op: \"delete\" }. Sessions already running as this agent are not \
             affected; the id just stops resolving for new sends.",
        )
        .metadata(json!({"tool": {"label": "Delete agent"}})),
    );
}

// ---------- core helpers (engine-free, reusable in tests) ----------

fn scan_profiles(cfg: &SkillsConfig) -> Vec<FsAgent> {
    fs_source::scan_agents(&cfg.resolved_agents_folder()).0
}

pub fn list_agents(cfg: &SkillsConfig) -> ListAgentsOutput {
    let agents = scan_profiles(cfg)
        .into_iter()
        .map(|a| AgentEntry {
            modified_at: fs_modified_at(&a.abs_path),
            skill_count: (!a.skills.is_empty()).then_some(a.skills.len()),
            model: a.model,
            icon: a.icon,
            leaf: a.leaf,
            id: a.name,
            name: a.display_name,
            description: a.description,
            logo: a.logo,
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
    let agents = scan_profiles(cfg);
    let Some(agent) = agents.iter().find(|a| a.name == req.id).cloned() else {
        return Err(agent_not_found(&agents, &req.id));
    };
    let body = fs_source::read_body(&agent.abs_path)?;
    let raw = if req.raw.unwrap_or(false) {
        Some(fs_source::read_raw(&agent.abs_path)?)
    } else {
        None
    };
    // `find_fs_skill_in` resolves the `<ns>` ↔ `<ns>/index` overview
    // alias, so both id forms an author might write count as known.
    let unknown_skills = agent
        .skills
        .iter()
        .filter(|id| find_fs_skill_in(visible_skills, id).is_none())
        .cloned()
        .collect();
    let unknown_delegates = agent
        .delegates_to
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter(|id| !agents.iter().any(|a| &a.name == *id))
        .cloned()
        .collect();
    Ok(AgentGetOutput {
        modified_at: fs_modified_at(&agent.abs_path),
        id: agent.name,
        name: agent.display_name,
        description: agent.description,
        logo: agent.logo,
        system_prompt: body,
        skills: agent.skills,
        unknown_skills,
        delegates_to: agent.delegates_to,
        leaf: agent.leaf,
        unknown_delegates,
        model: agent.model,
        icon: agent.icon,
        raw,
    })
}

pub fn create_agent(
    cfg: &SkillsConfig,
    req: &AgentCreateInput,
) -> Result<AgentWriteOutput, String> {
    validate_name(&req.id)?;
    let agents = scan_profiles(cfg);
    if agents.iter().any(|a| a.name == req.id) {
        return Err(invalid_input_message(
            "D414",
            &format!("agent {:?} already exists.", req.id),
            AGENT_CREATE_CONFLICT_NEXT,
        ));
    }
    let dest = cfg.resolved_agents_folder().join(format!("{}.md", req.id));
    if dest.exists() {
        return Err(invalid_input_message(
            "D414",
            &format!(
                "a file already exists at {} (currently skipped by the scanner); edit or \
                 remove it on disk.",
                dest.display()
            ),
            AGENT_CREATE_CONFLICT_NEXT,
        ));
    }
    let fm = validate_agent_content(&req.content)?;
    write_file_atomic(&dest, req.content.as_bytes())?;
    Ok(write_output(req.id.clone(), fm, req.content.len(), &dest))
}

pub fn update_agent(
    cfg: &SkillsConfig,
    req: &AgentUpdateInput,
) -> Result<AgentWriteOutput, String> {
    validate_name(&req.id)?;
    let agents = scan_profiles(cfg);
    let Some(agent) = agents.iter().find(|a| a.name == req.id) else {
        return Err(agent_not_found(&agents, &req.id));
    };
    let fm = validate_agent_content(&req.content)?;
    write_file_atomic(&agent.abs_path, req.content.as_bytes())?;
    Ok(write_output(
        req.id.clone(),
        fm,
        req.content.len(),
        &agent.abs_path,
    ))
}

pub fn delete_agent(
    cfg: &SkillsConfig,
    req: &AgentDeleteInput,
) -> Result<AgentDeleteOutput, String> {
    validate_name(&req.id)?;
    let agents = scan_profiles(cfg);
    let Some(agent) = agents.iter().find(|a| a.name == req.id) else {
        return Err(agent_not_found(&agents, &req.id));
    };
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
/// ([`fs_source::parse_agent_frontmatter`]), non-empty body.
fn validate_agent_content(content: &str) -> Result<fs_source::AgentFrontmatter, String> {
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
    Ok(fm)
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
    not_found_message("D410", "agent", missed, &candidates, AGENT_NOT_FOUND_NEXT)
}

fn write_output(
    id: String,
    fm: fs_source::AgentFrontmatter,
    bytes: usize,
    path: &std::path::Path,
) -> AgentWriteOutput {
    AgentWriteOutput {
        modified_at: fs_modified_at(path),
        id,
        name: fm.name.as_deref().unwrap_or("").trim().to_string(),
        description: fm
            .description
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .to_string(),
        logo: fm.logo.map(|l| l.trim().to_string()),
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

    const CAPTAIN: &str = "---\nname: Release Captain\ndescription: Cuts releases.\nlogo: \"🚢\"\nskills:\n  - iii-sandbox\n  - agent-memory/observe\ndelegates_to: [frontend-design]\nmodel: codex/gpt-5.4-mini\nicon: search\n---\nYou are the release captain.\n";

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
        assert_eq!(listed.agents.len(), 1);
        let row = &listed.agents[0];
        assert_eq!(row.id, "release-captain");
        assert_eq!(row.name, "Release Captain");
        assert_eq!(row.logo.as_deref(), Some("🚢"));
        assert_eq!(row.skill_count, Some(2));
        assert_eq!(row.model.as_deref(), Some("codex/gpt-5.4-mini"));
        assert_eq!(row.icon.as_deref(), Some("search"));
        assert!(!row.leaf);

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
        assert_eq!(
            got.delegates_to.as_deref(),
            Some(&["frontend-design".to_string()][..])
        );
        assert!(!got.leaf);
        assert_eq!(got.unknown_delegates, vec!["frontend-design".to_string()]);
        assert_eq!(got.model.as_deref(), Some("codex/gpt-5.4-mini"));
        assert_eq!(got.icon.as_deref(), Some("search"));
        assert_eq!(got.raw.as_deref(), Some(CAPTAIN));
    }

    #[test]
    fn get_reports_known_delegate_and_absent_lists() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "agents/architect.md", CAPTAIN);
        write_fixture(
            tmp.path(),
            "agents/frontend-design.md",
            "---\nname: Frontend\ndescription: UI work.\n---\nYou do frontend.\n",
        );
        let cfg = cfg_for(tmp.path());
        let got = get_agent(
            &cfg,
            AgentGetInput {
                id: "architect".into(),
                raw: None,
            },
            &[],
        )
        .unwrap();
        assert!(
            got.unknown_delegates.is_empty(),
            "{:?}",
            got.unknown_delegates
        );

        // Absent skills / delegates_to on the second agent.
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
        assert!(plain.delegates_to.is_none());
        assert!(plain.unknown_delegates.is_empty());
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
        assert!(err.starts_with("D410 not_found: agent"), "got: {err}");
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
        assert_eq!(ids, vec!["current"]);
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
        assert!(err.starts_with("D410 not_found: agent"), "got: {err}");
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
}
