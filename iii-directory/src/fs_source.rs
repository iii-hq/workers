//! Filesystem-backed sources for skills and prompts.
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
//! - [`split_frontmatter`]            — minimal `---\n...\n---\n` parser.
//! - [`scan_skills`]                  — id-keyed listing of all `**/*.md` outside `*/prompts/*`.
//! - [`scan_prompts`]                 — name-keyed listing of `*/prompts/*.md`.
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

#[derive(Debug, Default, Clone, Deserialize)]
pub struct SkillFrontmatter {
    /// Optional human-readable title. When non-empty (after trim) the
    /// reader returns this verbatim instead of the first body `# H1`.
    #[serde(default)]
    pub title: Option<String>,
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
/// previously did. The cap is checked against the raw file size so a
/// file with large frontmatter can't pass one path and fail the other.
/// Empty-after-strip bodies are an error so the resolver returns a
/// clear "not found" rather than serving an empty resource.
pub fn read_body(abs_path: &Path) -> Result<String, String> {
    let (_, body) = read_skill_with_frontmatter(abs_path)?;
    Ok(body)
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
fn top_level_namespaces(root: &Path) -> Vec<String> {
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

/// Merged scan of prompts from a global root and a local root.
///
/// Same whole-namespace override semantics as [`scan_skills_merged`].
pub fn scan_prompts_merged(
    global_root: &Path,
    local_root: &Path,
) -> (Vec<FsPrompt>, Vec<SkipReason>) {
    let local_ns = top_level_namespaces(local_root);

    let (global_prompts, mut global_skipped) = scan_prompts(global_root);
    let global_filtered: Vec<FsPrompt> = global_prompts
        .into_iter()
        .filter(|p| {
            // Prompt paths are under <ns>/prompts/<name>.md; the namespace
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

    let (local_prompts, local_skipped) = scan_prompts(local_root);

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
}
