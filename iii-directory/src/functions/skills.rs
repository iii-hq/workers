//! Filesystem-backed skills reader.
//!
//! Public API (reachable by any worker over `iii.trigger`):
//!
//!   * `directory::skills::list`         — metadata-only listing of every
//!     markdown skill under `skills_folder`, sorted by id.
//!   * `directory::skills::fetch-skill`  — batched read over one or more
//!     `iii://` URIs (or bare skill paths). Returns plain markdown
//!     joined with `\n\n---\n\n`.
//!
//! The URI resolution pipeline (`iii://directory/skills` index,
//! `iii://{id}` filesystem reads, `iii://fn/{path}` function triggers)
//! lives here and is invoked internally by `fetch_skill`. There are no
//! longer any MCP-shaped wrappers around it — this worker is
//! intentionally agnostic to MCP and any other adapter; agnostic-shape
//! readers are the rule.
//!
//! There are no write paths in this module. Files arrive on disk via
//! `directory::skills::download` (see [`crate::functions::download`])
//! or by direct editing under `skills_folder`. Mutations fan out
//! through the `directory::skills::on-change` trigger type which is
//! fired from the download function on success.

use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, TriggerRequest, III};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::{json, Value};

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

/// Reserved as the first path segment of any URI. `iii://fn/...` is the
/// section-URI marker; `iii://anything-else/...` is a filesystem-backed
/// skill body lookup. The reservation only applies to the first segment
/// — `iii://docs/fn-reference` (the literal `fn` deeper in the path) is
/// a perfectly valid skill id.
const FN_PREFIX: &str = "fn";
const URI_PREFIX: &str = "iii://";

/// The id segment(s) after `iii://` that map to the auto-rendered
/// skills index. The literal `directory/skills` URI is reserved for
/// the index render so it never collides with a real skill body. Kept
/// as a constant so [`parse_uri`] and [`render_index`] agree without a
/// string-match drift risk.
const INDEX_ID: &str = "directory/skills";

/// Description for the `directory::skills::fetch-skill` registration.
const FETCH_DESCRIPTION: &str = "Fetches the content of one or more skill resources. Each entry may be either a full \
     iii:// URI or a bare skill path (the id returned by directory::skills::list, e.g. \
     \"directory/skills/list\") which is auto-prefixed with iii://. Batch with `uris` when helpful.";

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct ListSkillsInput {}

#[derive(Debug, Serialize, JsonSchema)]
struct SkillEntry {
    id: String,
    bytes: usize,
    /// File mtime as RFC 3339 (best effort; empty if unavailable).
    modified_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ListSkillsOutput {
    skills: Vec<SkillEntry>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct FetchSkillInput {
    /// A single skill resource to read. Either a full `iii://` URI or
    /// a bare skill path (the id returned by `directory::skills::list`,
    /// e.g. `"directory/skills/list"`) which is auto-prefixed with
    /// `iii://`.
    #[serde(default)]
    pub uri: Option<String>,
    /// One or more skill resources to read in order. Same shape rules
    /// as `uri`. When both `uri` and `uris` are provided, `uris` wins
    /// (matches the TS reference implementation).
    #[serde(default)]
    pub uris: Option<Vec<String>>,
}

pub fn register(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    register_list_skills(iii, cfg);
    register_fetch_skill(iii, cfg);
}

fn register_list_skills(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async("directory::skills::list", move |_input: ListSkillsInput| {
            let _iii = iii_inner.clone();
            let cfg = cfg_inner.clone();
            async move {
                let (entries, _skipped) = fs_source::scan_skills(&cfg.resolved_skills_folder());
                let out: Vec<SkillEntry> = entries
                    .into_iter()
                    .map(|fs| {
                        let (bytes, modified_at) = fs_metadata(&fs);
                        SkillEntry {
                            id: fs.id,
                            bytes,
                            modified_at,
                        }
                    })
                    .collect();
                Ok::<_, IIIError>(ListSkillsOutput { skills: out })
            }
        })
        .description(
            "List filesystem-backed skills (id, body length, modified_at) from skills_folder.",
        ),
    );
}

fn register_fetch_skill(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async(
            "directory::skills::fetch-skill",
            move |req: FetchSkillInput| {
                let iii = iii_inner.clone();
                let cfg = cfg_inner.clone();
                async move {
                    fetch_skill(&iii, &cfg, req)
                        .await
                        .map_err(IIIError::Handler)
                }
            },
        )
        .description(FETCH_DESCRIPTION)
        .metadata(json!({"tool": {"label": "Fetch skill"}})),
    );
}

// ---------- internal URI resolution (used by fetch_skill) ----------

/// Resolve a single `iii://...` URI to its body. Returns an envelope
/// shaped `{ contents: [{ uri, mimeType, text }] }` so the per-URI
/// metadata is preserved when [`fetch_skill`] joins the results.
async fn read(iii: &III, cfg: &SkillsConfig, uri: &str) -> Result<Value, String> {
    let parsed = parse_uri(uri)?;
    match parsed {
        ParsedUri::Index => {
            let body = render_index(cfg);
            Ok(wrap_contents(uri, "text/markdown", &body))
        }
        ParsedUri::Skill(id) => {
            // The slashed path is the relative id. Re-validate so a
            // crafted `iii://Foo` URI fails fast even if it slipped
            // past the section-prefix check.
            validate_id(&id)?;
            if let Some(fs) = find_fs_skill(cfg, &id) {
                let body = fs_source::read_body(&fs.abs_path)?;
                return Ok(wrap_contents(uri, "text/markdown", &body));
            }
            Err(format!("Skill not found: {id}"))
        }
        ParsedUri::Section { function_id } => {
            let value = iii
                .trigger(TriggerRequest {
                    function_id: function_id.clone(),
                    payload: json!({}),
                    action: None,
                    timeout_ms: Some(cfg.download_timeout_ms),
                })
                .await
                .map_err(|e| format!("trigger {function_id}: {e}"))?;
            let (text, mime) = normalize_function_output(value);
            Ok(wrap_contents(uri, mime, &text))
        }
    }
}

// ---------- batched fetch (skills::fetch-skill) ----------

/// Pure half of the fetch tool: validates the input shape, normalizes
/// each entry to a trimmed `iii://` URI, and rejects anything outside
/// the `iii://` scheme.
///
/// Two input shapes are accepted per entry:
///   * Full `iii://...` URI — passed through verbatim.
///   * Bare skill path (matching the `id` returned by
///     `directory::skills::list`, e.g. `"directory/skills/list"`) —
///     prefixed with `iii://` automatically.
///
/// Anything else with a `://` (e.g. `https://...`) is rejected.
/// Split out so the validation branches can be unit-tested without an
/// iii engine.
pub fn validate_fetch_input(input: FetchSkillInput) -> Result<Vec<String>, String> {
    // `uris` wins when both are provided — matches the TS reference
    // impl and the handoff doc.
    let raw: Vec<String> = match (input.uris, input.uri) {
        (Some(v), _) if !v.is_empty() => v,
        (_, Some(s)) if !s.trim().is_empty() => vec![s],
        _ => return Err("Provide uri or a non-empty uris array".into()),
    };
    let list: Vec<String> = raw
        .into_iter()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .map(normalize_fetch_entry)
        .collect::<Result<Vec<_>, _>>()?;
    if list.is_empty() {
        return Err("Provide uri or a non-empty uris array".into());
    }
    Ok(list)
}

/// Normalize one fetch entry: pass `iii://` URIs through, prefix bare
/// skill paths with `iii://`, and reject any other URI scheme.
fn normalize_fetch_entry(entry: String) -> Result<String, String> {
    if entry.starts_with(URI_PREFIX) {
        return Ok(entry);
    }
    if entry.contains("://") {
        return Err(format!("Invalid URI (must start with iii://): {entry}"));
    }
    Ok(format!("{URI_PREFIX}{entry}"))
}

/// Resolve every `iii://` URI in `input` through [`read`], wrap each
/// result as `# {uri}\n\n{text}`, and join sections with
/// `\n\n---\n\n`. Returns plain markdown.
pub async fn fetch_skill(
    iii: &III,
    cfg: &SkillsConfig,
    input: FetchSkillInput,
) -> Result<String, String> {
    let list = validate_fetch_input(input)?;
    let mut sections = Vec::with_capacity(list.len());
    for uri in &list {
        let v = read(iii, cfg, uri).await?;
        let text = v["contents"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| c["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        sections.push(format!("# {uri}\n\n{}", text.trim_end()));
    }
    Ok(sections.join("\n\n---\n\n"))
}

// ---------- URI parsing ----------

#[derive(Debug, PartialEq, Eq)]
pub enum ParsedUri {
    /// `iii://directory/skills` — the auto-rendered tree-of-skills
    /// index.
    Index,
    /// Filesystem-backed skill body. The payload is the full slashed
    /// path the body is stored under (1+ segments). The first segment
    /// is never `fn` — that prefix is reserved for [`Section`].
    Skill(String),
    /// Function trigger. The payload is the resolved iii function id
    /// built by joining the URI segments after `fn/` with `::`.
    /// e.g. `iii://fn/scope/echo` → `function_id == "scope::echo"`.
    Section { function_id: String },
}

/// Parse an `iii://...` resource URI into a [`ParsedUri`].
pub fn parse_uri(uri: &str) -> Result<ParsedUri, String> {
    let rest = uri
        .strip_prefix(URI_PREFIX)
        .ok_or_else(|| format!("Resource URI must start with iii://: {uri}"))?;
    if rest.is_empty() {
        return Err(format!("Empty resource id: {uri}"));
    }
    if rest == INDEX_ID {
        return Ok(ParsedUri::Index);
    }

    let segments: Vec<&str> = rest.split('/').collect();
    if segments.iter().any(|s| s.is_empty()) {
        return Err(format!(
            "Resource URI may not contain empty segments (no leading, trailing, or doubled '/'): {uri}"
        ));
    }

    if segments[0] == FN_PREFIX {
        let fn_segments = &segments[1..];
        if fn_segments.is_empty() {
            return Err(format!(
                "Section URI 'iii://fn' is missing a function path: expected iii://fn/{{a}}/{{b}}/...: {uri}"
            ));
        }
        for seg in fn_segments {
            validate_id_segment(seg)
                .map_err(|e| format!("invalid section URI segment {seg:?}: {e}"))?;
        }
        Ok(ParsedUri::Section {
            function_id: fn_segments.join("::"),
        })
    } else {
        Ok(ParsedUri::Skill(rest.to_string()))
    }
}

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

/// Validate a full skill id. Accepts 1+ segments separated by `/`. The
/// first segment must NOT equal [`FN_PREFIX`] (`"fn"`) — that literal
/// is reserved as the section-URI prefix at the top level.
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
    if segments[0] == FN_PREFIX {
        return Err(format!(
            "id may not have {FN_PREFIX:?} as its first segment (reserved as the iii://fn/ section-URI marker): {id:?}"
        ));
    }
    Ok(())
}

// ---------- markdown helpers ----------

fn render_index(cfg: &SkillsConfig) -> String {
    let (skills, _skipped) = fs_source::scan_skills(&cfg.resolved_skills_folder());
    let mut out = String::from(
        "# Skills\n\nRead each skill's resource for orientation on when and why to call its functions. \
         Sub-skills are indented under their parent path so a top-level skill stays small \
         and the LLM can drill in only when it needs more detail.\n\n",
    );

    if skills.is_empty() {
        out.push_str("_No skills are currently available in skills_folder._\n");
        return out;
    }

    // `scan_skills` returns entries sorted lexicographically by id, so a
    // single linear pass yields a correct tree: every nested entry
    // appears immediately after its parent (or its parent's last
    // descendant). Indent each entry by `2 * depth` spaces, where depth
    // is the number of '/' separators in the id.
    for fs in &skills {
        let body = fs_source::read_body(&fs.abs_path).ok();
        let title = body
            .as_deref()
            .and_then(extract_title)
            .map(String::from)
            .unwrap_or_else(|| fs.id.clone());
        let desc = body
            .as_deref()
            .and_then(extract_description)
            .unwrap_or_default();
        push_index_bullet(&mut out, &fs.id, &title, &desc);
    }

    out
}

fn push_index_bullet(out: &mut String, id: &str, title: &str, desc: &str) {
    let depth = id.matches('/').count();
    let indent = " ".repeat(depth * 2);
    let suffix = if desc.is_empty() {
        String::new()
    } else {
        format!(" — {desc}")
    };
    out.push_str(&format!(
        "{indent}- [`{id}`](iii://{id}) — {title}{suffix}\n"
    ));
}

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

pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        Some((byte_end, _)) => format!("{}...", &s[..byte_end]),
        None => s.to_string(),
    }
}

// ---------- output normalization for iii://{skill}/{function} ----------

pub fn normalize_function_output(v: Value) -> (String, &'static str) {
    if let Value::String(s) = &v {
        return (s.clone(), "text/markdown");
    }
    if let Some(content) = v.get("content").and_then(|c| c.as_str()) {
        return (content.to_string(), "text/markdown");
    }
    let pretty = serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string());
    (pretty, "application/json")
}

fn wrap_contents(uri: &str, mime: &str, text: &str) -> Value {
    json!({
        "contents": [
            { "uri": uri, "mimeType": mime, "text": text }
        ]
    })
}

// ---------- fs lookup ----------

/// Targeted lookup for the read path. Returns `None` if no file under
/// `skills_folder` matches `id`.
fn find_fs_skill(cfg: &SkillsConfig, id: &str) -> Option<FsSkill> {
    let (fs, _skipped) = fs_source::scan_skills(&cfg.resolved_skills_folder());
    fs.into_iter().find(|s| s.id == id)
}

/// Cheap metadata for `skills::list`. Bytes is the on-disk file size;
/// `modified_at` is the file's mtime as RFC 3339.
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

    // ── parse_uri: index ────────────────────────────────────────────────

    #[test]
    fn parse_index_uri() {
        assert_eq!(
            parse_uri("iii://directory/skills").unwrap(),
            ParsedUri::Index
        );
    }

    #[test]
    fn parse_skill_uri_disambiguates_from_index() {
        assert_eq!(
            parse_uri("iii://directory/skills/list").unwrap(),
            ParsedUri::Skill("directory/skills/list".into())
        );
    }

    // ── parse_uri: skill bodies ─────────────────────────────────────────

    #[test]
    fn parse_single_skill_uri() {
        assert_eq!(
            parse_uri("iii://brain").unwrap(),
            ParsedUri::Skill("brain".into())
        );
    }

    #[test]
    fn parse_two_segment_skill_uri() {
        assert_eq!(
            parse_uri("iii://parent/sub").unwrap(),
            ParsedUri::Skill("parent/sub".into())
        );
    }

    #[test]
    fn parse_three_segment_skill_uri() {
        assert_eq!(
            parse_uri("iii://a/b/c").unwrap(),
            ParsedUri::Skill("a/b/c".into())
        );
    }

    #[test]
    fn parse_deeply_nested_skill_uri() {
        assert_eq!(
            parse_uri("iii://a/b/c/d/e").unwrap(),
            ParsedUri::Skill("a/b/c/d/e".into())
        );
    }

    #[test]
    fn parse_skill_uri_allows_fn_at_non_first_segment() {
        assert_eq!(
            parse_uri("iii://docs/fn-reference").unwrap(),
            ParsedUri::Skill("docs/fn-reference".into())
        );
        assert_eq!(
            parse_uri("iii://a/fn/c").unwrap(),
            ParsedUri::Skill("a/fn/c".into())
        );
    }

    // ── parse_uri: section URIs (function triggers) ─────────────────────

    #[test]
    fn parse_section_uri_single_segment() {
        assert_eq!(
            parse_uri("iii://fn/foo").unwrap(),
            ParsedUri::Section {
                function_id: "foo".into(),
            }
        );
    }

    #[test]
    fn parse_section_uri_two_segments_join_with_double_colon() {
        assert_eq!(
            parse_uri("iii://fn/scope/echo").unwrap(),
            ParsedUri::Section {
                function_id: "scope::echo".into(),
            }
        );
    }

    #[test]
    fn parse_section_uri_three_segments() {
        assert_eq!(
            parse_uri("iii://fn/resend/email/send").unwrap(),
            ParsedUri::Section {
                function_id: "resend::email::send".into(),
            }
        );
    }

    #[test]
    fn parse_section_uri_arbitrary_depth() {
        assert_eq!(
            parse_uri("iii://fn/a/b/c/d").unwrap(),
            ParsedUri::Section {
                function_id: "a::b::c::d".into(),
            }
        );
    }

    // ── parse_uri: error cases ──────────────────────────────────────────

    #[test]
    fn rejects_missing_prefix() {
        assert!(parse_uri("brain").is_err());
        assert!(parse_uri("https://example.com").is_err());
    }

    #[test]
    fn rejects_empty_body() {
        assert!(parse_uri("iii://").is_err());
    }

    #[test]
    fn rejects_empty_segments() {
        assert!(parse_uri("iii:///fn").is_err());
        assert!(parse_uri("iii://skill/").is_err());
        assert!(parse_uri("iii://a//b").is_err());
        assert!(parse_uri("iii://fn/").is_err());
    }

    #[test]
    fn rejects_section_uri_with_no_function_path() {
        let err = parse_uri("iii://fn").unwrap_err();
        assert!(err.contains("missing a function path"), "got: {err}");
    }

    #[test]
    fn rejects_section_uri_with_invalid_segment() {
        assert!(parse_uri("iii://fn/Bad-Case").is_err());
        assert!(parse_uri("iii://fn/a/b::c").is_err());
        assert!(parse_uri("iii://fn/a b").is_err());
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
    fn id_validation_allows_fn_at_non_first_segment() {
        assert!(validate_id("docs/fn-reference").is_ok());
        assert!(validate_id("a/fn").is_ok());
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
    fn id_validation_rejects_fn_as_first_segment() {
        let err = validate_id("fn").unwrap_err();
        assert!(err.contains("first segment"), "got: {err}");
        assert!(validate_id("fn/anything").is_err());
        assert!(validate_id("fn/a/b").is_err());
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

    #[test]
    fn normalize_string_returns_markdown() {
        let (text, mime) = normalize_function_output(Value::String("hello".into()));
        assert_eq!(text, "hello");
        assert_eq!(mime, "text/markdown");
    }

    #[test]
    fn normalize_content_object_returns_markdown() {
        let (text, mime) = normalize_function_output(json!({ "content": "hi" }));
        assert_eq!(text, "hi");
        assert_eq!(mime, "text/markdown");
    }

    #[test]
    fn normalize_other_falls_back_to_json() {
        let (text, mime) = normalize_function_output(json!({ "x": 1 }));
        assert_eq!(mime, "application/json");
        assert!(text.contains("\"x\""));
    }

    #[test]
    fn truncate_chars_handles_multibyte() {
        let s = "áéíóú".repeat(50);
        let out = truncate_chars(&s, 5);
        assert!(out.starts_with("áéíóú"));
        assert!(out.ends_with("..."));
        assert_eq!(out.chars().count(), 5 + 3);
    }

    // ── fetch input validation ─────────────────────────────────────────

    #[test]
    fn fetch_skill_rejects_no_uri() {
        let err = validate_fetch_input(FetchSkillInput::default()).unwrap_err();
        assert!(err.contains("Provide uri"), "got: {err}");
    }

    #[test]
    fn fetch_skill_rejects_blank_uri() {
        let err = validate_fetch_input(FetchSkillInput {
            uri: Some("   ".into()),
            uris: None,
        })
        .unwrap_err();
        assert!(err.contains("Provide uri"), "got: {err}");
    }

    #[test]
    fn fetch_skill_rejects_empty_uris_array() {
        let err = validate_fetch_input(FetchSkillInput {
            uri: None,
            uris: Some(vec![]),
        })
        .unwrap_err();
        assert!(err.contains("Provide uri"), "got: {err}");
    }

    #[test]
    fn fetch_skill_rejects_non_iii_uri() {
        let err = validate_fetch_input(FetchSkillInput {
            uri: Some("https://example.com".into()),
            uris: None,
        })
        .unwrap_err();
        assert!(err.contains("iii://"), "got: {err}");
    }

    #[test]
    fn fetch_skill_rejects_non_iii_uri_in_array() {
        let err = validate_fetch_input(FetchSkillInput {
            uri: None,
            uris: Some(vec!["iii://ok".into(), "ftp://nope".into()]),
        })
        .unwrap_err();
        assert!(err.contains("iii://"), "got: {err}");
    }

    #[test]
    fn fetch_skill_accepts_bare_skill_path_and_prefixes_it() {
        let list = validate_fetch_input(FetchSkillInput {
            uri: Some("agent-memory/observe".into()),
            uris: None,
        })
        .unwrap();
        assert_eq!(list, vec!["iii://agent-memory/observe".to_string()]);
    }

    #[test]
    fn fetch_skill_accepts_mixed_batch_of_uris_and_bare_paths() {
        let list = validate_fetch_input(FetchSkillInput {
            uri: None,
            uris: Some(vec![
                "iii://full".into(),
                "directory/skills/list".into(),
                "single".into(),
            ]),
        })
        .unwrap();
        assert_eq!(
            list,
            vec![
                "iii://full".to_string(),
                "iii://directory/skills/list".to_string(),
                "iii://single".to_string(),
            ]
        );
    }

    #[test]
    fn fetch_skill_uris_takes_precedence_when_both_provided() {
        let list = validate_fetch_input(FetchSkillInput {
            uri: Some("iii://from-uri".into()),
            uris: Some(vec!["iii://from-uris".into()]),
        })
        .unwrap();
        assert_eq!(list, vec!["iii://from-uris".to_string()]);
    }

    #[test]
    fn fetch_skill_trims_whitespace_around_uris() {
        let list = validate_fetch_input(FetchSkillInput {
            uri: None,
            uris: Some(vec!["  iii://a  ".into(), "iii://b\n".into()]),
        })
        .unwrap();
        assert_eq!(list, vec!["iii://a".to_string(), "iii://b".to_string()]);
    }

    #[test]
    fn fetch_skill_drops_blank_entries_in_uris_array() {
        let list = validate_fetch_input(FetchSkillInput {
            uri: None,
            uris: Some(vec!["iii://a".into(), "   ".into(), "iii://b".into()]),
        })
        .unwrap();
        assert_eq!(list, vec!["iii://a".to_string(), "iii://b".to_string()]);
    }

    #[test]
    fn fetch_skill_single_uri_preserved_after_trim() {
        let list = validate_fetch_input(FetchSkillInput {
            uri: Some("  iii://only  ".into()),
            uris: None,
        })
        .unwrap();
        assert_eq!(list, vec!["iii://only".to_string()]);
    }

    #[test]
    fn index_id_constant_matches_index_uri_suffix() {
        assert_eq!(INDEX_ID, "directory/skills");
    }
}
