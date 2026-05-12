//! Filesystem-backed sources for skills and prompts.
//!
//! Everything is anchored at the configured `skills_folder` from
//! [`crate::config::SkillsConfig`]. Layout:
//!
//! ```text
//! <skills_folder>/
//!   <ns>/                       # one folder per download namespace
//!     index.md                  # → iii://<ns>/index
//!     anything.md               # → iii://<ns>/anything
//!     deep/path.md              # → iii://<ns>/deep/path
//!     prompts/                  # ← magic marker for prompts
//!       send-email.md           # ← MCP prompt (needs YAML frontmatter)
//!       triage.md
//! ```
//!
//! Files matched as skills become entries whose body is re-read from
//! disk on every resolve. The file system is the single source of
//! truth — nothing is ever cached or mirrored to iii-state.
//!
//! Public surface:
//!
//! - [`split_frontmatter`]   — minimal `---\n...\n---\n` parser.
//! - [`scan_skills`]         — id-keyed listing of all `**/*.md` outside `*/prompts/*`.
//! - [`scan_prompts`]        — name-keyed listing of `*/prompts/*.md`.
//! - [`read_body`]           — cap-checked body read with frontmatter stripped.

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

/// One filesystem-backed prompt entry. `description` is parsed from
/// frontmatter at scan time so [`crate::functions::prompts::mcp_list`]
/// can render the slash-command picker without re-reading every file.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Skill,
    Prompt,
}

#[derive(Debug, Default, Deserialize)]
struct PromptFrontmatter {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
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

/// True if any path component is exactly `prompts`. Used to bisect the
/// `<skills_folder>/**/*.md` walk into the skill side and the prompt
/// side.
fn has_prompts_segment(rel: &Path) -> bool {
    rel.components().any(|c| c.as_os_str() == "prompts")
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

fn rel_to_id(rel: &Path) -> Result<String, String> {
    let rel_str = rel
        .to_str()
        .ok_or_else(|| format!("non-UTF-8 path: {}", rel.display()))?;
    let stripped = rel_str.strip_suffix(".md").unwrap_or(rel_str);
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
        if has_prompts_segment(&rel) {
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

/// Scan every `*.md` under `<skills_folder>/<ns>/prompts/`. Each match
/// must have YAML frontmatter declaring at least `description`; `name`
/// is optional and overrides the file-basename-derived default.
///
/// Rejection reasons mirror [`scan_skills`]: missing frontmatter,
/// invalid YAML, missing `description`, invalid prompt name, or a name
/// collision with another prompt.
pub fn scan_prompts(skills_folder: &Path) -> (Vec<FsPrompt>, Vec<SkipReason>) {
    let mut prompts: Vec<FsPrompt> = Vec::new();
    let mut skipped: Vec<SkipReason> = Vec::new();

    let entries = match walk_markdown(skills_folder) {
        Ok(v) => v,
        Err(e) => {
            skipped.push(SkipReason {
                kind: SourceKind::Prompt,
                path: skills_folder.to_path_buf(),
                reason: e,
            });
            return (prompts, skipped);
        }
    };

    for (abs, rel) in entries {
        if !has_prompts_segment(&rel) {
            continue;
        }
        let content = match std::fs::read_to_string(&abs) {
            Ok(c) => c,
            Err(e) => {
                skipped.push(SkipReason {
                    kind: SourceKind::Prompt,
                    path: abs,
                    reason: format!("read: {e}"),
                });
                continue;
            }
        };
        let (fm_text, _) = split_frontmatter(&content);
        let Some(fm_text) = fm_text else {
            skipped.push(SkipReason {
                kind: SourceKind::Prompt,
                path: abs,
                reason: "missing YAML frontmatter (expected --- ... --- block at file start)"
                    .into(),
            });
            continue;
        };
        let fm: PromptFrontmatter = match serde_yaml::from_str(fm_text) {
            Ok(f) => f,
            Err(e) => {
                skipped.push(SkipReason {
                    kind: SourceKind::Prompt,
                    path: abs,
                    reason: format!("invalid frontmatter YAML: {e}"),
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
                kind: SourceKind::Prompt,
                path: abs,
                reason: format!("invalid prompt name {name:?}: {e}"),
            });
            continue;
        }

        let description = match fm.description {
            Some(d) if !d.trim().is_empty() => d.trim().to_string(),
            _ => {
                skipped.push(SkipReason {
                    kind: SourceKind::Prompt,
                    path: abs,
                    reason: "frontmatter missing non-empty `description`".into(),
                });
                continue;
            }
        };

        if let Some(existing) = prompts.iter().find(|p| p.name == name) {
            if existing.abs_path != abs {
                skipped.push(SkipReason {
                    kind: SourceKind::Prompt,
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

/// Read a fs entry's body fresh from disk, strip any leading
/// frontmatter, and enforce the same 256 KiB cap as the registry
/// previously did. The cap is checked against the raw file size
/// (matching `crate::how_to::scan_how_tos`) so a file with large
/// frontmatter can't pass one path and fail the other.
/// Empty-after-strip bodies are an error so the resolver returns a
/// clear "not found" rather than serving an empty resource.
pub fn read_body(abs_path: &Path) -> Result<String, String> {
    let raw = std::fs::read_to_string(abs_path)
        .map_err(|e| format!("read {}: {e}", abs_path.display()))?;
    if raw.len() > SKILL_BODY_MAX_BYTES {
        return Err(format!(
            "file {} is too large ({} bytes; max {SKILL_BODY_MAX_BYTES})",
            abs_path.display(),
            raw.len()
        ));
    }
    let (_, body) = split_frontmatter(&raw);
    let trimmed = body.trim_matches('\n');
    if trimmed.is_empty() {
        return Err(format!("file {} has empty body", abs_path.display()));
    }
    Ok(body.to_string())
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

    // ── scan_prompts ─────────────────────────────────────────────────

    #[test]
    fn scan_prompts_reads_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns/prompts/open-pr.md",
            "---\nname: open-pr\ndescription: Open a PR.\n---\nBody here.\n",
        );

        let (prompts, skipped) = scan_prompts(tmp.path());
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].name, "open-pr");
        assert_eq!(prompts[0].description, "Open a PR.");
    }

    #[test]
    fn scan_prompts_falls_back_to_filename_for_name() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns/prompts/foo.md",
            "---\ndescription: Just a description.\n---\nBody.\n",
        );

        let (prompts, _skipped) = scan_prompts(tmp.path());
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].name, "foo");
    }

    #[test]
    fn scan_prompts_rejects_missing_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "ns/prompts/no-fm.md", "# heading\nbody\n");

        let (prompts, skipped) = scan_prompts(tmp.path());
        assert!(prompts.is_empty());
        assert_eq!(skipped.len(), 1);
        assert!(
            skipped[0].reason.contains("missing YAML frontmatter"),
            "got: {:?}",
            skipped[0].reason
        );
    }

    #[test]
    fn scan_prompts_rejects_missing_description() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns/prompts/no-desc.md",
            "---\nname: foo\n---\nBody.\n",
        );
        let (prompts, skipped) = scan_prompts(tmp.path());
        assert!(prompts.is_empty());
        assert_eq!(skipped.len(), 1);
        assert!(
            skipped[0].reason.contains("description"),
            "got: {:?}",
            skipped[0].reason
        );
    }

    #[test]
    fn scan_prompts_rejects_invalid_name() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns/prompts/bad.md",
            "---\nname: Has Spaces\ndescription: x\n---\nBody.\n",
        );
        let (prompts, skipped) = scan_prompts(tmp.path());
        assert!(prompts.is_empty());
        assert_eq!(skipped.len(), 1);
        assert!(
            skipped[0].reason.contains("invalid prompt name"),
            "got: {:?}",
            skipped[0].reason
        );
    }

    #[test]
    fn scan_prompts_collision_skips_second() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "ns-a/prompts/shared.md",
            "---\nname: shared\ndescription: from a\n---\nBody A.\n",
        );
        write_fixture(
            tmp.path(),
            "ns-b/prompts/shared.md",
            "---\nname: shared\ndescription: from b\n---\nBody B.\n",
        );
        let (prompts, skipped) = scan_prompts(tmp.path());
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
}
