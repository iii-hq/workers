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
//! Reuses [`crate::fs_source::split_frontmatter`] / [`crate::fs_source::read_body`]
//! and the same `**/*.md` walker so the new scanner inherits the existing
//! id-validation, cap-checking, and CRLF-tolerance behaviour.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::fs_source::split_frontmatter;
use crate::functions::skills::SKILL_BODY_MAX_BYTES;

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
/// shape served by `skill::fetch` (`iii://fn/...`) so the scanner
/// matches the links agents would actually paste.
pub fn function_id_to_uri(function_id: &str) -> String {
    format!("iii://fn/{}", function_id.replace("::", "/"))
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
        assert_eq!(
            function_id_to_uri("a::b::c::leaf"),
            "iii://fn/a/b/c/leaf"
        );
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
        write_fixture(tmp.path(), "notes/y.md", "# plain markdown\nno frontmatter\n");
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
}
