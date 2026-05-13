//! Filesystem-backed skills reader.
//!
//! Public API (reachable by any worker over `iii.trigger`):
//!
//!   * `directory::skills::list` — enriched listing of every markdown
//!     skill under `skills_folder`, sorted by id. Each row carries
//!     `id`, `title`, `description`, `bytes`, and `modified_at` so a
//!     consumer can render a picker / index in one round trip without
//!     follow-up `get` calls per row.
//!   * `directory::skills::get`  — fetch one skill by id. Returns
//!     `{ id, title, description, body, modified_at }` — the same flat
//!     shape `directory::prompts::get` returns for prompts so the two
//!     read APIs stay symmetric.
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
use crate::fs_source::{self, FsSkill};

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
     title, description, and modified_at — same flat shape as directory::prompts::get. \
     Accepts a bare id (e.g. \"directory/skills/list\") or the same id prefixed with iii://.";

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct ListSkillsInput {}

#[derive(Debug, Serialize, JsonSchema)]
struct SkillEntry {
    id: String,
    /// First `# H1` line in the body, falling back to `id` when absent.
    title: String,
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
pub struct SkillGetInput {
    /// Skill id (the same string returned by `directory::skills::list`,
    /// e.g. `"directory/skills/list"`). The legacy `iii://{id}` form is
    /// also accepted for ergonomics; the prefix is stripped before
    /// validation. Other URI schemes are rejected.
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SkillGetOutput {
    pub id: String,
    pub title: String,
    pub description: String,
    /// Raw markdown body (post-frontmatter) from disk.
    pub body: String,
    /// File mtime as RFC 3339.
    pub modified_at: String,
}

pub fn register(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    register_list_skills(iii, cfg);
    register_get_skill(iii, cfg);
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
            "List filesystem-backed skills (id, title, description, bytes, modified_at) from \
             skills_folder. Each row carries the H1 title and first-paragraph description so \
             consumers can render a picker or indented index without one get per row.",
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

// ---------- core handler ----------

pub async fn get_skill(cfg: &SkillsConfig, req: SkillGetInput) -> Result<SkillGetOutput, String> {
    let id = normalize_get_id(&req.id)?;
    validate_id(&id)?;
    let Some(fs) = find_fs_skill(cfg, &id) else {
        return Err(format!("Skill not found: {id}"));
    };
    let body = fs_source::read_body(&fs.abs_path)?;
    let title = extract_title(&body)
        .map(str::to_string)
        .unwrap_or_else(|| fs.id.clone());
    let description = extract_description(&body).unwrap_or_default();
    let (_, modified_at) = fs_metadata(&fs);
    Ok(SkillGetOutput {
        id: fs.id,
        title,
        description,
        body,
        modified_at,
    })
}

/// Trim, strip an optional `iii://` prefix, and reject any other URI
/// scheme. The remaining string still has to satisfy [`validate_id`];
/// this function only handles the prefix-stripping ergonomics.
pub fn normalize_get_id(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("id must be non-empty".into());
    }
    if let Some(rest) = trimmed.strip_prefix(URI_PREFIX) {
        return Ok(rest.to_string());
    }
    if trimmed.contains("://") {
        return Err(format!(
            "Invalid id (must be a bare skill path or an iii:// URI): {trimmed}"
        ));
    }
    Ok(trimmed.to_string())
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

// ---------- fs lookup ----------

/// Targeted lookup for the read path. Returns `None` if no file under
/// `skills_folder` matches `id`.
fn find_fs_skill(cfg: &SkillsConfig, id: &str) -> Option<FsSkill> {
    let (fs, _skipped) = fs_source::scan_skills(&cfg.resolved_skills_folder());
    fs.into_iter().find(|s| s.id == id)
}

/// Build a `SkillEntry` for `list` output. Reads the file body so the
/// row carries title + description; on read failure the row still
/// surfaces the id with empty title/description so a single broken
/// file doesn't hide every other skill from the picker.
fn skill_entry_from_fs(fs: FsSkill) -> SkillEntry {
    let (bytes, modified_at) = fs_metadata(&fs);
    let (title, description) = match fs_source::read_body(&fs.abs_path) {
        Ok(body) => {
            let title = extract_title(&body)
                .map(str::to_string)
                .unwrap_or_else(|| fs.id.clone());
            let description = extract_description(&body).unwrap_or_default();
            (title, description)
        }
        Err(_) => (fs.id.clone(), String::new()),
    };
    SkillEntry {
        id: fs.id,
        title,
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
        assert_eq!(entry.description, "First paragraph.");
        assert!(entry.bytes > 0);
        assert!(!entry.modified_at.is_empty());
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
        assert_eq!(entry.description, "");
        assert_eq!(entry.bytes, 0);
    }
}
