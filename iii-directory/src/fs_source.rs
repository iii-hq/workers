//! Filesystem-backed sources for skills, system prompts, and agent profiles.
//!
//! Everything is anchored at the configured `skills_folder` from
//! [`crate::config::SkillsConfig`]. Layout:
//!
//! ```text
//! <skills_folder>/
//!   <ns>/                       # one folder per download namespace
//!     index.md                  # → iii://<ns>/index
//!     SKILLS.md                 # → iii://<ns>/index   (alias of index.md)
//!     anything.md               # → iii://<ns>/anything
//!     deep/path.md              # → iii://<ns>/deep/path
//!     system-prompts/           # ← system prompts (YAML frontmatter required)
//!       reviewer.md
//!     agents/                   # ← reusable agent profiles
//!       release-captain.md
//! ```
//!
//! Files matched as skills become entries whose body is re-read from
//! disk on every resolve. The file system is the single source of
//! truth — nothing is ever cached or mirrored to iii-state.
//!
//! Public surface:
//!
//! - [`split_frontmatter`]            — minimal `---\n...\n---\n` parser.
//! - [`scan_skills`]                  — id-keyed listing of skill markdown files.
//! - [`scan_agents_skills`]           — shallow `<skill>/SKILL.md` listing of the read-only
//!   agents root (the `~/.agents/skills` convention).
//! - [`scan_system_prompts`]          — name-keyed listing of `*/system-prompts/*.md`.
//! - [`read_body`]                    — cap-checked body read with frontmatter stripped.
//! - [`read_skill_with_frontmatter`]  — same caps as `read_body` plus a
//!   parsed `SkillFrontmatter` (title + type) so the skills reader can
//!   prefer the frontmatter title over a body H1 in one read.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::functions::prompts::validate_name;
use crate::functions::skills::{validate_id, SKILL_BODY_MAX_BYTES};

/// One filesystem-backed skill entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsSkill {
    pub id: String,
    pub abs_path: PathBuf,
}

/// One filesystem-backed system prompt. `description` is parsed from
/// frontmatter at scan time so list callers do not re-read every file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsPrompt {
    pub name: String,
    pub description: String,
    pub abs_path: PathBuf,
}

/// Diagnostic record for one file that failed to load. Surfaced via
/// boot-time logging so misconfigured layouts are easy to spot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkipReason {
    pub kind: SourceKind,
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    Skill,
    SystemPrompt,
    Agent,
}

#[derive(Debug, Default, Deserialize)]
pub struct PromptFrontmatter {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Parse the REQUIRED prompt frontmatter block out of raw file content.
/// Shared by [`scan_system_prompts`] (scan-time) and system-prompt updates
/// (write-time) so the two validations can't drift: a write that this
/// function rejects is exactly a file the next scan would skip.
pub fn parse_prompt_frontmatter(content: &str) -> Result<PromptFrontmatter, String> {
    let (fm_text, _) = split_frontmatter(content);
    let Some(fm_text) = fm_text else {
        return Err("missing YAML frontmatter (expected --- ... --- block at file start)".into());
    };
    serde_yaml::from_str(fm_text).map_err(|e| format!("invalid frontmatter YAML: {e}"))
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct SkillFrontmatter {
    /// Optional human-readable title. When non-empty (after trim) the
    /// reader returns this verbatim instead of the first body `# H1`.
    #[serde(default)]
    pub title: Option<String>,
    /// Optional skill name — the key the `~/.agents/skills` convention
    /// (and most repo-bundled SKILL.md files) use instead of `title`.
    /// Used as a title fallback (title → name → body H1).
    #[serde(default)]
    pub name: Option<String>,
    /// Free-form classifier (e.g. `index`, `how-to`, `reference`).
    /// Renamed from the YAML key `type` to avoid the Rust reserved word.
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    /// Canonical bus function id this skill documents (e.g.
    /// `sandbox::create`). When present, surfaced verbatim by
    /// `directory::skills::list` / `::get` so a calling agent can tell
    /// the SKILL id (`sandbox/skills/sandbox/create`, the on-disk path)
    /// apart from the FUNCTION id (`sandbox::create`, what
    /// `agent_trigger` actually expects). Missing for index-type and
    /// reference-type skills that aren't 1:1 with a single function.
    #[serde(default)]
    pub function_id: Option<String>,
    /// Optional short description. When present and non-empty, preferred
    /// over the body first-paragraph as the teaser text in `list` rows.
    #[serde(default)]
    pub description: Option<String>,
    /// Whether model-facing indexes should omit this skill from invocation candidates.
    #[serde(default, rename = "disable-model-invocation")]
    pub disable_model_invocation: bool,
}

// ───────────────────────── pure helpers ──────────────────────────────

/// Recognize a YAML frontmatter block at the start of `content`. The
/// fence is `---` on its own line. Returns `(Some(yaml), body)` on
/// success and `(None, content)` if no frontmatter is present (or the
/// opening fence has no matching closer). The body is whatever follows
/// the closing fence's newline, byte-for-byte (no trim).
pub fn split_frontmatter(content: &str) -> (Option<&str>, &str) {
    let after_open = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"));
    let Some(rest) = after_open else {
        return (None, content);
    };
    if let Some(idx) = rest.find("\n---\n") {
        return (Some(&rest[..idx]), &rest[idx + "\n---\n".len()..]);
    }
    if let Some(idx) = rest.find("\n---\r\n") {
        return (Some(&rest[..idx]), &rest[idx + "\n---\r\n".len()..]);
    }
    if let Some(stripped) = rest.strip_suffix("\n---") {
        return (Some(stripped), "");
    }
    if let Some(stripped) = rest.strip_suffix("\n---\r") {
        return (Some(stripped), "");
    }
    (None, content)
}

pub const SYSTEM_PROMPTS_SEGMENT: &str = "system-prompts";
pub const PROMPTS_SEGMENT: &str = "prompts";

/// Path component that marks the agents family in the walk (also the
/// top-level folder `directory::agents::create` writes into). Unrelated
/// to the `agents_skills_folder` config root (`~/.agents/skills`), which
/// is an external-tool *skills* convention.
pub const AGENTS_SEGMENT: &str = "agents";

/// Classify one path (relative to a scan root) by its segments — the
/// single discriminator the scanner, the download classifier, and the
/// fs watcher all share. NOT an exhaustive match: a kind missing an arm
/// here silently falls through to `Skill`, so every new family must add
/// one. Precedence when a path carries several marker segments:
/// `system-prompts` > `prompts` > `agents`. Command-prompt paths are
/// intentionally ignored, including when they also contain `agents`.
pub fn classify_rel_path(rel: &Path) -> Option<SourceKind> {
    let has = |seg: &str| rel.components().any(|c| c.as_os_str() == seg);
    if has(SYSTEM_PROMPTS_SEGMENT) {
        Some(SourceKind::SystemPrompt)
    } else if has(PROMPTS_SEGMENT) {
        None
    } else if has(AGENTS_SEGMENT) {
        Some(SourceKind::Agent)
    } else {
        Some(SourceKind::Skill)
    }
}

/// Walk `base_dir` with the `**/*.md` glob and return every `.md` file
/// it finds. The function does not validate ids or read bodies — it's
/// just a directory traversal helper.
fn walk_markdown(base_dir: &Path) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    if !base_dir.exists() {
        return Ok(Vec::new());
    }
    let pattern = base_dir
        .join("**/*.md")
        .to_str()
        .ok_or_else(|| format!("non-UTF-8 path: {}", base_dir.display()))?
        .to_string();
    let entries = glob::glob(&pattern).map_err(|e| format!("invalid glob {pattern:?}: {e}"))?;
    let mut out = Vec::new();
    for entry in entries {
        let path = match entry {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "glob entry error; skipping");
                continue;
            }
        };
        if !path.is_file() {
            continue;
        }
        let rel = match path.strip_prefix(base_dir) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };
        out.push((path, rel));
    }
    Ok(out)
}

/// Convert a `<skills_folder>`-relative path to a skill id.
///
/// Both `SKILLS.md` and `SKILL.md` (case-sensitive exact match on the
/// final path component) are treated as aliases for `index.md`, so
/// `<ns>/SKILLS.md` and `<ns>/SKILL.md` both produce the id
/// `<ns>/index`. The alias runs on the final path component only —
/// directories named `SKILLS` or `SKILL` are *not* renamed.
fn rel_to_id(rel: &Path) -> Result<String, String> {
    let rel_str = rel
        .to_str()
        .ok_or_else(|| format!("non-UTF-8 path: {}", rel.display()))?;
    let aliased = if let Some(parent) = rel.parent() {
        let is_index_alias = rel
            .file_name()
            .and_then(|s| s.to_str())
            .map(|n| n == "SKILLS.md" || n == "SKILL.md")
            .unwrap_or(false);
        if is_index_alias {
            let parent_str = parent.to_str().unwrap_or("");
            if parent_str.is_empty() {
                "index.md".to_string()
            } else {
                format!("{}/index.md", parent_str.replace('\\', "/"))
            }
        } else {
            rel_str.to_string()
        }
    } else {
        rel_str.to_string()
    };
    let stripped = aliased.strip_suffix(".md").unwrap_or(&aliased).to_string();
    Ok(stripped.replace('\\', "/"))
}

/// Scan every `*.md` under `skills_folder` (recursive) excluding files
/// with a `prompts/` segment in their relative path. Returns the
/// validated, deduped, lex-sorted list plus a parallel diagnostic list
/// of files that were rejected.
///
/// Rejection reasons:
///
/// - The derived id fails [`validate_id`] (uppercase, spaces, etc.).
/// - Two distinct files produce the same id (intra-fs duplicate).
pub fn scan_skills(skills_folder: &Path) -> (Vec<FsSkill>, Vec<SkipReason>) {
    let mut skills: Vec<FsSkill> = Vec::new();
    let mut skipped: Vec<SkipReason> = Vec::new();

    let entries = match walk_markdown(skills_folder) {
        Ok(v) => v,
        Err(e) => {
            skipped.push(SkipReason {
                kind: SourceKind::Skill,
                path: skills_folder.to_path_buf(),
                reason: e,
            });
            return (skills, skipped);
        }
    };

    for (abs, rel) in entries {
        if classify_rel_path(&rel) != Some(SourceKind::Skill) {
            continue;
        }
        let id = match rel_to_id(&rel) {
            Ok(s) => s,
            Err(e) => {
                skipped.push(SkipReason {
                    kind: SourceKind::Skill,
                    path: abs,
                    reason: e,
                });
                continue;
            }
        };
        if let Err(e) = validate_id(&id) {
            skipped.push(SkipReason {
                kind: SourceKind::Skill,
                path: abs,
                reason: format!("invalid id {id:?}: {e}"),
            });
            continue;
        }
        if let Some(existing) = skills.iter().find(|s| s.id == id) {
            if existing.abs_path != abs {
                skipped.push(SkipReason {
                    kind: SourceKind::Skill,
                    path: abs,
                    reason: format!(
                        "duplicate id {id:?} also produced by {}",
                        existing.abs_path.display()
                    ),
                });
            }
            continue;
        }
        skills.push(FsSkill { id, abs_path: abs });
    }

    skills.sort_by(|a, b| a.id.cmp(&b.id));
    (skills, skipped)
}

/// Scan system-prompt files (`system-prompts` path segment) under
/// `skills_folder`. Each match must have YAML frontmatter declaring at
/// least `description`; `name` is optional and overrides the
/// file-basename-derived default.
///
/// Rejection reasons mirror [`scan_skills`]: missing frontmatter,
/// invalid YAML, missing `description`, invalid prompt name, or a name
/// collision with another prompt.
pub fn scan_system_prompts(skills_folder: &Path) -> (Vec<FsPrompt>, Vec<SkipReason>) {
    let mut prompts: Vec<FsPrompt> = Vec::new();
    let mut skipped: Vec<SkipReason> = Vec::new();

    let entries = match walk_markdown(skills_folder) {
        Ok(v) => v,
        Err(e) => {
            skipped.push(SkipReason {
                kind: SourceKind::SystemPrompt,
                path: skills_folder.to_path_buf(),
                reason: e,
            });
            return (prompts, skipped);
        }
    };

    for (abs, rel) in entries {
        if classify_rel_path(&rel) != Some(SourceKind::SystemPrompt) {
            continue;
        }
        let content = match std::fs::read_to_string(&abs) {
            Ok(c) => c,
            Err(e) => {
                skipped.push(SkipReason {
                    kind: SourceKind::SystemPrompt,
                    path: abs,
                    reason: format!("read: {e}"),
                });
                continue;
            }
        };
        let fm = match parse_prompt_frontmatter(&content) {
            Ok(f) => f,
            Err(reason) => {
                skipped.push(SkipReason {
                    kind: SourceKind::SystemPrompt,
                    path: abs,
                    reason,
                });
                continue;
            }
        };

        // Prompt names are flat — fall back to the file stem when
        // frontmatter doesn't declare one.
        let derived = abs
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let name = fm
            .name
            .as_deref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or(derived);

        if let Err(e) = validate_name(&name) {
            skipped.push(SkipReason {
                kind: SourceKind::SystemPrompt,
                path: abs,
                reason: format!("invalid prompt name {name:?}: {e}"),
            });
            continue;
        }

        let description = match fm.description {
            Some(d) if !d.trim().is_empty() => d.trim().to_string(),
            _ => {
                skipped.push(SkipReason {
                    kind: SourceKind::SystemPrompt,
                    path: abs,
                    reason: "frontmatter missing non-empty `description`".into(),
                });
                continue;
            }
        };

        if let Some(existing) = prompts.iter().find(|p| p.name == name) {
            if existing.abs_path != abs {
                skipped.push(SkipReason {
                    kind: SourceKind::SystemPrompt,
                    path: abs,
                    reason: format!(
                        "duplicate name {name:?} also produced by {}",
                        existing.abs_path.display()
                    ),
                });
            }
            continue;
        }

        prompts.push(FsPrompt {
            name,
            description,
            abs_path: abs,
        });
    }

    prompts.sort_by(|a, b| a.name.cmp(&b.name));
    (prompts, skipped)
}

// ───────────────────────── agents family ─────────────────────────────

/// One filesystem-backed agent profile entry. Everything a `list` row
/// or a delegation catalog needs is parsed at scan time; only the body
/// (the system prompt) is re-read on `get`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsAgent {
    /// Flat id: the file stem, validated like a prompt name.
    pub name: String,
    /// Frontmatter `name` — the display name ("Release Captain").
    pub display_name: String,
    pub description: String,
    /// Emoji logo, verbatim from frontmatter. v1 is emoji-only.
    pub logo: Option<String>,
    /// Skill-id filter. Empty = every skill.
    pub skills: Vec<String>,
    /// Delegation catalog filter. `None` = every agent.
    pub delegates_to: Option<Vec<String>>,
    /// `true` = this agent may not delegate at all.
    pub leaf: bool,
    /// Default model id for sessions running as this agent (a router
    /// model id, e.g. `codex/gpt-5.4-mini`). `None` = the send decides.
    /// Stored verbatim — whether the id resolves is checked where it is
    /// used (the harness / the UI's model catalog), not at scan time,
    /// so an agent never fails to load over a retired model.
    pub model: Option<String>,
    /// Harness subagent icon token (one of [`AGENT_ICON_TOKENS`]) for
    /// spawn display identities. `None` = caller picks.
    pub icon: Option<String>,
    pub abs_path: PathBuf,
}

#[derive(Debug, Default, Deserialize)]
pub struct AgentFrontmatter {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub delegates_to: Option<Vec<String>>,
    #[serde(default)]
    pub leaf: bool,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

/// The harness `SubagentIcon` closed token set — `harness::spawn`
/// rejects anything else, so writes validate against this list rather
/// than letting a typo fail every future spawn.
pub const AGENT_ICON_TOKENS: &[&str] = &[
    "agent", "code", "search", "terminal", "database", "test", "review", "docs", "design",
];

/// Max byte length for an emoji logo (a couple of emoji with modifiers).
pub const AGENT_LOGO_MAX_BYTES: usize = 16;

/// Parse and validate the REQUIRED agent frontmatter block. Shared by
/// [`scan_agents`] (scan-time) and `directory::agents::create` /
/// `::update` (write-time) so the two can't drift: a write this rejects
/// is exactly a file the next scan would skip.
///
/// Hard rules: frontmatter present and valid YAML, non-empty `name`,
/// valid emoji `logo` when present. `description` missing is an empty
/// string; `skills` / `delegates_to` entries are NOT shape-checked here —
/// an id that matches nothing surfaces as `unknown_*` on `get`, a
/// warning rather than a load failure.
pub fn parse_agent_frontmatter(content: &str) -> Result<AgentFrontmatter, String> {
    let (fm_text, _) = split_frontmatter(content);
    let Some(fm_text) = fm_text else {
        return Err("missing YAML frontmatter (expected --- ... --- block at file start)".into());
    };
    let fm: AgentFrontmatter =
        serde_yaml::from_str(fm_text).map_err(|e| format!("invalid frontmatter YAML: {e}"))?;
    if fm.name.as_deref().map(str::trim).unwrap_or("").is_empty() {
        return Err("frontmatter missing non-empty `name`".into());
    }
    if let Some(logo) = fm.logo.as_deref() {
        validate_agent_logo(logo)?;
    }
    if let Some(icon) = fm.icon.as_deref().map(str::trim).filter(|i| !i.is_empty()) {
        if !AGENT_ICON_TOKENS.contains(&icon) {
            return Err(format!(
                "`icon` must be one of {} (got {icon:?})",
                AGENT_ICON_TOKENS.join(", ")
            ));
        }
    }
    Ok(fm)
}

/// v1 logos are emoji only: short, no path characters, no whitespace.
pub fn validate_agent_logo(logo: &str) -> Result<(), String> {
    let logo = logo.trim();
    if logo.is_empty() {
        return Err("`logo` must be non-empty when present (emoji only)".into());
    }
    if logo.len() > AGENT_LOGO_MAX_BYTES {
        return Err(format!(
            "`logo` too long ({} bytes; max {AGENT_LOGO_MAX_BYTES}) — emoji only",
            logo.len()
        ));
    }
    if logo
        .chars()
        .any(|c| c == '/' || c == '\\' || c.is_whitespace())
    {
        return Err("`logo` may not contain path separators or whitespace — emoji only".into());
    }
    Ok(())
}

/// Scan agent profiles (`agents/` path segment) under `root`. Mirrors
/// [`scan_system_prompts`]: required frontmatter, stem-derived flat id
/// validated by [`validate_name`], first-wins dedupe with a
/// [`SkipReason`] for the loser.
pub fn scan_agents(root: &Path) -> (Vec<FsAgent>, Vec<SkipReason>) {
    let mut agents: Vec<FsAgent> = Vec::new();
    let mut skipped: Vec<SkipReason> = Vec::new();

    let entries = match walk_markdown(root) {
        Ok(v) => v,
        Err(e) => {
            skipped.push(SkipReason {
                kind: SourceKind::Agent,
                path: root.to_path_buf(),
                reason: e,
            });
            return (agents, skipped);
        }
    };

    for (abs, rel) in entries {
        if classify_rel_path(&rel) != Some(SourceKind::Agent) {
            continue;
        }
        let mut skip = |reason: String| {
            skipped.push(SkipReason {
                kind: SourceKind::Agent,
                path: abs.clone(),
                reason,
            });
        };
        let content = match std::fs::read_to_string(&abs) {
            Ok(c) => c,
            Err(e) => {
                skip(format!("read: {e}"));
                continue;
            }
        };
        let fm = match parse_agent_frontmatter(&content) {
            Ok(f) => f,
            Err(reason) => {
                skip(reason);
                continue;
            }
        };
        let name = abs
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if let Err(e) = validate_name(&name) {
            skip(format!("invalid agent id {name:?}: {e}"));
            continue;
        }
        if let Some(existing) = agents.iter().find(|a| a.name == name) {
            if existing.abs_path != abs {
                let reason = format!(
                    "duplicate id {name:?} also produced by {}",
                    existing.abs_path.display()
                );
                skip(reason);
            }
            continue;
        }
        agents.push(FsAgent {
            name,
            display_name: fm.name.as_deref().unwrap_or("").trim().to_string(),
            description: fm
                .description
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .to_string(),
            logo: fm.logo.map(|l| l.trim().to_string()),
            skills: fm.skills,
            delegates_to: fm.delegates_to,
            leaf: fm.leaf,
            model: fm
                .model
                .as_deref()
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .map(str::to_string),
            icon: fm
                .icon
                .as_deref()
                .map(str::trim)
                .filter(|i| !i.is_empty())
                .map(str::to_string),
            abs_path: abs,
        });
    }

    agents.sort_by(|a, b| a.name.cmp(&b.name));
    (agents, skipped)
}

/// Merged scan of agents from a global root and a local root — same
/// whole-namespace override semantics as [`scan_system_prompts_merged`].
pub fn scan_agents_merged(
    global_root: &Path,
    local_root: &Path,
) -> (Vec<FsAgent>, Vec<SkipReason>) {
    let local_ns = top_level_namespaces(local_root);

    let (global_agents, mut global_skipped) = scan_agents(global_root);
    let global_filtered: Vec<FsAgent> = global_agents
        .into_iter()
        .filter(|a| {
            let top_seg = a
                .abs_path
                .strip_prefix(global_root)
                .ok()
                .and_then(|r| r.components().next())
                .and_then(|c| c.as_os_str().to_str())
                .unwrap_or("");
            !local_ns.contains(&top_seg.to_string())
        })
        .collect();

    global_skipped.retain(|s| {
        let rel = s
            .path
            .strip_prefix(global_root)
            .ok()
            .and_then(|p| p.components().next())
            .and_then(|c| c.as_os_str().to_str())
            .unwrap_or("");
        !local_ns.contains(&rel.to_string())
    });

    let (local_agents, local_skipped) = scan_agents(local_root);

    let mut merged = local_agents;
    merged.extend(global_filtered);
    merged.sort_by(|a, b| a.name.cmp(&b.name));

    let mut all_skipped = global_skipped;
    all_skipped.extend(local_skipped);

    (merged, all_skipped)
}

/// Read a fs entry's body fresh from disk, strip any leading
/// frontmatter, and enforce the same 256 KiB cap as the registry
/// previously did. The cap is checked against the raw file size so a
/// file with large frontmatter can't pass one path and fail the other.
/// Empty-after-strip bodies are an error so the resolver returns a
/// clear "not found" rather than serving an empty resource.
pub fn read_body(abs_path: &Path) -> Result<String, String> {
    let (_, body) = read_skill_with_frontmatter(abs_path)?;
    Ok(body)
}

/// Read a file's FULL raw content (frontmatter block included) with the
/// same size cap as [`read_skill_with_frontmatter`]. Serves the `raw: true`
/// read path so editors can round-trip the exact on-disk file; unlike the
/// body readers it accepts an empty-after-frontmatter body (the raw form
/// is for editing, not serving).
pub fn read_raw(abs_path: &Path) -> Result<String, String> {
    let raw = std::fs::read_to_string(abs_path)
        .map_err(|e| format!("read {}: {e}", abs_path.display()))?;
    if raw.len() > SKILL_BODY_MAX_BYTES {
        return Err(format!(
            "file {} is too large ({} bytes; max {SKILL_BODY_MAX_BYTES})",
            abs_path.display(),
            raw.len()
        ));
    }
    Ok(raw)
}

/// Like [`read_body`] but also returns the parsed [`SkillFrontmatter`].
/// Files without a frontmatter block (or with malformed YAML) still
/// succeed and yield `SkillFrontmatter::default()` so the read path
/// keeps working for plain markdown — only the size cap and the
/// empty-body rule are hard errors. Callers use the returned title /
/// type to fill in the corresponding `directory::skills::*` response
/// fields without re-reading the file.
pub fn read_skill_with_frontmatter(abs_path: &Path) -> Result<(SkillFrontmatter, String), String> {
    let raw = std::fs::read_to_string(abs_path)
        .map_err(|e| format!("read {}: {e}", abs_path.display()))?;
    if raw.len() > SKILL_BODY_MAX_BYTES {
        return Err(format!(
            "file {} is too large ({} bytes; max {SKILL_BODY_MAX_BYTES})",
            abs_path.display(),
            raw.len()
        ));
    }
    let (fm_text, body) = split_frontmatter(&raw);
    let trimmed = body.trim_matches('\n');
    if trimmed.is_empty() {
        return Err(format!("file {} has empty body", abs_path.display()));
    }
    let fm = fm_text
        .and_then(|t| serde_yaml::from_str::<SkillFrontmatter>(t).ok())
        .unwrap_or_default();
    Ok((fm, body.to_string()))
}

/// Top-level namespace directories under `root`. Returns a sorted,
/// deduped list of directory names.
pub(crate) fn top_level_namespaces(root: &Path) -> Vec<String> {
    let mut ns = Vec::new();
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return ns,
    };
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                ns.push(name.to_string());
            }
        }
    }
    ns.sort();
    ns.dedup();
    ns
}

/// Merged scan of skills from a global root and a local root.
///
/// **Whole-namespace local override**: for any top-level namespace
/// directory present under `local_root` (mere existence of the
/// directory is enough), that namespace's skills come ONLY from
/// `local_root`; all other namespaces come from `global_root`.
/// Downloads always write the global root.
///
/// ```text
///   global_root/
///     worker-a/   ← global (no local override)
///     worker-b/   ← shadowed by local
///   local_root/
///     worker-b/   ← takes over entirely
///     worker-c/   ← local-only namespace
/// ```
pub fn scan_skills_merged(
    global_root: &Path,
    local_root: &Path,
) -> (Vec<FsSkill>, Vec<SkipReason>) {
    let local_ns = top_level_namespaces(local_root);

    // Scan global, filtering out namespaces that are shadowed locally.
    let (global_skills, mut global_skipped) = scan_skills(global_root);
    let global_filtered: Vec<FsSkill> = global_skills
        .into_iter()
        .filter(|s| {
            let top_seg = s.id.split('/').next().unwrap_or("");
            !local_ns.contains(&top_seg.to_string())
        })
        .collect();

    // Also filter global skipped diagnostics for shadowed namespaces.
    global_skipped.retain(|s| {
        let rel = s
            .path
            .strip_prefix(global_root)
            .ok()
            .and_then(|p| p.components().next())
            .and_then(|c| c.as_os_str().to_str())
            .unwrap_or("");
        !local_ns.contains(&rel.to_string())
    });

    // Scan local.
    let (local_skills, local_skipped) = scan_skills(local_root);

    // Merge: local skills first (they won any shadowed namespace),
    // then global-only namespaces. Re-sort by id for deterministic order.
    let mut merged = local_skills;
    merged.extend(global_filtered);
    merged.sort_by(|a, b| a.id.cmp(&b.id));

    let mut all_skipped = global_skipped;
    all_skipped.extend(local_skipped);

    (merged, all_skipped)
}

/// Shallow scan of a read-only agents skills root (the `~/.agents/skills`
/// convention: one directory per installed skill, each containing a
/// `SKILL.md` plus arbitrary support payload such as `reference/` and
/// `scripts/`).
///
/// Deliberately NOT a `**/*.md` glob: only `<dir>/SKILL.md` is picked up
/// (id `<dir>/index`, per the [`rel_to_id`] alias), so a skill's support
/// docs never flood `directory::skills::list`. A missing or unreadable
/// root is silently empty — this worker never creates or writes under it.
pub fn scan_agents_skills(agents_root: &Path) -> (Vec<FsSkill>, Vec<SkipReason>) {
    let mut skills: Vec<FsSkill> = Vec::new();
    let mut skipped: Vec<SkipReason> = Vec::new();

    let entries = match std::fs::read_dir(agents_root) {
        Ok(e) => e,
        Err(_) => return (skills, skipped),
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let abs = dir.join("SKILL.md");
        if !abs.is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            skipped.push(SkipReason {
                kind: SourceKind::Skill,
                path: abs,
                reason: "non-UTF-8 directory name".into(),
            });
            continue;
        };
        let id = format!("{name}/index");
        if let Err(e) = validate_id(&id) {
            skipped.push(SkipReason {
                kind: SourceKind::Skill,
                path: abs,
                reason: format!("invalid id {id:?}: {e}"),
            });
            continue;
        }
        skills.push(FsSkill { id, abs_path: abs });
    }

    skills.sort_by(|a, b| a.id.cmp(&b.id));
    (skills, skipped)
}

/// Namespaces the agents root ACTUALLY serves: the top segment of every
/// [`scan_agents_skills`] entry, i.e. only directories carrying a valid
/// `SKILL.md`. Deliberately narrower than [`top_level_namespaces`] — a
/// stray skill-less directory under `~/.agents/skills` must neither
/// exempt a namespace from `filter_unregistered` nor reserve it against
/// `directory::skills::create`.
pub fn agents_namespaces(agents_root: &Path) -> Vec<String> {
    let (skills, _skipped) = scan_agents_skills(agents_root);
    let mut ns: Vec<String> = skills
        .into_iter()
        .filter_map(|s| s.id.split('/').next().map(str::to_string))
        .collect();
    ns.dedup(); // scan output is id-sorted, so dups are adjacent
    ns
}

/// Append non-shadowed agents-root skills to an already-resolved visible
/// set. Same whole-namespace override semantics as [`scan_skills_merged`],
/// one tier lower: a top-level namespace directory present under either
/// `local_root` or `global_root` shadows that namespace in the agents
/// root entirely (local > global > agents).
pub fn merge_agents_root(
    visible: Vec<FsSkill>,
    global_root: &Path,
    local_root: &Path,
    agents_root: &Path,
) -> (Vec<FsSkill>, Vec<SkipReason>) {
    let mut shadow_ns = top_level_namespaces(local_root);
    shadow_ns.extend(top_level_namespaces(global_root));
    shadow_ns.sort();
    shadow_ns.dedup();

    let (agents_skills, mut skipped) = scan_agents_skills(agents_root);
    let mut merged = visible;
    merged.extend(agents_skills.into_iter().filter(|s| {
        let top_seg = s.id.split('/').next().unwrap_or("");
        !shadow_ns.contains(&top_seg.to_string())
    }));
    skipped.retain(|s| {
        let top_seg = s
            .path
            .strip_prefix(agents_root)
            .ok()
            .and_then(|p| p.components().next())
            .and_then(|c| c.as_os_str().to_str())
            .unwrap_or("");
        !shadow_ns.contains(&top_seg.to_string())
    });
    merged.sort_by(|a, b| a.id.cmp(&b.id));
    (merged, skipped)
}

/// Merged scan of system prompts from a global root and a local root.
///
/// Same whole-namespace override semantics as [`scan_skills_merged`].
pub fn scan_system_prompts_merged(
    global_root: &Path,
    local_root: &Path,
) -> (Vec<FsPrompt>, Vec<SkipReason>) {
    let local_ns = top_level_namespaces(local_root);

    let (global_prompts, mut global_skipped) = scan_system_prompts(global_root);
    let global_filtered: Vec<FsPrompt> = global_prompts
        .into_iter()
        .filter(|p| {
            // System-prompt paths are under <ns>/system-prompts/<name>.md; the namespace
            // is inferred from the abs_path relative to global_root.
            let top_seg = p
                .abs_path
                .strip_prefix(global_root)
                .ok()
                .and_then(|r| r.components().next())
                .and_then(|c| c.as_os_str().to_str())
                .unwrap_or("");
            !local_ns.contains(&top_seg.to_string())
        })
        .collect();

    global_skipped.retain(|s| {
        let rel = s
            .path
            .strip_prefix(global_root)
            .ok()
            .and_then(|p| p.components().next())
            .and_then(|c| c.as_os_str().to_str())
            .unwrap_or("");
        !local_ns.contains(&rel.to_string())
    });

    let (local_prompts, local_skipped) = scan_system_prompts(local_root);

    let mut merged = local_prompts;
    merged.extend(global_filtered);
    merged.sort_by(|a, b| a.name.cmp(&b.name));

    let mut all_skipped = global_skipped;
    all_skipped.extend(local_skipped);

    (merged, all_skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_fixture(dir: &Path, rel: &str, contents: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    // ── split_frontmatter ────────────────────────────────────────────

    #[test]
    fn split_no_frontmatter() {
        let (fm, body) = split_frontmatter("# Title\nbody\n");
        assert!(fm.is_none());
        assert_eq!(body, "# Title\nbody\n");
    }

    #[test]
    fn split_frontmatter_with_body() {
        let (fm, body) = split_frontmatter("---\nname: open-pr\n---\nThe body.\n");
        assert_eq!(fm, Some("name: open-pr"));
        assert_eq!(body, "The body.\n");
    }

    #[test]
    fn split_frontmatter_only_no_body() {
        let (fm, body) = split_frontmatter("---\nname: x\ndescription: y\n---\n");
        assert_eq!(fm, Some("name: x\ndescription: y"));
        assert_eq!(body, "");
    }

    #[test]
    fn split_frontmatter_eof_fence_no_trailing_newline() {
        let (fm, body) = split_frontmatter("---\nfoo: bar\n---");
        assert_eq!(fm, Some("foo: bar"));
        assert_eq!(body, "");
    }

    #[test]
    fn split_frontmatter_unclosed_falls_back() {
        let content = "---\nname: x\nbody without closing fence\n";
        let (fm, body) = split_frontmatter(content);
        assert!(fm.is_none());
        assert_eq!(body, content);
    }

    #[test]
    fn split_frontmatter_must_start_at_byte_zero() {
        let content = "# header\n---\nname: x\n---\nbody\n";
        let (fm, body) = split_frontmatter(content);
        assert!(fm.is_none());
        assert_eq!(body, content);
    }

    #[test]
    fn split_frontmatter_handles_crlf() {
        let (fm, body) = split_frontmatter("---\r\nname: x\r\n---\r\nbody\r\n");
        assert_eq!(fm, Some("name: x\r"));
        assert_eq!(body, "body\r\n");
    }

    // ── scan_skills ──────────────────────────────────────────────────

    #[test]
    fn scan_skills_basic_nested() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "ns/a.md", "# A\n");
        write_fixture(tmp.path(), "ns/sub/b.md", "# B\n");
        write_fixture(tmp.path(), "ns/sub/deep/c.md", "# C\n");

        let (skills, skipped) = scan_skills(tmp.path());
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");
        let ids: Vec<_> = skills.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["ns/a", "ns/sub/b", "ns/sub/deep/c"]);
    }

    #[test]
    fn scan_skills_excludes_prompts_segment() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "ns/skill.md", "# Skill\n");
        write_fixture(
            tmp.path(),
            "ns/prompts/p.md",
            "---\ndescription: x\n---\nb\n",
        );

        let (skills, skipped) = scan_skills(tmp.path());
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");
        let ids: Vec<_> = skills.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["ns/skill"]);
    }

    #[test]
    fn scan_skills_rejects_invalid_id_segments() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "ns/Bad-Case.md", "# bad\n");
        write_fixture(tmp.path(), "ns/with space.md", "# space\n");
        write_fixture(tmp.path(), "ns/ok.md", "# ok\n");

        let (skills, skipped) = scan_skills(tmp.path());
        let ids: Vec<_> = skills.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["ns/ok"]);
        assert_eq!(skipped.len(), 2);
        for s in &skipped {
            assert_eq!(s.kind, SourceKind::Skill);
            assert!(s.reason.contains("invalid id"));
        }
    }

    #[test]
    fn scan_skills_handles_missing_dir() {
        let (skills, skipped) = scan_skills(Path::new("/no/such/dir/at/all/here"));
        assert!(skills.is_empty());
        assert!(skipped.is_empty());
    }

    #[test]
    fn scan_skills_treats_skills_md_as_index_alias() {
        let tmp = tempfile::tempdir().unwrap();
        let ns = tmp.path().join("resend");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(ns.join("SKILLS.md"), "# resend\n").unwrap();

        let (skills, skipped) = scan_skills(tmp.path());
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].id, "resend/index");
    }

    #[test]
    fn scan_skills_treats_nested_skills_md_as_index_alias() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("resend/emails");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("SKILLS.md"), "# emails\n").unwrap();

        let (skills, skipped) = scan_skills(tmp.path());
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].id, "resend/emails/index");
    }

    #[test]
    fn scan_skills_treats_skill_md_as_index_alias() {
        let tmp = tempfile::tempdir().unwrap();
        let ns = tmp.path().join("my-worker");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(ns.join("SKILL.md"), "# my-worker\n").unwrap();

        let (skills, skipped) = scan_skills(tmp.path());
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].id, "my-worker/index");
    }

    #[test]
    fn scan_skills_collision_index_and_skill_md() {
        // When both index.md and SKILL.md exist in the same namespace,
        // they both map to <ns>/index. Deterministic lex sort means
        // SKILL.md < index.md alphabetically, so SKILL.md wins first-seen
        // and index.md is reported as duplicate.
        let tmp = tempfile::tempdir().unwrap();
        let ns = tmp.path().join("resend");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(ns.join("SKILL.md"), "# from SKILL\n").unwrap();
        std::fs::write(ns.join("index.md"), "# from index\n").unwrap();

        let (skills, skipped) = scan_skills(tmp.path());
        assert_eq!(skills.len(), 1, "should keep exactly one entry");
        assert_eq!(skills[0].id, "resend/index");
        assert_eq!(
            skipped.len(),
            1,
            "second entry should be reported as duplicate"
        );
        assert!(
            skipped[0].reason.contains("duplicate id \"resend/index\""),
            "expected duplicate-id skip, got: {}",
            skipped[0].reason
        );
    }

    #[test]
    fn scan_skills_collision_all_three_aliases() {
        // SKILL.md, SKILLS.md, and index.md all map to <ns>/index.
        // Lex order: SKILL.md < SKILLS.md < index.md — first wins.
        let tmp = tempfile::tempdir().unwrap();
        let ns = tmp.path().join("triple");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(ns.join("SKILL.md"), "# from SKILL\n").unwrap();
        std::fs::write(ns.join("SKILLS.md"), "# from SKILLS\n").unwrap();
        std::fs::write(ns.join("index.md"), "# from index\n").unwrap();

        let (skills, skipped) = scan_skills(tmp.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].id, "triple/index");
        assert_eq!(skipped.len(), 2, "two duplicates should be skipped");
    }

    #[test]
    fn scan_skills_skips_one_when_both_index_and_skills_present() {
        let tmp = tempfile::tempdir().unwrap();
        let ns = tmp.path().join("resend");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(ns.join("index.md"), "# from index\n").unwrap();
        std::fs::write(ns.join("SKILLS.md"), "# from SKILLS\n").unwrap();

        let (skills, skipped) = scan_skills(tmp.path());
        assert_eq!(skills.len(), 1, "should keep exactly one entry");
        assert_eq!(skills[0].id, "resend/index");
        assert_eq!(
            skipped.len(),
            1,
            "second entry should be reported as duplicate"
        );
        assert!(
            skipped[0].reason.contains("duplicate id \"resend/index\""),
            "expected duplicate-id skip, got: {}",
            skipped[0].reason
        );
    }

    // ── scan_system_prompts ─────────────────────────────────────────────────

    #[test]
    fn scan_system_prompts_reads_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns/system-prompts/open-pr.md",
            "---\nname: open-pr\ndescription: Open a PR.\n---\nBody here.\n",
        );

        let (prompts, skipped) = scan_system_prompts(tmp.path());
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].name, "open-pr");
        assert_eq!(prompts[0].description, "Open a PR.");
    }

    #[test]
    fn scan_system_prompts_falls_back_to_filename_for_name() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns/system-prompts/foo.md",
            "---\ndescription: Just a description.\n---\nBody.\n",
        );

        let (prompts, _skipped) = scan_system_prompts(tmp.path());
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].name, "foo");
    }

    #[test]
    fn scan_system_prompts_rejects_missing_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns/system-prompts/no-fm.md",
            "# heading\nbody\n",
        );

        let (prompts, skipped) = scan_system_prompts(tmp.path());
        assert!(prompts.is_empty());
        assert_eq!(skipped.len(), 1);
        assert!(
            skipped[0].reason.contains("missing YAML frontmatter"),
            "got: {:?}",
            skipped[0].reason
        );
    }

    #[test]
    fn scan_system_prompts_rejects_missing_description() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns/system-prompts/no-desc.md",
            "---\nname: foo\n---\nBody.\n",
        );
        let (prompts, skipped) = scan_system_prompts(tmp.path());
        assert!(prompts.is_empty());
        assert_eq!(skipped.len(), 1);
        assert!(
            skipped[0].reason.contains("description"),
            "got: {:?}",
            skipped[0].reason
        );
    }

    #[test]
    fn scan_system_prompts_rejects_invalid_name() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns/system-prompts/bad.md",
            "---\nname: Has Spaces\ndescription: x\n---\nBody.\n",
        );
        let (prompts, skipped) = scan_system_prompts(tmp.path());
        assert!(prompts.is_empty());
        assert_eq!(skipped.len(), 1);
        assert!(
            skipped[0].reason.contains("invalid prompt name"),
            "got: {:?}",
            skipped[0].reason
        );
    }

    #[test]
    fn scan_system_prompts_collision_skips_second() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns-a/system-prompts/shared.md",
            "---\nname: shared\ndescription: from a\n---\nBody A.\n",
        );
        write_fixture(
            tmp.path(),
            "ns-b/system-prompts/shared.md",
            "---\nname: shared\ndescription: from b\n---\nBody B.\n",
        );
        let (prompts, skipped) = scan_system_prompts(tmp.path());
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].name, "shared");
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].reason.contains("duplicate name"));
    }

    // ── read_body ────────────────────────────────────────────────────

    #[test]
    fn read_body_strips_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("foo.md");
        std::fs::write(
            &path,
            "---\nname: foo\ndescription: test\n---\n# Title\n\nBody here.\n",
        )
        .unwrap();
        let body = read_body(&path).unwrap();
        assert_eq!(body, "# Title\n\nBody here.\n");
    }

    #[test]
    fn read_body_no_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("foo.md");
        std::fs::write(&path, "# Title\n\nBody.\n").unwrap();
        let body = read_body(&path).unwrap();
        assert_eq!(body, "# Title\n\nBody.\n");
    }

    #[test]
    fn read_body_rejects_empty_after_frontmatter_strip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("foo.md");
        std::fs::write(&path, "---\nname: x\ndescription: y\n---\n").unwrap();
        let err = read_body(&path).unwrap_err();
        assert!(err.contains("empty body"), "got: {err}");
    }

    #[test]
    fn read_body_enforces_size_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("big.md");
        let body = "x".repeat(SKILL_BODY_MAX_BYTES + 1);
        std::fs::write(&path, &body).unwrap();
        let err = read_body(&path).unwrap_err();
        assert!(err.contains("too large"), "got: {err}");
    }

    #[test]
    fn read_body_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nope.md");
        let err = read_body(&path).unwrap_err();
        assert!(err.contains("read"));
    }

    // ── read_skill_with_frontmatter ──────────────────────────────────

    #[test]
    fn read_with_frontmatter_extracts_title_and_type() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("foo.md");
        std::fs::write(
            &path,
            "---\ntitle: Real title\ntype: how-to\n---\n# Body H1\n\nThe body.\n",
        )
        .unwrap();
        let (fm, body) = read_skill_with_frontmatter(&path).unwrap();
        assert_eq!(fm.title.as_deref(), Some("Real title"));
        assert_eq!(fm.kind.as_deref(), Some("how-to"));
        assert_eq!(body, "# Body H1\n\nThe body.\n");
    }

    #[test]
    fn read_with_frontmatter_defaults_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("plain.md");
        std::fs::write(&path, "# Plain\n\nbody\n").unwrap();
        let (fm, body) = read_skill_with_frontmatter(&path).unwrap();
        assert!(fm.title.is_none());
        assert!(fm.kind.is_none());
        assert_eq!(body, "# Plain\n\nbody\n");
    }

    #[test]
    fn read_with_frontmatter_tolerates_unrelated_yaml_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("rich.md");
        std::fs::write(
            &path,
            "---\ntitle: Hi\ntype: index\nfunctions: [a::b]\nfunction_id: c::d\n---\nbody\n",
        )
        .unwrap();
        let (fm, _) = read_skill_with_frontmatter(&path).unwrap();
        assert_eq!(fm.title.as_deref(), Some("Hi"));
        assert_eq!(fm.kind.as_deref(), Some("index"));
    }

    #[test]
    fn read_with_frontmatter_falls_back_on_invalid_yaml() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bad.md");
        std::fs::write(&path, "---\nnot: [valid yaml\n---\n# heading\nbody\n").unwrap();
        let (fm, body) = read_skill_with_frontmatter(&path).unwrap();
        assert!(fm.title.is_none());
        assert!(fm.kind.is_none());
        assert!(body.contains("# heading"));
    }

    #[test]
    fn read_with_frontmatter_extracts_name() {
        // The ~/.agents/skills convention: name + description, no title.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("SKILL.md");
        std::fs::write(
            &path,
            "---\nname: impeccable\ndescription: Use when designing UI.\nversion: 4.0.4\n---\nBody without an H1.\n",
        )
        .unwrap();
        let (fm, _body) = read_skill_with_frontmatter(&path).unwrap();
        assert_eq!(fm.name.as_deref(), Some("impeccable"));
        assert!(fm.title.is_none());
        assert_eq!(fm.description.as_deref(), Some("Use when designing UI."));
    }

    #[test]
    fn read_with_frontmatter_extracts_description() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("desc.md");
        std::fs::write(
            &path,
            "---\ntitle: My skill\ndescription: A short teaser.\n---\n# Heading\n\nBody.\n",
        )
        .unwrap();
        let (fm, _body) = read_skill_with_frontmatter(&path).unwrap();
        assert_eq!(fm.description.as_deref(), Some("A short teaser."));
    }

    #[test]
    fn read_with_frontmatter_description_defaults_to_none() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("no-desc.md");
        std::fs::write(&path, "---\ntitle: Hi\n---\n# Heading\n\nBody.\n").unwrap();
        let (fm, _body) = read_skill_with_frontmatter(&path).unwrap();
        assert!(fm.description.is_none());
    }

    #[test]
    fn read_with_frontmatter_enforces_size_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("big.md");
        let body = "x".repeat(SKILL_BODY_MAX_BYTES + 1);
        std::fs::write(&path, &body).unwrap();
        let err = read_skill_with_frontmatter(&path).unwrap_err();
        assert!(err.contains("too large"), "got: {err}");
    }

    #[test]
    fn read_with_frontmatter_rejects_empty_body() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("only-fm.md");
        std::fs::write(&path, "---\ntitle: x\n---\n").unwrap();
        let err = read_skill_with_frontmatter(&path).unwrap_err();
        assert!(err.contains("empty body"), "got: {err}");
    }

    // ── scan_skills_merged ──────────────────────────────────────────

    #[test]
    fn merged_local_namespace_shadows_global() {
        let global = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();

        // Global has worker-a and worker-b.
        write_fixture(global.path(), "worker-a/index.md", "# Global A\n");
        write_fixture(global.path(), "worker-b/index.md", "# Global B\n");

        // Local has worker-b (shadows global) and worker-c (local-only).
        write_fixture(local.path(), "worker-b/index.md", "# Local B\n");
        write_fixture(local.path(), "worker-c/index.md", "# Local C\n");

        let (skills, skipped) = scan_skills_merged(global.path(), local.path());
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");

        let ids: Vec<&str> = skills.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["worker-a/index", "worker-b/index", "worker-c/index"]
        );

        // worker-b must come from local, not global.
        let worker_b = skills.iter().find(|s| s.id == "worker-b/index").unwrap();
        assert!(
            worker_b.abs_path.starts_with(local.path()),
            "worker-b should come from local root, got: {}",
            worker_b.abs_path.display()
        );
    }

    #[test]
    fn merged_global_only_namespace_still_listed() {
        let global = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();

        write_fixture(global.path(), "only-global/readme.md", "# Global\n");

        let (skills, _skipped) = scan_skills_merged(global.path(), local.path());
        let ids: Vec<&str> = skills.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["only-global/readme"]);
    }

    #[test]
    fn merged_empty_local_dir_shadows_global_namespace() {
        // Mere existence of the directory in local is enough to shadow.
        let global = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();

        write_fixture(global.path(), "worker-x/index.md", "# Global X\n");
        // Create local worker-x directory with no .md files.
        std::fs::create_dir_all(local.path().join("worker-x")).unwrap();

        let (skills, _skipped) = scan_skills_merged(global.path(), local.path());
        assert!(
            skills.is_empty(),
            "empty local dir should shadow global; got: {:?}",
            skills.iter().map(|s| &s.id).collect::<Vec<_>>()
        );
    }

    // ── scan_agents_skills / merge_agents_root ───────────────────────

    #[test]
    fn scan_agents_skills_shallow_only_skill_md() {
        let agents = tempfile::tempdir().unwrap();
        write_fixture(agents.path(), "impeccable/SKILL.md", "# Impeccable\n");
        write_fixture(agents.path(), "impeccable/reference/polish.md", "# Ref\n");
        write_fixture(agents.path(), "impeccable/scripts/x.md", "# Script doc\n");
        // A dir without SKILL.md and a stray top-level file are ignored.
        write_fixture(agents.path(), "no-entry/reference/a.md", "# A\n");
        write_fixture(agents.path(), "stray.md", "# Stray\n");

        let (skills, skipped) = scan_agents_skills(agents.path());
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");
        let ids: Vec<&str> = skills.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["impeccable/index"]);
        assert!(skills[0].abs_path.ends_with("impeccable/SKILL.md"));
    }

    #[test]
    fn scan_agents_skills_missing_dir_is_empty() {
        let (skills, skipped) = scan_agents_skills(Path::new("/no/such/agents/dir"));
        assert!(skills.is_empty());
        assert!(skipped.is_empty());
    }

    #[test]
    fn scan_agents_skills_skips_invalid_dir_name() {
        let agents = tempfile::tempdir().unwrap();
        write_fixture(agents.path(), "My-Skill/SKILL.md", "# Bad case\n");
        write_fixture(agents.path(), "good-skill/SKILL.md", "# Good\n");

        let (skills, skipped) = scan_agents_skills(agents.path());
        let ids: Vec<&str> = skills.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["good-skill/index"]);
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].reason.contains("invalid id"));
    }

    #[test]
    fn merge_agents_root_global_or_local_dir_shadows() {
        let global = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let agents = tempfile::tempdir().unwrap();

        write_fixture(agents.path(), "impeccable/SKILL.md", "# Agents\n");
        write_fixture(agents.path(), "solo/SKILL.md", "# Solo\n");
        // Mere existence of the namespace dir in global shadows agents...
        std::fs::create_dir_all(global.path().join("impeccable")).unwrap();
        // ...and a local dir shadows too.
        write_fixture(agents.path(), "localized/SKILL.md", "# Agents L\n");
        std::fs::create_dir_all(local.path().join("localized")).unwrap();

        let (visible, _) = scan_skills_merged(global.path(), local.path());
        let (merged, skipped) =
            merge_agents_root(visible, global.path(), local.path(), agents.path());
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");
        let ids: Vec<&str> = merged.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["solo/index"]);
    }

    #[test]
    fn agents_namespaces_requires_skill_md() {
        // A stray skill-less directory must not count as an agents
        // namespace: it would wrongly exempt same-named stale skills from
        // filter_unregistered and reserve the namespace against create.
        let agents = tempfile::tempdir().unwrap();
        write_fixture(agents.path(), "real-skill/SKILL.md", "# Real\n");
        std::fs::create_dir_all(agents.path().join("stray-dir")).unwrap();

        assert_eq!(
            agents_namespaces(agents.path()),
            vec!["real-skill".to_string()]
        );
        assert!(agents_namespaces(Path::new("/no/such/agents/dir")).is_empty());
    }

    #[test]
    fn merge_agents_root_appends_and_sorts() {
        let global = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let agents = tempfile::tempdir().unwrap();

        write_fixture(global.path(), "worker-z/index.md", "# Z\n");
        write_fixture(agents.path(), "aaa-skill/SKILL.md", "# A\n");

        let (visible, _) = scan_skills_merged(global.path(), local.path());
        let (merged, _) = merge_agents_root(visible, global.path(), local.path(), agents.path());
        let ids: Vec<&str> = merged.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["aaa-skill/index", "worker-z/index"]);
    }

    // ── third kind: classification + kind-scoped scans ───────────────

    #[test]
    fn classify_rel_path_ignores_command_prompt_paths() {
        use std::path::Path;
        assert_eq!(
            classify_rel_path(Path::new("ns/a.md")),
            Some(SourceKind::Skill)
        );
        assert_eq!(classify_rel_path(Path::new("ns/prompts/a.md")), None);
        assert_eq!(
            classify_rel_path(Path::new("system-prompts/a.md")),
            Some(SourceKind::SystemPrompt)
        );
        assert_eq!(
            classify_rel_path(Path::new("ns/system-prompts/a.md")),
            Some(SourceKind::SystemPrompt)
        );
        // Both segments, either order: system-prompts wins, exactly one kind.
        assert_eq!(
            classify_rel_path(Path::new("system-prompts/prompts/a.md")),
            Some(SourceKind::SystemPrompt)
        );
        assert_eq!(
            classify_rel_path(Path::new("ns/prompts/system-prompts/a.md")),
            Some(SourceKind::SystemPrompt)
        );
        // Component-boundary near-misses: a segment name that merely
        // starts with (or extends) "prompts"/"system-prompts" is a
        // distinct path component and must NOT match — classification is
        // by exact component equality, not substring.
        assert_eq!(
            classify_rel_path(Path::new("promptsx/foo.md")),
            Some(SourceKind::Skill)
        );
        assert_eq!(
            classify_rel_path(Path::new("ns/prompts-extra/a.md")),
            Some(SourceKind::Skill)
        );
        assert_eq!(
            classify_rel_path(Path::new("system-promptsx/a.md")),
            Some(SourceKind::Skill)
        );
    }

    #[test]
    fn scan_system_prompts_and_skills_scan_excludes_ignored_paths() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns/prompts/cmd.md",
            "---\ndescription: c\n---\nB\n",
        );
        write_fixture(
            tmp.path(),
            "system-prompts/sys.md",
            "---\ndescription: s\n---\nB\n",
        );
        let (sys, skipped) = scan_system_prompts(tmp.path());
        assert!(skipped.is_empty(), "{skipped:?}");
        let names: Vec<&str> = sys.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["sys"]);
        // Neither prompt-ish file leaks into the skills scan.
        let (skills, _) = scan_skills(tmp.path());
        assert!(skills.is_empty(), "{skills:?}");
    }

    #[test]
    fn system_prompt_wins_when_a_path_has_both_segments() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns/prompts/hello.md",
            "---\ndescription: c\n---\nC\n",
        );
        write_fixture(
            tmp.path(),
            "system-prompts/hello.md",
            "---\ndescription: s\n---\nS\n",
        );
        let (sys, skipped) = scan_system_prompts(tmp.path());
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(sys.len(), 1);
        assert_eq!(sys[0].description, "s");
    }

    // ── fourth kind: agents ──────────────────────────────────────────

    #[test]
    fn classify_rel_path_agents_kind_and_precedence() {
        assert_eq!(
            classify_rel_path(Path::new("agents/captain.md")),
            Some(SourceKind::Agent)
        );
        assert_eq!(
            classify_rel_path(Path::new("ns/agents/captain.md")),
            Some(SourceKind::Agent)
        );
        // Prompt-ish segments win over agents regardless of order.
        assert_eq!(classify_rel_path(Path::new("ns/agents/prompts/a.md")), None);
        assert_eq!(
            classify_rel_path(Path::new("system-prompts/agents/a.md")),
            Some(SourceKind::SystemPrompt)
        );
        // Component-boundary near-misses stay skills.
        assert_eq!(
            classify_rel_path(Path::new("agentsx/a.md")),
            Some(SourceKind::Skill)
        );
        assert_eq!(
            classify_rel_path(Path::new("ns/agents-extra/a.md")),
            Some(SourceKind::Skill)
        );
    }

    #[test]
    fn scan_agents_parses_full_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "agents/captain.md",
            "---\nname: Release Captain\ndescription: Cuts releases.\nlogo: \"🚢\"\nskills:\n  - iii-sandbox\ndelegates_to: [frontend]\nleaf: true\nmodel: codex/gpt-5.4-mini\n---\nYou are the captain.\n",
        );
        let (agents, skipped) = scan_agents(tmp.path());
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");
        assert_eq!(agents.len(), 1);
        let a = &agents[0];
        assert_eq!(a.name, "captain");
        assert_eq!(a.display_name, "Release Captain");
        assert_eq!(a.description, "Cuts releases.");
        assert_eq!(a.logo.as_deref(), Some("🚢"));
        assert_eq!(a.skills, vec!["iii-sandbox".to_string()]);
        assert_eq!(
            a.delegates_to.as_deref(),
            Some(&["frontend".to_string()][..])
        );
        assert!(a.leaf);
        assert_eq!(a.model.as_deref(), Some("codex/gpt-5.4-mini"));
    }

    #[test]
    fn scan_agents_empty_description_and_absent_optionals_ok() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "agents/min.md", "---\nname: Min\n---\nBody.\n");
        let (agents, skipped) = scan_agents(tmp.path());
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");
        let a = &agents[0];
        assert_eq!(a.description, "");
        assert!(a.logo.is_none());
        assert!(a.skills.is_empty());
        assert!(a.delegates_to.is_none());
        assert!(!a.leaf);
        assert!(a.model.is_none());
    }

    #[test]
    fn scan_agents_skips_invalid_files() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "agents/no-fm.md", "no frontmatter\n");
        write_fixture(
            tmp.path(),
            "agents/no-name.md",
            "---\ndescription: x\n---\nB\n",
        );
        write_fixture(
            tmp.path(),
            "agents/bad-logo.md",
            "---\nname: X\nlogo: ./logo.png\n---\nB\n",
        );
        write_fixture(tmp.path(), "agents/Bad-Stem.md", "---\nname: X\n---\nB\n");
        write_fixture(tmp.path(), "agents/good.md", "---\nname: Good\n---\nB\n");
        let (agents, skipped) = scan_agents(tmp.path());
        let ids: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(ids, vec!["good"]);
        assert_eq!(skipped.len(), 4, "{skipped:?}");
        for s in &skipped {
            assert_eq!(s.kind, SourceKind::Agent);
        }
        let reasons = skipped
            .iter()
            .map(|s| s.reason.as_str())
            .collect::<Vec<_>>();
        assert!(reasons
            .iter()
            .any(|r| r.contains("missing YAML frontmatter")));
        assert!(reasons.iter().any(|r| r.contains("non-empty `name`")));
        assert!(reasons.iter().any(|r| r.contains("emoji only")));
        assert!(reasons.iter().any(|r| r.contains("invalid agent id")));
    }

    #[test]
    fn scan_agents_duplicate_stem_first_wins() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "agents/shared.md", "---\nname: Top\n---\nA\n");
        write_fixture(tmp.path(), "ns/agents/shared.md", "---\nname: Ns\n---\nB\n");
        let (agents, skipped) = scan_agents(tmp.path());
        assert_eq!(agents.len(), 1);
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].reason.contains("duplicate id \"shared\""));
    }

    #[test]
    fn scan_skills_excludes_agents_segment() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "ns/skill.md", "# Skill\n");
        write_fixture(tmp.path(), "ns/agents/a.md", "---\nname: A\n---\nB\n");
        let (skills, skipped) = scan_skills(tmp.path());
        assert!(skipped.is_empty(), "{skipped:?}");
        let ids: Vec<_> = skills.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["ns/skill"]);
    }

    #[test]
    fn scan_agents_merged_local_namespace_shadows_global() {
        let global = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        write_fixture(global.path(), "agents/a.md", "---\nname: GA\n---\nG\n");
        write_fixture(local.path(), "agents/b.md", "---\nname: LB\n---\nL\n");
        // Local top-level `agents/` dir shadows the global one wholesale.
        let (merged, _) = scan_agents_merged(global.path(), local.path());
        let ids: Vec<&str> = merged.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(ids, vec!["b"]);
    }

    #[test]
    fn validate_agent_logo_rules() {
        assert!(validate_agent_logo("🚢").is_ok());
        assert!(validate_agent_logo("⚡🔥").is_ok());
        assert!(validate_agent_logo("").is_err());
        assert!(validate_agent_logo("./x.png").is_err());
        assert!(validate_agent_logo("a b").is_err());
        assert!(validate_agent_logo("🚢🚢🚢🚢🚢").is_err(), "over byte cap");
    }

    #[test]
    fn scan_system_prompts_merged_shadows_per_top_level_namespace() {
        let tmp = tempfile::tempdir().unwrap();
        let global = tmp.path().join("global");
        let local = tmp.path().join("local");
        write_fixture(
            &global,
            "system-prompts/a.md",
            "---\ndescription: g\n---\nG\n",
        );
        write_fixture(
            &local,
            "system-prompts/b.md",
            "---\ndescription: l\n---\nL\n",
        );
        // A local top-level `system-prompts/` dir shadows the global one
        // wholesale — same semantics every namespace already has.
        let (merged, _) = scan_system_prompts_merged(&global, &local);
        let names: Vec<&str> = merged.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["b"]);
    }
}
