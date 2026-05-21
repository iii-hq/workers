//! Filesystem-backed skills reader.
//!
//! Public API (reachable by any worker over `iii.trigger`):
//!
//!   * `directory::skills::list` — enriched listing of every markdown
//!     skill under `skills_folder`, sorted by id. Each row carries
//!     `id`, `title`, `type`, `description`, `bytes`, and `modified_at`
//!     so a consumer can render a picker / index in one round trip
//!     without follow-up `get` calls per row.
//!   * `directory::skills::get`  — fetch one skill by id. Returns
//!     `{ id, title, type, description, body, modified_at }` — the
//!     same flat shape `directory::prompts::get` returns for prompts
//!     plus `type` from the file's YAML frontmatter.
//!
//! Title resolution precedence (shared by `list` and `get`): the YAML
//! frontmatter `title:` wins when present and non-empty, then the
//! first `# H1` line in the body, with the bare id as final fallback.
//! `type` is read straight from the frontmatter `type:` key (e.g.
//! `index`, `how-to`, `reference`) and serialised as `null` when the
//! file omits it.
//!
//! There are no write paths in this module. Files arrive on disk via
//! `directory::skills::download` (see [`crate::functions::download`])
//! or by direct editing under `skills_folder`. Mutations fan out
//! through the `directory::skills::on-change` trigger type which is
//! fired from the download function on success.

use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, III};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;

use crate::config::SkillsConfig;
use crate::fs_source::{self, FsSkill, SkillFrontmatter};

/// Soft-cap on a single skill body (matches the historic state-backed
/// limit the registry enforced).
pub const SKILL_BODY_MAX_BYTES: usize = 256 * 1024;

/// Per-segment cap for both skill ids and section URI segments. The
/// total id is allowed to chain many segments via `/`, but each
/// individual segment stays short so directory listings stay readable.
const ID_SEGMENT_MAX_LEN: usize = 64;

/// Soft ceiling on the slashed id length. With the per-segment cap above
/// this allows depth ~16 in practice — far deeper than any reasonable
/// tree, while preventing pathological inputs.
const ID_TOTAL_MAX_LEN: usize = 1024;

/// `iii://` prefix accepted on `get` inputs as a convenience so callers
/// can paste a link target verbatim. The prefix is stripped before id
/// validation; any other URI scheme (`https://`, `ftp://`, ...) is
/// rejected.
const URI_PREFIX: &str = "iii://";

/// Description for the `directory::skills::get` registration.
const GET_DESCRIPTION: &str =
    "Fetch one filesystem-backed skill by id. Returns the raw markdown body plus id, \
     title, type, description, and modified_at — same flat shape as directory::prompts::get \
     with `type` lifted from the YAML frontmatter and `title` preferring frontmatter \
     over the body H1. Accepts a bare id (e.g. \"directory/skills/list\"), the same id \
     suffixed with `.md` (e.g. \"directory/skills/list.md\"), or either form prefixed \
     with iii://.";

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct ListSkillsInput {}

#[derive(Debug, Serialize, JsonSchema)]
struct SkillEntry {
    id: String,
    /// Frontmatter `title:` when present and non-empty, otherwise the
    /// first `# H1` line in the body, otherwise the bare `id`.
    title: String,
    /// Frontmatter `type:` (e.g. `index`, `how-to`, `reference`).
    /// `null` when the file has no frontmatter or omits the key.
    #[serde(rename = "type")]
    kind: Option<String>,
    /// First paragraph of the body, empty when the file has only headings.
    description: String,
    bytes: usize,
    /// File mtime as RFC 3339 (best effort; empty if unavailable).
    modified_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ListSkillsOutput {
    skills: Vec<SkillEntry>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct IndexSkillsInput {}

#[derive(Debug, Serialize, JsonSchema)]
struct IndexSkillsOutput {
    /// Rendered markdown document — one short `## <title>` block per
    /// installed worker (skills with frontmatter `type: index`),
    /// carrying the worker's first-paragraph overview and a read-more
    /// link pointing at the file path `<ns>/index.md`. Sorted lex by id.
    body: String,
    /// Number of worker entries rendered (i.e. the count of
    /// `type: index` skills that survived the filter). Cheap sanity
    /// check that doesn't require re-parsing the body.
    workers_count: usize,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SkillGetInput {
    /// Skill id (the same string returned by `directory::skills::list`,
    /// e.g. `"directory/skills/list"`). Two ergonomic variants are also
    /// accepted: the file-path form `<id>.md` (the trailing `.md` is
    /// stripped) and the legacy `iii://{id}` URI form. Other URI
    /// schemes are rejected. The filename `SKILLS.md` is aliased to
    /// `index.md` to match the filesystem scanner.
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SkillGetOutput {
    pub id: String,
    /// Frontmatter `title:` when present and non-empty, otherwise the
    /// first `# H1` line in the body, otherwise the bare `id`.
    pub title: String,
    /// Frontmatter `type:` (e.g. `index`, `how-to`, `reference`).
    /// `null` when the file has no frontmatter or omits the key.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub description: String,
    /// Raw markdown body (post-frontmatter) from disk.
    pub body: String,
    /// File mtime as RFC 3339.
    pub modified_at: String,
}

pub fn register(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    register_list_skills(iii, cfg);
    register_get_skill(iii, cfg);
    register_index_skills(iii, cfg);
}

fn register_list_skills(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async("directory::skills::list", move |_input: ListSkillsInput| {
            let cfg = cfg_inner.clone();
            async move {
                let (entries, _skipped) = fs_source::scan_skills(&cfg.resolved_skills_folder());
                let out: Vec<SkillEntry> = entries.into_iter().map(skill_entry_from_fs).collect();
                Ok::<_, IIIError>(ListSkillsOutput { skills: out })
            }
        })
        .description(
            "List filesystem-backed skills (id, title, type, description, bytes, modified_at) \
             from skills_folder. `title` prefers the YAML frontmatter `title:` over the body H1, \
             `type` is lifted from frontmatter `type:`, and `description` is the first paragraph \
             of the body — so consumers can render a picker or indented index without one get \
             per row.",
        ),
    );
}

fn register_get_skill(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async("directory::skills::get", move |req: SkillGetInput| {
            let cfg = cfg_inner.clone();
            async move { get_skill(&cfg, req).await.map_err(IIIError::Handler) }
        })
        .description(GET_DESCRIPTION)
        .metadata(json!({"tool": {"label": "Get skill"}})),
    );
}

fn register_index_skills(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async(
            "directory::skills::index",
            move |_input: IndexSkillsInput| {
                let cfg = cfg_inner.clone();
                async move {
                    let (entries, _skipped) = fs_source::scan_skills(&cfg.resolved_skills_folder());
                    let rows: Vec<SkillEntry> =
                        entries.into_iter().map(skill_entry_from_fs).collect();
                    let body = render_index_markdown(&rows);
                    let workers_count = rows
                        .iter()
                        .filter(|e| e.kind.as_deref() == Some("index"))
                        .count();
                    Ok::<_, IIIError>(IndexSkillsOutput {
                        body,
                        workers_count,
                    })
                }
            },
        )
        .description(
            "Render one short markdown entry per installed worker (skills with frontmatter \
             `type: index`). Each entry is a `## <worker title>` heading, the first paragraph \
             of the worker's overview, and a `Read <ns>/index.md` line the agent can \
             follow via `directory::skills::get` for the full reference. Token-light by \
             design; for per-skill rows use `directory::skills::list`.",
        ),
    );
}

// ---------- core handler ----------

pub async fn get_skill(cfg: &SkillsConfig, req: SkillGetInput) -> Result<SkillGetOutput, String> {
    let id = normalize_get_id(&req.id)?;
    validate_id(&id)?;
    let Some(fs) = find_fs_skill(cfg, &id) else {
        // Include a remediation hint in the error so a calling LLM agent
        // can self-correct on the next turn. Without this, models tend to
        // hallucinate a sibling id and retry the same not-found pattern
        // instead of listing what actually exists. Investigation:
        // model asked for "sandbox/create" which doesn't exist; agent
        // would have recovered if the error pointed at the catalog.
        return Err(format!(
            "Skill not found: {id}. List available skills via `directory::skills::list`, \
             or browse worker overviews via `directory::skills::index`."
        ));
    };
    let (fm, body) = fs_source::read_skill_with_frontmatter(&fs.abs_path)?;
    let title = resolve_title(&fm, &body, &fs.id);
    let kind = clean_optional(fm.kind);
    let description = extract_description(&body).unwrap_or_default();
    let (_, modified_at) = fs_metadata(&fs);
    Ok(SkillGetOutput {
        id: fs.id,
        title,
        kind,
        description,
        body,
        modified_at,
    })
}

/// Trim and strip an optional `iii://` prefix; reject any other URI
/// scheme. Also accepts a file-path form: a trailing `.md` is stripped
/// so callers can paste either `hello-worker/index` or
/// `hello-worker/index.md` and get the same id. The literal filename
/// `SKILLS.md` (final path component) is aliased to `index.md` — same
/// rule the filesystem scanner uses. The remaining string still has to
/// satisfy [`validate_id`]; this function only handles the prefix /
/// suffix ergonomics.
pub fn normalize_get_id(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("id must be non-empty".into());
    }
    let without_scheme = if let Some(rest) = trimmed.strip_prefix(URI_PREFIX) {
        rest
    } else if trimmed.contains("://") {
        return Err(format!(
            "Invalid id (must be a bare skill path, a path ending in .md, or an iii:// URI): {trimmed}"
        ));
    } else {
        trimmed
    };
    let aliased = if let Some(stem) = without_scheme.strip_suffix("/SKILLS.md") {
        format!("{stem}/index")
    } else if without_scheme == "SKILLS.md" {
        "index".to_string()
    } else {
        without_scheme
            .strip_suffix(".md")
            .unwrap_or(without_scheme)
            .to_string()
    };
    Ok(aliased)
}

// ---------- validation ----------

/// Validate a single id segment.
pub fn validate_id_segment(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("segment must be non-empty".into());
    }
    if s.len() > ID_SEGMENT_MAX_LEN {
        return Err(format!(
            "segment too long ({} chars; max {ID_SEGMENT_MAX_LEN}): {s:?}",
            s.len()
        ));
    }
    for c in s.chars() {
        let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_';
        if !ok {
            return Err(format!(
                "segment may only contain lowercase ASCII letters, digits, '-' and '_': {s:?}"
            ));
        }
    }
    Ok(())
}

/// Validate a full skill id. Accepts 1+ segments separated by `/`.
pub fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("id must be non-empty".into());
    }
    if id.starts_with('/') || id.ends_with('/') {
        return Err(format!("id may not have a leading or trailing '/': {id:?}"));
    }
    if id.len() > ID_TOTAL_MAX_LEN {
        return Err(format!(
            "id too long ({} chars; max {ID_TOTAL_MAX_LEN}): {id:?}",
            id.len()
        ));
    }
    let segments: Vec<&str> = id.split('/').collect();
    for (i, seg) in segments.iter().enumerate() {
        validate_id_segment(seg)
            .map_err(|e| format!("invalid id (segment {} of {:?}): {e}", i + 1, id))?;
    }
    Ok(())
}

// ---------- markdown helpers ----------

pub fn extract_title(markdown: &str) -> Option<&str> {
    markdown.lines().find_map(|line| {
        let trimmed = line.trim_start();
        trimmed.strip_prefix("# ").map(|s| s.trim())
    })
}

/// Pick the best title for a skill: frontmatter `title:` (when present
/// and non-empty after trim), then the first body `# H1`, then the
/// bare `id` so the response field is never empty.
pub fn resolve_title(fm: &SkillFrontmatter, body: &str, id: &str) -> String {
    if let Some(t) = fm.title.as_deref() {
        let trimmed = t.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Some(h1) = extract_title(body) {
        if !h1.is_empty() {
            return h1.to_string();
        }
    }
    id.to_string()
}

/// Trim, then drop the value when the result is empty. Used to keep
/// the response `type` field as `null` rather than an empty string
/// when the frontmatter declares `type:` with no value.
pub fn clean_optional(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn extract_description(markdown: &str) -> Option<String> {
    let mut buf = String::new();
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !buf.is_empty() {
                break;
            }
            continue;
        }
        if trimmed.starts_with('#') {
            if !buf.is_empty() {
                break;
            }
            continue;
        }
        if !buf.is_empty() {
            buf.push(' ');
        }
        buf.push_str(trimmed);
    }
    if buf.is_empty() {
        return None;
    }
    Some(buf)
}

/// Render a `directory::skills::index` markdown document from already
/// title/description-resolved rows. Filters down to entries with
/// frontmatter `type: index` (one per installed worker) and emits a
/// compact per-worker block:
///
/// ```markdown
/// # Skills index
///
/// N worker(s).
///
/// ## <resolved title>
///
/// <first paragraph from the worker's overview>
///
/// Read [`<id>.md`](<id>.md) (legacy `iii://<id>`) for the full worker reference.
/// ```
///
/// The legacy `iii://<id>` form is emitted alongside the file-path
/// pointer so harnesses that grep for the old URI scheme keep working
/// while new consumers prefer the markdown link target.
///
/// The description block is omitted (no extra blank line) when the
/// overview body has no paragraph. Entries must already be sorted lex
/// by `id` (the order `fs_source::scan_skills` returns); this function
/// does not re-sort.
fn render_index_markdown(entries: &[SkillEntry]) -> String {
    let workers: Vec<&SkillEntry> = entries
        .iter()
        .filter(|e| e.kind.as_deref() == Some("index"))
        .collect();

    let mut out = String::new();
    out.push_str("# Skills index\n\n");
    out.push_str(&format!("{} worker(s).\n", workers.len()));

    for worker in workers {
        out.push('\n');
        out.push_str(&format!("## {}\n", worker.title));
        if !worker.description.is_empty() {
            out.push('\n');
            out.push_str(&format!("{}\n", worker.description));
        }
        out.push('\n');
        out.push_str(&format!(
            "Read [`{id}.md`]({id}.md) (legacy `iii://{id}`) for the full worker reference.\n",
            id = worker.id
        ));
    }

    out
}

// ---------- fs lookup ----------

/// Targeted lookup for the read path. Returns `None` if no file under
/// `skills_folder` matches `id`.
///
/// A **bare worker name** with no `/` is treated as shorthand for
/// `<id>/index`. So `find_fs_skill(cfg, "sandbox")` returns the same
/// skill as `find_fs_skill(cfg, "sandbox/index")` whenever
/// `sandbox/index.md` exists and no literal `sandbox.md` shadows it.
/// Multi-segment ids (`sandbox/exec`) must match literally — no
/// recursive `/index` expansion, so a typo never silently resolves to
/// the wrong skill.
fn find_fs_skill(cfg: &SkillsConfig, id: &str) -> Option<FsSkill> {
    let (fs, _skipped) = fs_source::scan_skills(&cfg.resolved_skills_folder());
    let alias = (!id.contains('/')).then(|| format!("{id}/index"));
    let mut exact: Option<FsSkill> = None;
    let mut aliased: Option<FsSkill> = None;
    for skill in fs {
        if skill.id == id {
            exact = Some(skill);
            continue;
        }
        if alias.as_deref() == Some(skill.id.as_str()) {
            aliased = Some(skill);
        }
    }
    exact.or(aliased)
}

/// Build a `SkillEntry` for `list` output. Reads the file body and
/// frontmatter so the row carries title + type + description; on read
/// failure the row still surfaces the id with empty title / null type /
/// empty description so a single broken file doesn't hide every other
/// skill from the picker.
fn skill_entry_from_fs(fs: FsSkill) -> SkillEntry {
    let (bytes, modified_at) = fs_metadata(&fs);
    let (title, kind, description) = match fs_source::read_skill_with_frontmatter(&fs.abs_path) {
        Ok((fm, body)) => {
            let title = resolve_title(&fm, &body, &fs.id);
            let kind = clean_optional(fm.kind);
            let description = extract_description(&body).unwrap_or_default();
            (title, kind, description)
        }
        Err(_) => (fs.id.clone(), None, String::new()),
    };
    SkillEntry {
        id: fs.id,
        title,
        kind,
        description,
        bytes,
        modified_at,
    }
}

/// Cheap metadata for `skills::list` rows. Bytes is the on-disk file
/// size; `modified_at` is the file's mtime as RFC 3339.
fn fs_metadata(skill: &FsSkill) -> (usize, String) {
    match std::fs::metadata(&skill.abs_path) {
        Ok(meta) => {
            let bytes = meta.len() as usize;
            let modified = meta
                .modified()
                .ok()
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
                .unwrap_or_default();
            (bytes, modified)
        }
        Err(_) => (0, String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── normalize_get_id ────────────────────────────────────────────────

    #[test]
    fn normalize_accepts_bare_id() {
        assert_eq!(
            normalize_get_id("agent-memory/observe").unwrap(),
            "agent-memory/observe"
        );
    }

    #[test]
    fn normalize_strips_iii_prefix() {
        assert_eq!(
            normalize_get_id("iii://agent-memory/observe").unwrap(),
            "agent-memory/observe"
        );
    }

    #[test]
    fn normalize_trims_whitespace() {
        assert_eq!(normalize_get_id("  iii://foo  ").unwrap(), "foo");
        assert_eq!(normalize_get_id("\nfoo\t").unwrap(), "foo");
    }

    #[test]
    fn normalize_rejects_empty() {
        assert!(normalize_get_id("").is_err());
        assert!(normalize_get_id("   ").is_err());
    }

    #[test]
    fn normalize_rejects_other_uri_schemes() {
        let err = normalize_get_id("https://example.com").unwrap_err();
        assert!(err.contains("iii://"), "got: {err}");
        assert!(normalize_get_id("ftp://nope").is_err());
    }

    #[test]
    fn normalize_strips_md_suffix_on_bare_path() {
        assert_eq!(
            normalize_get_id("hello-worker/index.md").unwrap(),
            "hello-worker/index"
        );
    }

    #[test]
    fn normalize_aliases_skills_md_to_index() {
        assert_eq!(
            normalize_get_id("hello-worker/SKILLS.md").unwrap(),
            "hello-worker/index"
        );
    }

    #[test]
    fn normalize_aliases_nested_skills_md_to_index() {
        assert_eq!(
            normalize_get_id("resend/emails/SKILLS.md").unwrap(),
            "resend/emails/index"
        );
    }

    #[test]
    fn normalize_strips_md_after_iii_prefix() {
        assert_eq!(
            normalize_get_id("iii://hello-worker/index.md").unwrap(),
            "hello-worker/index"
        );
    }

    #[test]
    fn normalize_does_not_strip_md_in_middle_of_path() {
        // ".md" inside a segment is a real id, not a file suffix.
        assert_eq!(
            normalize_get_id("hello-worker/index_md").unwrap(),
            "hello-worker/index_md"
        );
    }

    // ── iii:// back-compat ─────────────────────────────────────────────

    #[test]
    fn normalize_iii_prefix_with_skills_md_aliases_to_index() {
        // `iii://` + `SKILLS.md` filename composes through both transforms.
        assert_eq!(normalize_get_id("iii://ns/SKILLS.md").unwrap(), "ns/index");
    }

    #[test]
    fn normalize_iii_prefix_with_nested_skills_md_aliases_to_index() {
        assert_eq!(
            normalize_get_id("iii://resend/emails/SKILLS.md").unwrap(),
            "resend/emails/index"
        );
    }

    #[test]
    fn normalize_iii_prefix_round_trips_with_render_emitted_id() {
        // The `iii://<id>` token render_index_markdown emits for the
        // legacy-pointer footer must parse back through normalize_get_id
        // without modification.
        let emitted = "iii://agent-memory/index";
        assert_eq!(normalize_get_id(emitted).unwrap(), "agent-memory/index");
    }

    // ── validate_id: happy paths ────────────────────────────────────────

    #[test]
    fn id_validation_accepts_single_segment() {
        assert!(validate_id("brain").is_ok());
        assert!(validate_id("agent_memory").is_ok());
        assert!(validate_id("my-skill-1").is_ok());
        assert!(validate_id("a").is_ok());
    }

    #[test]
    fn id_validation_accepts_multi_segment() {
        assert!(validate_id("a/b").is_ok());
        assert!(validate_id("a/b/c").is_ok());
        assert!(validate_id("a/b/c/d/e").is_ok());
        assert!(validate_id("resend/email/send").is_ok());
    }

    #[test]
    fn id_validation_allows_fn_segment_anywhere() {
        assert!(validate_id("fn").is_ok());
        assert!(validate_id("fn/anything").is_ok());
        assert!(validate_id("docs/fn-reference").is_ok());
        assert!(validate_id("a/fn/c").is_ok());
    }

    // ── validate_id: error cases ────────────────────────────────────────

    #[test]
    fn id_validation_rejects_bad_chars() {
        assert!(validate_id("").is_err());
        assert!(validate_id("UpperCase").is_err());
        assert!(validate_id("with space").is_err());
        assert!(validate_id("with::colon").is_err());
    }

    #[test]
    fn id_validation_rejects_leading_or_trailing_slash() {
        assert!(validate_id("/a").is_err());
        assert!(validate_id("a/").is_err());
        assert!(validate_id("a//b").is_err());
    }

    #[test]
    fn id_validation_enforces_per_segment_length() {
        let too_long = "x".repeat(ID_SEGMENT_MAX_LEN + 1);
        assert!(validate_id(&too_long).is_err());
        let nested_with_long_segment = format!("ok/{too_long}");
        assert!(validate_id(&nested_with_long_segment).is_err());
        let max_segment = "x".repeat(ID_SEGMENT_MAX_LEN);
        assert!(validate_id(&max_segment).is_ok());
    }

    #[test]
    fn id_validation_enforces_total_length() {
        let too_long: String = "ab/".repeat((ID_TOTAL_MAX_LEN / 3) + 5);
        let trimmed = too_long.trim_end_matches('/').to_string();
        assert!(trimmed.len() > ID_TOTAL_MAX_LEN);
        assert!(validate_id(&trimmed).is_err());
    }

    // ── extract_title / extract_description ─────────────────────────────

    #[test]
    fn extract_title_finds_h1() {
        let md = "# my skill\n\nbody\n";
        assert_eq!(extract_title(md), Some("my skill"));
    }

    #[test]
    fn extract_title_ignores_h2() {
        let md = "## sub\n\nbody\n";
        assert_eq!(extract_title(md), None);
    }

    #[test]
    fn extract_description_grabs_first_paragraph() {
        let md = "# title\n\nfirst paragraph here.\n\nsecond paragraph.\n";
        assert_eq!(
            extract_description(md).as_deref(),
            Some("first paragraph here.")
        );
    }

    #[test]
    fn extract_description_skips_subheadings() {
        let md = "# title\n\n## sub\n\n### deeper\n\nfinally text.\n";
        assert_eq!(extract_description(md).as_deref(), Some("finally text."));
    }

    #[test]
    fn extract_description_handles_missing_paragraph() {
        let md = "# only a title\n";
        assert_eq!(extract_description(md), None);
    }

    #[test]
    fn extract_description_keeps_long_first_paragraph() {
        let body = "x".repeat(200);
        let md = format!("# t\n\n{body}\n");
        let desc = extract_description(&md).unwrap();
        assert_eq!(desc, body);
        assert!(!desc.contains("..."));
    }

    #[test]
    fn extract_description_stops_at_blank_line() {
        let md = "# t\n\nfirst paragraph here.\n\nsecond paragraph.\n";
        assert_eq!(
            extract_description(md).as_deref(),
            Some("first paragraph here.")
        );
    }

    // ── resolve_title / clean_optional ──────────────────────────────────

    #[test]
    fn resolve_title_prefers_frontmatter_over_h1() {
        let fm = SkillFrontmatter {
            title: Some("Frontmatter wins".into()),
            kind: None,
        };
        assert_eq!(
            resolve_title(&fm, "# Body H1\n\nbody", "ns/foo"),
            "Frontmatter wins"
        );
    }

    #[test]
    fn resolve_title_trims_frontmatter_whitespace() {
        let fm = SkillFrontmatter {
            title: Some("   spaced   ".into()),
            kind: None,
        };
        assert_eq!(resolve_title(&fm, "# H1", "id"), "spaced");
    }

    #[test]
    fn resolve_title_falls_back_to_h1_when_frontmatter_missing() {
        let fm = SkillFrontmatter::default();
        assert_eq!(resolve_title(&fm, "# Body H1\n\nbody", "ns/foo"), "Body H1");
    }

    #[test]
    fn resolve_title_falls_back_to_h1_when_frontmatter_blank() {
        let fm = SkillFrontmatter {
            title: Some("   ".into()),
            kind: None,
        };
        assert_eq!(resolve_title(&fm, "# Body H1", "ns/foo"), "Body H1");
    }

    #[test]
    fn resolve_title_falls_back_to_id_when_no_h1_or_frontmatter() {
        let fm = SkillFrontmatter::default();
        assert_eq!(resolve_title(&fm, "no heading here", "ns/foo"), "ns/foo");
    }

    #[test]
    fn clean_optional_drops_blank_strings() {
        assert_eq!(clean_optional(None), None);
        assert_eq!(clean_optional(Some("".into())), None);
        assert_eq!(clean_optional(Some("   ".into())), None);
        assert_eq!(
            clean_optional(Some(" how-to ".into())),
            Some("how-to".into())
        );
    }

    // ── skill_entry_from_fs ─────────────────────────────────────────────

    #[test]
    fn list_row_pulls_title_and_description_from_body() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("foo.md");
        std::fs::write(&path, "# My title\n\nFirst paragraph.\n").unwrap();
        let fs = FsSkill {
            id: "foo".into(),
            abs_path: path,
        };
        let entry = skill_entry_from_fs(fs);
        assert_eq!(entry.id, "foo");
        assert_eq!(entry.title, "My title");
        assert_eq!(entry.kind, None);
        assert_eq!(entry.description, "First paragraph.");
        assert!(entry.bytes > 0);
        assert!(!entry.modified_at.is_empty());
    }

    #[test]
    fn list_row_prefers_frontmatter_title_and_carries_type() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("foo.md");
        std::fs::write(
            &path,
            "---\ntitle: Real title\ntype: how-to\n---\n# Body H1\n\nFirst paragraph.\n",
        )
        .unwrap();
        let fs = FsSkill {
            id: "foo".into(),
            abs_path: path,
        };
        let entry = skill_entry_from_fs(fs);
        assert_eq!(entry.title, "Real title");
        assert_eq!(entry.kind.as_deref(), Some("how-to"));
        assert_eq!(entry.description, "First paragraph.");
    }

    #[test]
    fn list_row_falls_back_to_id_when_h1_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bare.md");
        std::fs::write(&path, "no heading at all\n").unwrap();
        let fs = FsSkill {
            id: "bare".into(),
            abs_path: path,
        };
        let entry = skill_entry_from_fs(fs);
        assert_eq!(entry.title, "bare");
        assert_eq!(entry.kind, None);
        assert_eq!(entry.description, "no heading at all");
    }

    #[test]
    fn list_row_survives_unreadable_body() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("missing.md");
        let fs = FsSkill {
            id: "missing".into(),
            abs_path: missing,
        };
        let entry = skill_entry_from_fs(fs);
        assert_eq!(entry.title, "missing");
        assert_eq!(entry.kind, None);
        assert_eq!(entry.description, "");
        assert_eq!(entry.bytes, 0);
    }

    // ── get_skill (full handler) ────────────────────────────────────────

    fn cfg_with_skills_folder(root: &std::path::Path) -> SkillsConfig {
        SkillsConfig {
            skills_folder: root.to_string_lossy().into_owned(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn get_prefers_frontmatter_title_and_returns_type() {
        let tmp = tempfile::tempdir().unwrap();
        let ns = tmp.path().join("ns");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(
            ns.join("doc.md"),
            "---\ntitle: Real title\ntype: how-to\n---\n# Body H1\n\nThe body.\n",
        )
        .unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let out = get_skill(
            &cfg,
            SkillGetInput {
                id: "ns/doc".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(out.id, "ns/doc");
        assert_eq!(out.title, "Real title");
        assert_eq!(out.kind.as_deref(), Some("how-to"));
        assert!(out.body.contains("Body H1"));
    }

    #[tokio::test]
    async fn get_falls_back_to_h1_without_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let ns = tmp.path().join("ns");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(ns.join("plain.md"), "# Just an H1\n\nbody.\n").unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let out = get_skill(
            &cfg,
            SkillGetInput {
                id: "ns/plain".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(out.title, "Just an H1");
        assert_eq!(out.kind, None);
    }

    #[tokio::test]
    async fn get_skill_not_found_error_points_agent_at_directory_skills_list() {
        // LLM agents calling directory::skills::get tend to guess skill
        // ids (observed: "sandbox/create" hallucinated). The error must
        // include a remediation hint that gets the agent into a recovery
        // loop instead of doubling down on the wrong path.
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let err = get_skill(
            &cfg,
            SkillGetInput {
                id: "sandbox/create".into(),
            },
        )
        .await
        .expect_err("should error on missing skill");
        // The id itself stays in the message so logs still pin which id
        // was requested.
        assert!(
            err.contains("Skill not found: sandbox/create"),
            "missing id in error: {err}",
        );
        // The hint must mention the catalog-listing function the agent
        // should call next.
        assert!(
            err.contains("directory::skills::list"),
            "missing list-hint in error: {err}",
        );
        assert!(
            err.contains("directory::skills::index"),
            "missing index-hint in error: {err}",
        );
    }

    #[tokio::test]
    async fn get_serialises_type_field_with_correct_json_key() {
        let tmp = tempfile::tempdir().unwrap();
        let ns = tmp.path().join("ns");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(
            ns.join("doc.md"),
            "---\ntitle: T\ntype: index\n---\n# H\n\nb\n",
        )
        .unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let out = get_skill(
            &cfg,
            SkillGetInput {
                id: "ns/doc".into(),
            },
        )
        .await
        .unwrap();
        let v = serde_json::to_value(&out).unwrap();
        assert_eq!(v["type"].as_str(), Some("index"));
        assert!(v.get("kind").is_none(), "kind should be renamed to type");
        assert!(v["title"].as_str() == Some("T"));
    }

    // ── bare worker name → <worker>/index alias ─────────────────────────

    #[tokio::test]
    async fn get_accepts_bare_worker_name_as_alias_for_index() {
        // The user-facing requirement: agents reach for the worker name
        // (e.g. `sandbox`) when they want the worker overview. That call
        // must resolve to `<worker>/index.md` and the response must carry
        // the CANONICAL id so the agent learns the real form on the way
        // through.
        let tmp = tempfile::tempdir().unwrap();
        let ns = tmp.path().join("sandbox");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(ns.join("index.md"), "# Sandbox\n\nWorker overview.\n").unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let out = get_skill(
            &cfg,
            SkillGetInput {
                id: "sandbox".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            out.id, "sandbox/index",
            "response must carry the canonical id, not the shorthand the caller sent"
        );
        assert!(out.body.contains("Worker overview."));
    }

    #[tokio::test]
    async fn bare_name_and_explicit_index_return_same_body() {
        let tmp = tempfile::tempdir().unwrap();
        let ns = tmp.path().join("sandbox");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(
            ns.join("index.md"),
            "---\ntitle: Sandbox\ntype: index\n---\n# Sandbox\n\nShared body.\n",
        )
        .unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let bare = get_skill(
            &cfg,
            SkillGetInput {
                id: "sandbox".into(),
            },
        )
        .await
        .unwrap();
        let explicit = get_skill(
            &cfg,
            SkillGetInput {
                id: "sandbox/index".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(bare.id, explicit.id);
        assert_eq!(bare.title, explicit.title);
        assert_eq!(bare.body, explicit.body);
        assert_eq!(bare.kind, explicit.kind);
    }

    #[tokio::test]
    async fn multi_segment_id_does_not_auto_alias_to_slash_index() {
        // Multi-segment ids must match literally. Without this guard a
        // typo like `sandbox/exec` would silently fall back to
        // `sandbox/exec/index`, which is the wrong skill.
        let tmp = tempfile::tempdir().unwrap();
        let ns = tmp.path().join("sandbox");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(ns.join("index.md"), "# Sandbox\n\nOverview.\n").unwrap();
        // Note: we deliberately do NOT create sandbox/exec/index.md.
        let cfg = cfg_with_skills_folder(tmp.path());
        let err = get_skill(
            &cfg,
            SkillGetInput {
                id: "sandbox/exec".into(),
            },
        )
        .await
        .expect_err("multi-segment id must not auto-alias to /index");
        assert!(
            err.contains("Skill not found: sandbox/exec"),
            "expected literal-id miss, got: {err}"
        );
    }

    #[tokio::test]
    async fn bare_id_with_literal_root_skill_wins_over_index_alias() {
        // When both `<root>/sandbox.md` and `<root>/sandbox/index.md`
        // exist, the literal root skill takes precedence over the
        // bare-name → index alias. Documents the precedence rule so a
        // future refactor doesn't silently flip it.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("sandbox.md"), "# Root\n\nRoot body.\n").unwrap();
        let ns = tmp.path().join("sandbox");
        std::fs::create_dir_all(&ns).unwrap();
        std::fs::write(ns.join("index.md"), "# Index\n\nIndex body.\n").unwrap();
        let cfg = cfg_with_skills_folder(tmp.path());
        let out = get_skill(
            &cfg,
            SkillGetInput {
                id: "sandbox".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            out.id, "sandbox",
            "literal root skill must win over the index alias"
        );
        assert!(out.body.contains("Root body."));
    }

    // ── render_index_markdown ───────────────────────────────────────────

    /// Build a `SkillEntry` for renderer tests. The `kind` argument
    /// drives the `type: index` filter — pass `Some("index")` for a
    /// worker overview, anything else (or `None`) to exercise the
    /// "should be filtered out" path.
    fn entry(id: &str, title: &str, kind: Option<&str>, description: &str) -> SkillEntry {
        SkillEntry {
            id: id.into(),
            title: title.into(),
            kind: kind.map(String::from),
            description: description.into(),
            bytes: 0,
            modified_at: String::new(),
        }
    }

    #[test]
    fn render_index_starts_with_h1_and_worker_count() {
        let body = render_index_markdown(&[
            entry(
                "agent-memory/index",
                "agent-memory",
                Some("index"),
                "Memory tier.",
            ),
            entry(
                "iii-directory/index",
                "iii-directory",
                Some("index"),
                "Directory worker.",
            ),
        ]);
        assert!(
            body.starts_with("# Skills index\n\n2 worker(s).\n"),
            "got: {body}"
        );
    }

    #[test]
    fn render_index_empty_input_still_emits_header() {
        let body = render_index_markdown(&[]);
        assert_eq!(body, "# Skills index\n\n0 worker(s).\n");
    }

    #[test]
    fn render_index_filters_to_type_index() {
        let body = render_index_markdown(&[
            entry(
                "agent-memory/index",
                "agent-memory",
                Some("index"),
                "Worker overview.",
            ),
            entry(
                "agent-memory/observe",
                "Observe",
                Some("how-to"),
                "Record an event.",
            ),
            entry("agent-memory/strays", "Stray", None, "Untyped skill."),
        ]);
        assert!(body.contains("## agent-memory"), "missing h2; got: {body}");
        assert!(
            !body.contains("## Observe"),
            "how-to should be filtered out; got: {body}"
        );
        assert!(
            !body.contains("## Stray"),
            "untyped skill should be filtered out; got: {body}"
        );
        // Filtered-out skills must not leak into the read-more pointers either.
        assert!(
            !body.contains("agent-memory/observe.md"),
            "filtered-out how-to leaked a link; got: {body}"
        );
        assert!(body.contains("1 worker(s).\n"), "wrong count; got: {body}");
    }

    #[test]
    fn render_index_emits_h2_per_worker_using_resolved_title() {
        let body = render_index_markdown(&[
            entry(
                "agent-memory/index",
                "agent-memory",
                Some("index"),
                "Memory tier.",
            ),
            entry(
                "iii-directory/index",
                "iii-directory",
                Some("index"),
                "Directory worker.",
            ),
        ]);
        assert_eq!(
            body.matches("\n## ").count(),
            2,
            "expected exactly two `##` headings; got: {body}"
        );
        assert!(body.contains("\n## agent-memory\n"), "got: {body}");
        assert!(body.contains("\n## iii-directory\n"), "got: {body}");
    }

    #[test]
    fn render_index_includes_description_paragraph() {
        let body = render_index_markdown(&[entry(
            "iii-directory/index",
            "iii-directory",
            Some("index"),
            "Engine introspection and filesystem-backed skill reader.",
        )]);
        // Description sits between the `## title` and the read-more line,
        // separated by blank lines on either side.
        assert!(
            body.contains(
                "\n## iii-directory\n\nEngine introspection and filesystem-backed skill reader.\n\nRead "
            ),
            "description not framed correctly; got: {body}"
        );
    }

    #[test]
    fn render_index_emits_dive_deeper_link() {
        let body = render_index_markdown(&[entry(
            "agent-memory/index",
            "agent-memory",
            Some("index"),
            "Memory tier.",
        )]);
        assert!(
            body.contains(
                "Read [`agent-memory/index.md`](agent-memory/index.md) (legacy `iii://agent-memory/index`) for the full worker reference.\n"
            ),
            "missing dive-deeper pointer; got: {body}"
        );
    }

    #[test]
    fn render_index_skips_blank_description() {
        let body = render_index_markdown(&[entry(
            "bare/index",
            "bare",
            Some("index"),
            "", // body has no paragraph
        )]);
        // Title comes immediately before the read-more line — no extra
        // blank paragraph in the middle.
        assert!(
            body.contains("\n## bare\n\nRead [`bare/index.md`](bare/index.md)"),
            "blank-description block should compress; got: {body}"
        );
        // And the rest of the document still has the header.
        assert!(body.contains("1 worker(s).\n"));
    }

    #[test]
    fn render_index_ordering_follows_input_lex_order() {
        // Input is already lex-sorted by `scan_skills`; the renderer
        // emits sections in the same order.
        let body = render_index_markdown(&[
            entry("agent-memory/index", "agent-memory", Some("index"), "a"),
            entry("iii-directory/index", "iii-directory", Some("index"), "b"),
            entry("resend/index", "resend", Some("index"), "c"),
        ]);
        let am = body.find("## agent-memory").expect("am missing");
        let iii = body.find("## iii-directory").expect("iii missing");
        let resend = body.find("## resend").expect("resend missing");
        assert!(
            am < iii && iii < resend,
            "headings out of order; got: {body}"
        );
    }

    #[test]
    fn render_index_emits_both_file_path_and_iii_pointer() {
        let entries = vec![SkillEntry {
            id: "agent-memory/index".into(),
            title: "agent-memory".into(),
            kind: Some("index".into()),
            description: "Memory worker overview.".into(),
            bytes: 10,
            modified_at: String::new(),
        }];
        let body = render_index_markdown(&entries);
        assert!(
            body.contains("[`agent-memory/index.md`](agent-memory/index.md)"),
            "expected file-path pointer, got:\n{body}"
        );
        assert!(
            body.contains("legacy `iii://agent-memory/index`"),
            "expected legacy iii:// pointer for back-compat, got:\n{body}"
        );
    }
}
