//! How-to skill discovery for `directory::function-info`.
//!
//! Scans `<skills_folder>/**/*.md` for files whose YAML frontmatter
//! declares `type: how-to` and links them to one or more iii function
//! ids. Linkage precedence (first match wins):
//!
//!   1. Frontmatter `functions: [...]` array contains the queried id
//!   2. Frontmatter `function_id: "..."` equals the queried id
//!   3. Body contains the literal `iii://fn/<dotted/path>` URI for the
//!      queried id (e.g. `mem::observe` → `iii://fn/mem/observe`)
//!
//! Also surfaces *related* skills (any `type`, not just how-to) that
//! mention the function via either the literal `function_id` or the
//! `iii://fn/<dotted/path>` URI form — see [`find_related_for_function`].
//!
//! Reuses [`crate::fs_source::split_frontmatter`] / [`crate::fs_source::read_body`]
//! and the same `**/*.md` walker so the new scanner inherits the existing
//! id-validation, cap-checking, and CRLF-tolerance behaviour.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::fs_source::split_frontmatter;
use crate::functions::skills::{extract_title, SKILL_BODY_MAX_BYTES};

/// One on-disk how-to skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsHowTo {
    /// Slashed id (relative path under `skills_folder`, `.md` stripped).
    pub skill_id: String,
    pub abs_path: PathBuf,
    pub frontmatter: HowToFrontmatter,
    pub body: String,
}

/// Subset of frontmatter fields the scanner cares about. Anything else
/// in the YAML block is preserved verbatim by `split_frontmatter` but
/// ignored here.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct HowToFrontmatter {
    /// Required marker — only `type: how-to` files are considered.
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    /// Optional list of function ids this how-to covers.
    #[serde(default)]
    pub functions: Vec<String>,
    /// Optional single function id (alternative to `functions`).
    #[serde(default)]
    pub function_id: Option<String>,
    /// Optional human-readable title (mirrors what an `# H1` would give).
    #[serde(default)]
    pub title: Option<String>,
}

impl HowToFrontmatter {
    pub fn is_how_to(&self) -> bool {
        self.kind.as_deref() == Some("how-to")
    }

    /// True when this how-to declares the supplied id via either
    /// `functions:[...]` or `function_id:`. Body-grep matches are
    /// resolved separately in [`find_for_function`].
    pub fn declares_function(&self, function_id: &str) -> bool {
        if self.functions.iter().any(|f| f == function_id) {
            return true;
        }
        if self.function_id.as_deref() == Some(function_id) {
            return true;
        }
        false
    }
}

/// Walk `skills_folder` and return every `.md` file whose frontmatter
/// has `type: how-to`. Files without frontmatter, with invalid YAML,
/// without the `type: how-to` marker, or that exceed the
/// [`SKILL_BODY_MAX_BYTES`] cap are silently skipped — the scanner is
/// best-effort and does not surface diagnostics (directory reads must
/// stay fast).
pub fn scan_how_tos(skills_folder: &Path) -> Vec<FsHowTo> {
    if !skills_folder.exists() {
        return Vec::new();
    }
    let pattern = match skills_folder.join("**/*.md").to_str() {
        Some(s) => s.to_string(),
        None => return Vec::new(),
    };
    let entries = match glob::glob(&pattern) {
        Ok(it) => it,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    for entry in entries {
        let abs = match entry {
            Ok(p) if p.is_file() => p,
            _ => continue,
        };
        let rel = match abs.strip_prefix(skills_folder) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };
        let raw = match std::fs::read_to_string(&abs) {
            Ok(s) if s.len() <= SKILL_BODY_MAX_BYTES => s,
            _ => continue,
        };
        let (fm_text, body) = split_frontmatter(&raw);
        let Some(fm_text) = fm_text else {
            continue;
        };
        let fm: HowToFrontmatter = match serde_yaml::from_str(fm_text) {
            Ok(f) => f,
            Err(_) => continue,
        };
        if !fm.is_how_to() {
            continue;
        }
        let skill_id = match rel_to_id(&rel) {
            Some(id) => id,
            None => continue,
        };
        out.push(FsHowTo {
            skill_id,
            abs_path: abs,
            frontmatter: fm,
            body: body.trim_matches('\n').to_string(),
        });
    }
    out.sort_by(|a, b| a.skill_id.cmp(&b.skill_id));
    out
}

/// Find the first how-to that documents `function_id`. Precedence:
/// frontmatter-declared (`functions:` / `function_id:`) wins over
/// body-grep, and within each tier the lex-first `skill_id` wins (the
/// scan returns entries already sorted by id).
pub fn find_for_function(skills_folder: &Path, function_id: &str) -> Option<FsHowTo> {
    let how_tos = scan_how_tos(skills_folder);
    if let Some(found) = how_tos
        .iter()
        .find(|h| h.frontmatter.declares_function(function_id))
    {
        return Some(found.clone());
    }
    let needle = function_id_to_uri(function_id);
    how_tos.iter().find(|h| h.body.contains(&needle)).cloned()
}

/// `mem::observe` → `iii://fn/mem/observe`. Mirrors the section-URI
/// shape served by `skills::fetch-skill` (`iii://fn/...`) so the
/// scanner matches the links agents would actually paste.
pub fn function_id_to_uri(function_id: &str) -> String {
    format!("iii://fn/{}", function_id.replace("::", "/"))
}

/// Title-only reference to another skill that mentions a function.
/// Bodies are intentionally omitted; callers fetch on demand via
/// `skills::fetch-skill iii://<skill_id>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RelatedSkillRef {
    pub title: String,
    pub skill_id: String,
}

/// Resolve a display title from (in order): an explicit frontmatter
/// `title`, the first `# H1` line in the body, or the `skill_id` as a
/// final fallback. Empty inputs are skipped.
pub fn resolve_title(frontmatter_title: Option<&str>, body: &str, skill_id: &str) -> String {
    if let Some(t) = frontmatter_title {
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
    skill_id.to_string()
}

/// Frontmatter slice used by [`find_related_for_function`] to harvest a
/// `title` and check `function_id` / `functions` declarations on skills
/// of any `type` (the related scan is more permissive than `scan_how_tos`).
#[derive(Debug, Default, Deserialize)]
struct AnyFrontmatter {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    function_id: Option<String>,
    #[serde(default)]
    functions: Vec<String>,
}

/// Walk every `*.md` under `skills_folder` (any frontmatter `type`,
/// including no frontmatter at all) and return the ones that mention
/// `function_id` via any of:
///
///   * frontmatter `function_id` equals the queried id, or
///   * frontmatter `functions: [...]` contains it, or
///   * body contains the URI form `iii://fn/<dotted/path>`, or
///   * body contains the literal `function_id` substring.
///
/// `exclude_skill_id`, when set, drops the chosen `how_guide` from the
/// result so the same skill doesn't appear in both the primary
/// how-guide slot and the related list. Output is sorted lex by
/// `skill_id` and deduped.
pub fn find_related_for_function(
    skills_folder: &Path,
    function_id: &str,
    exclude_skill_id: Option<&str>,
) -> Vec<RelatedSkillRef> {
    if !skills_folder.exists() {
        return Vec::new();
    }
    let pattern = match skills_folder.join("**/*.md").to_str() {
        Some(s) => s.to_string(),
        None => return Vec::new(),
    };
    let entries = match glob::glob(&pattern) {
        Ok(it) => it,
        Err(_) => return Vec::new(),
    };

    let uri = function_id_to_uri(function_id);
    let mut out: Vec<RelatedSkillRef> = Vec::new();
    for entry in entries {
        let abs = match entry {
            Ok(p) if p.is_file() => p,
            _ => continue,
        };
        let rel = match abs.strip_prefix(skills_folder) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };
        let raw = match std::fs::read_to_string(&abs) {
            Ok(s) if s.len() <= SKILL_BODY_MAX_BYTES => s,
            _ => continue,
        };
        let (fm_text, body) = split_frontmatter(&raw);
        let fm: AnyFrontmatter = fm_text
            .and_then(|t| serde_yaml::from_str(t).ok())
            .unwrap_or_default();

        let frontmatter_match = fm.function_id.as_deref() == Some(function_id)
            || fm.functions.iter().any(|f| f == function_id);
        let body_match = body.contains(&uri) || body.contains(function_id);
        if !frontmatter_match && !body_match {
            continue;
        }

        let skill_id = match rel_to_id(&rel) {
            Some(id) => id,
            None => continue,
        };
        if exclude_skill_id == Some(skill_id.as_str()) {
            continue;
        }
        if out.iter().any(|r| r.skill_id == skill_id) {
            continue;
        }
        let title = resolve_title(fm.title.as_deref(), body, &skill_id);
        out.push(RelatedSkillRef { title, skill_id });
    }
    out.sort_by(|a, b| a.skill_id.cmp(&b.skill_id));
    out
}

fn rel_to_id(rel: &Path) -> Option<String> {
    let s = rel.to_str()?;
    let stripped = s.strip_suffix(".md").unwrap_or(s);
    Some(stripped.replace('\\', "/"))
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

    #[test]
    fn function_id_to_uri_replaces_double_colons() {
        assert_eq!(function_id_to_uri("mem::observe"), "iii://fn/mem/observe");
        assert_eq!(function_id_to_uri("a::b::c::leaf"), "iii://fn/a/b/c/leaf");
        assert_eq!(function_id_to_uri("flat"), "iii://fn/flat");
    }

    #[test]
    fn declares_function_matches_array_or_single() {
        let mut fm = HowToFrontmatter::default();
        assert!(!fm.declares_function("mem::observe"));
        fm.functions.push("mem::observe".into());
        assert!(fm.declares_function("mem::observe"));
        fm.functions.clear();
        fm.function_id = Some("mem::observe".into());
        assert!(fm.declares_function("mem::observe"));
        assert!(!fm.declares_function("mem::recall"));
    }

    #[test]
    fn scan_picks_up_how_to_with_array() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "mem/how-observe.md",
            "---\ntype: how-to\nfunctions: [\"mem::observe\", \"mem::recall\"]\n---\n# How to observe\n\nDo X.\n",
        );
        let how_tos = scan_how_tos(tmp.path());
        assert_eq!(how_tos.len(), 1);
        assert_eq!(how_tos[0].skill_id, "mem/how-observe");
        assert_eq!(
            how_tos[0].frontmatter.functions,
            vec!["mem::observe".to_string(), "mem::recall".to_string()]
        );
        assert!(how_tos[0].body.contains("Do X."));
    }

    #[test]
    fn scan_skips_non_how_to_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "notes/x.md",
            "---\ntype: reference\n---\n# x\nbody\n",
        );
        write_fixture(
            tmp.path(),
            "notes/y.md",
            "# plain markdown\nno frontmatter\n",
        );
        assert!(scan_how_tos(tmp.path()).is_empty());
    }

    #[test]
    fn find_prefers_frontmatter_over_body_grep() {
        let tmp = tempfile::tempdir().unwrap();
        // a — body grep match only
        write_fixture(
            tmp.path(),
            "a.md",
            "---\ntype: how-to\n---\nSee iii://fn/mem/observe for details.\n",
        );
        // b — frontmatter declared (should win)
        write_fixture(
            tmp.path(),
            "b.md",
            "---\ntype: how-to\nfunction_id: mem::observe\n---\nThe canonical guide.\n",
        );
        let found = find_for_function(tmp.path(), "mem::observe").unwrap();
        assert_eq!(found.skill_id, "b");
    }

    #[test]
    fn find_falls_back_to_body_grep() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "guide.md",
            "---\ntype: how-to\n---\nFollow the steps at iii://fn/scope/echo and you're done.\n",
        );
        let found = find_for_function(tmp.path(), "scope::echo").unwrap();
        assert_eq!(found.skill_id, "guide");
    }

    #[test]
    fn find_returns_none_when_nothing_matches() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "x.md",
            "---\ntype: how-to\nfunctions: [\"foo::bar\"]\n---\nbody\n",
        );
        assert!(find_for_function(tmp.path(), "missing::fn").is_none());
    }

    #[test]
    fn scan_skips_oversized_files() {
        let tmp = tempfile::tempdir().unwrap();
        let big_body = "x".repeat(SKILL_BODY_MAX_BYTES + 10);
        write_fixture(
            tmp.path(),
            "big.md",
            &format!("---\ntype: how-to\n---\n{big_body}\n"),
        );
        assert!(scan_how_tos(tmp.path()).is_empty());
    }

    #[test]
    fn scan_handles_missing_dir() {
        assert!(scan_how_tos(Path::new("/no/such/dir")).is_empty());
    }

    // ── resolve_title ────────────────────────────────────────────────

    #[test]
    fn resolve_title_prefers_frontmatter() {
        let title = resolve_title(Some("Frontmatter title"), "# Body H1\n\nbody", "skills/foo");
        assert_eq!(title, "Frontmatter title");
    }

    #[test]
    fn resolve_title_trims_frontmatter_whitespace() {
        let title = resolve_title(Some("   spaced   "), "# H1", "id");
        assert_eq!(title, "spaced");
    }

    #[test]
    fn resolve_title_falls_back_to_h1_when_frontmatter_missing() {
        let title = resolve_title(None, "# Body H1\n\nbody", "skills/foo");
        assert_eq!(title, "Body H1");
    }

    #[test]
    fn resolve_title_falls_back_to_h1_when_frontmatter_empty() {
        let title = resolve_title(Some("   "), "# Body H1", "skills/foo");
        assert_eq!(title, "Body H1");
    }

    #[test]
    fn resolve_title_falls_back_to_skill_id_when_no_h1() {
        let title = resolve_title(None, "no heading here", "skills/foo");
        assert_eq!(title, "skills/foo");
    }

    // ── find_related_for_function ────────────────────────────────────

    #[test]
    fn related_picks_frontmatter_function_id() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "guides/frontmatter.md",
            "---\ntype: how-to\nfunction_id: mem::observe\ntitle: How to observe\n---\n# How to observe\n\nbody\n",
        );
        let related = find_related_for_function(tmp.path(), "mem::observe", None);
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].skill_id, "guides/frontmatter");
        assert_eq!(related[0].title, "How to observe");
    }

    #[test]
    fn related_picks_frontmatter_functions_array() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "guides/array.md",
            "---\ntype: how-to\nfunctions: [\"mem::observe\", \"mem::recall\"]\n---\n# Memory tour\n\nbody\n",
        );
        let related = find_related_for_function(tmp.path(), "mem::observe", None);
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].skill_id, "guides/array");
        assert_eq!(related[0].title, "Memory tour");
    }

    #[test]
    fn related_picks_uri_form_in_body() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "tour.md",
            "# Memory tour\n\nSee iii://fn/mem/observe for details.\n",
        );
        let related = find_related_for_function(tmp.path(), "mem::observe", None);
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].skill_id, "tour");
    }

    #[test]
    fn related_picks_literal_id_in_body() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "notes.md",
            "# Notes\n\nCheck mem::observe before recall.\n",
        );
        let related = find_related_for_function(tmp.path(), "mem::observe", None);
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].skill_id, "notes");
    }

    #[test]
    fn related_excludes_chosen_how_guide() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "primary.md",
            "---\ntype: how-to\nfunction_id: mem::observe\n---\n# Primary\n\nbody\n",
        );
        write_fixture(
            tmp.path(),
            "secondary.md",
            "# Other\n\nLink: iii://fn/mem/observe\n",
        );
        let related = find_related_for_function(tmp.path(), "mem::observe", Some("primary"));
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].skill_id, "secondary");
    }

    #[test]
    fn related_returns_empty_for_unrelated_function_id() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "x.md", "# x\n\nbody mentions other::fn only\n");
        let related = find_related_for_function(tmp.path(), "missing::fn", None);
        assert!(related.is_empty());
    }

    #[test]
    fn related_returns_lex_sorted_unique_results() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "b.md", "# b\n\niii://fn/mem/observe\n");
        write_fixture(tmp.path(), "a.md", "# a\n\nmem::observe\n");
        write_fixture(
            tmp.path(),
            "c.md",
            "---\ntype: how-to\nfunction_id: mem::observe\n---\n# c\n",
        );
        let related = find_related_for_function(tmp.path(), "mem::observe", None);
        let ids: Vec<_> = related.iter().map(|r| r.skill_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn related_handles_missing_dir() {
        let related = find_related_for_function(Path::new("/no/such/dir"), "x::y", None);
        assert!(related.is_empty());
    }
}
