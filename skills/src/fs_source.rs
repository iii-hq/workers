//! Filesystem-backed sources for skills and prompts.
//!
//! Globs from [`crate::config::SkillsConfig::skills`] /
//! [`SkillsConfig::prompts`][cfg] are expanded against the config
//! directory; each match becomes a filesystem-backed entry whose body
//! is re-read from disk on every resolve. Nothing here writes to
//! iii-state — the file system is the single source of truth.
//!
//! [cfg]: crate::config::SkillsConfig::prompts
//!
//! Public surface:
//!
//! - [`static_prefix`]            — directory portion of a glob pattern.
//! - [`split_frontmatter`]        — minimal `---\n...\n---\n` parser.
//! - [`expand_skill_globs`]       — id-keyed listing for skills globs.
//! - [`expand_prompt_globs`]      — name-keyed listing for prompts globs.
//! - [`read_body`]                — cap-checked body read with frontmatter stripped.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::functions::prompts::validate_name;
use crate::functions::skills::{validate_id, SKILL_BODY_MAX_BYTES};

/// One filesystem-backed skill entry. The body lives on disk; we cache
/// only the metadata needed to render the index and resolve `iii://`
/// reads.
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
/// boot-time logging so misconfigured globs are easy to spot.
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

/// Return the static directory portion of a glob pattern: everything
/// before the first `*`/`?`/`[`/`{`, truncated back to the last `/`
/// (so the prefix is always either empty or ends in `/`). The glob
/// expander uses this to strip a deterministic prefix when computing
/// ids — `my-folder/**/*.md` matching `my-folder/foo/bar.md` yields
/// the relative path `foo/bar.md`.
pub fn static_prefix(pattern: &str) -> &str {
    let bytes = pattern.as_bytes();
    let first_meta = bytes
        .iter()
        .position(|&b| matches!(b, b'*' | b'?' | b'[' | b'{'));
    let cut = first_meta.unwrap_or(bytes.len());
    let prefix = &pattern[..cut];
    match prefix.rfind('/') {
        Some(i) => &prefix[..=i],
        None => "",
    }
}

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

fn derive_relative_id(prefix_abs: &Path, abs_path: &Path) -> Result<String, String> {
    let rel = abs_path.strip_prefix(prefix_abs).map_err(|_| {
        format!(
            "{} is not under prefix {}",
            abs_path.display(),
            prefix_abs.display()
        )
    })?;
    let rel_str = rel
        .to_str()
        .ok_or_else(|| format!("non-UTF-8 path: {}", abs_path.display()))?;
    let stripped = rel_str.strip_suffix(".md").unwrap_or(rel_str);
    Ok(stripped.replace('\\', "/"))
}

// ───────────────────────── glob expansion ────────────────────────────

fn run_glob(base_dir: &Path, pattern: &str) -> Result<Vec<(String, PathBuf)>, String> {
    let pat_path = Path::new(pattern);
    let abs_pattern_buf = if pat_path.is_absolute() {
        pat_path.to_path_buf()
    } else {
        base_dir.join(pat_path)
    };
    let abs_pattern = abs_pattern_buf
        .to_str()
        .ok_or_else(|| {
            format!(
                "non-UTF-8 path while joining {} + {pattern:?}",
                base_dir.display()
            )
        })?
        .to_string();

    let static_prefix_str = static_prefix(&abs_pattern);
    let prefix_path = if static_prefix_str.is_empty() {
        // `*.md` style — the static prefix is empty, so we anchor ids
        // at base_dir.
        base_dir.to_path_buf()
    } else {
        PathBuf::from(static_prefix_str)
    };

    let entries = glob::glob(&abs_pattern).map_err(|e| format!("invalid glob {pattern:?}: {e}"))?;

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
        match derive_relative_id(&prefix_path, &path) {
            Ok(id) => out.push((id, path)),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "derive id failed; skipping");
            }
        }
    }
    Ok(out)
}

/// Expand every skill glob in `patterns` against `base_dir`. Returns
/// the validated, deduped, lex-sorted list plus a parallel diagnostic
/// list of files that were rejected.
///
/// Rejection reasons:
///
/// - The derived id fails [`validate_id`] (uppercase, spaces, etc.).
/// - Two distinct files produce the same id (intra-fs duplicate).
///
/// Files matched by multiple glob patterns from the same scan deduplicate
/// silently — they're the same file, the same id, no warning.
pub fn expand_skill_globs(base_dir: &Path, patterns: &[String]) -> (Vec<FsSkill>, Vec<SkipReason>) {
    let mut skills: Vec<FsSkill> = Vec::new();
    let mut skipped: Vec<SkipReason> = Vec::new();

    for pattern in patterns {
        let matches = match run_glob(base_dir, pattern) {
            Ok(m) => m,
            Err(e) => {
                skipped.push(SkipReason {
                    kind: SourceKind::Skill,
                    path: PathBuf::from(pattern),
                    reason: e,
                });
                continue;
            }
        };
        for (id, path) in matches {
            if let Err(e) = validate_id(&id) {
                skipped.push(SkipReason {
                    kind: SourceKind::Skill,
                    path,
                    reason: format!("invalid id {id:?}: {e}"),
                });
                continue;
            }
            if let Some(existing) = skills.iter().find(|s| s.id == id) {
                if existing.abs_path != path {
                    skipped.push(SkipReason {
                        kind: SourceKind::Skill,
                        path,
                        reason: format!(
                            "duplicate id {id:?} also produced by {}",
                            existing.abs_path.display()
                        ),
                    });
                }
                continue;
            }
            skills.push(FsSkill { id, abs_path: path });
        }
    }

    skills.sort_by(|a, b| a.id.cmp(&b.id));
    (skills, skipped)
}

/// Expand every prompt glob in `patterns`. Each match must have a
/// leading YAML frontmatter block declaring at least `description`;
/// `name` is optional and overrides the file-basename-derived default.
///
/// Rejection reasons follow the same shape as [`expand_skill_globs`]:
/// missing frontmatter, invalid YAML, missing `description`, invalid
/// `validate_name`, or a name collision with another fs prompt.
pub fn expand_prompt_globs(
    base_dir: &Path,
    patterns: &[String],
) -> (Vec<FsPrompt>, Vec<SkipReason>) {
    let mut prompts: Vec<FsPrompt> = Vec::new();
    let mut skipped: Vec<SkipReason> = Vec::new();

    for pattern in patterns {
        let matches = match run_glob(base_dir, pattern) {
            Ok(m) => m,
            Err(e) => {
                skipped.push(SkipReason {
                    kind: SourceKind::Prompt,
                    path: PathBuf::from(pattern),
                    reason: e,
                });
                continue;
            }
        };
        for (id, path) in matches {
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    skipped.push(SkipReason {
                        kind: SourceKind::Prompt,
                        path,
                        reason: format!("read: {e}"),
                    });
                    continue;
                }
            };
            let (fm_text, _) = split_frontmatter(&content);
            let Some(fm_text) = fm_text else {
                skipped.push(SkipReason {
                    kind: SourceKind::Prompt,
                    path,
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
                        path,
                        reason: format!("invalid frontmatter YAML: {e}"),
                    });
                    continue;
                }
            };

            // Prompt names are flat — fall back to the basename of the
            // derived id so a `prompts/sub/foo.md` becomes `foo` by
            // default. Frontmatter `name:` always wins when present.
            let derived_basename = id.rsplit('/').next().unwrap_or(&id).to_string();
            let name = fm
                .name
                .as_deref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or(derived_basename);

            if let Err(e) = validate_name(&name) {
                skipped.push(SkipReason {
                    kind: SourceKind::Prompt,
                    path,
                    reason: format!("invalid prompt name {name:?}: {e}"),
                });
                continue;
            }

            let description = match fm.description {
                Some(d) if !d.trim().is_empty() => d.trim().to_string(),
                _ => {
                    skipped.push(SkipReason {
                        kind: SourceKind::Prompt,
                        path,
                        reason: "frontmatter missing non-empty `description`".into(),
                    });
                    continue;
                }
            };

            if let Some(existing) = prompts.iter().find(|p| p.name == name) {
                if existing.abs_path != path {
                    skipped.push(SkipReason {
                        kind: SourceKind::Prompt,
                        path,
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
                abs_path: path,
            });
        }
    }

    prompts.sort_by(|a, b| a.name.cmp(&b.name));
    (prompts, skipped)
}

/// Read a fs entry's body fresh from disk, strip any leading
/// frontmatter, and enforce the same 256 KiB cap state-backed
/// registrations face. Empty-after-strip bodies are an error so the
/// resolver returns a clear "not found" rather than serving an empty
/// resource.
pub fn read_body(abs_path: &Path) -> Result<String, String> {
    let raw = std::fs::read_to_string(abs_path)
        .map_err(|e| format!("read {}: {e}", abs_path.display()))?;
    let (_, body) = split_frontmatter(&raw);
    let trimmed = body.trim_matches('\n');
    if trimmed.is_empty() {
        return Err(format!("file {} has empty body", abs_path.display()));
    }
    if body.len() > SKILL_BODY_MAX_BYTES {
        return Err(format!(
            "file {} is too large ({} bytes; max {SKILL_BODY_MAX_BYTES})",
            abs_path.display(),
            body.len()
        ));
    }
    Ok(body.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── static_prefix ────────────────────────────────────────────────

    #[test]
    fn static_prefix_no_meta_falls_back_to_dir() {
        // `dir/file.md` has no glob meta, so the whole file path's
        // directory is the static prefix.
        assert_eq!(static_prefix("dir/sub/file.md"), "dir/sub/");
    }

    #[test]
    fn static_prefix_strips_back_to_last_slash() {
        assert_eq!(static_prefix("dir/sub/*.md"), "dir/sub/");
        assert_eq!(static_prefix("dir/**/*.md"), "dir/");
        assert_eq!(static_prefix("dir/file*.md"), "dir/");
    }

    #[test]
    fn static_prefix_no_dir_yields_empty() {
        assert_eq!(static_prefix("*.md"), "");
        assert_eq!(static_prefix("**/*.md"), "");
        assert_eq!(static_prefix("[abc].md"), "");
    }

    #[test]
    fn static_prefix_question_mark_and_brace() {
        assert_eq!(static_prefix("dir/file?.md"), "dir/");
        assert_eq!(static_prefix("dir/{a,b}.md"), "dir/");
    }

    #[test]
    fn static_prefix_absolute_paths() {
        assert_eq!(static_prefix("/abs/dir/**/*.md"), "/abs/dir/");
        assert_eq!(static_prefix("/abs/file.md"), "/abs/");
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
        // Missing closing fence — return as-is, no frontmatter.
        let content = "---\nname: x\nbody without closing fence\n";
        let (fm, body) = split_frontmatter(content);
        assert!(fm.is_none());
        assert_eq!(body, content);
    }

    #[test]
    fn split_frontmatter_must_start_at_byte_zero() {
        // A `---` line later in the file is NOT a frontmatter opener.
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

    // ── derive_relative_id ───────────────────────────────────────────

    #[test]
    fn derive_id_strips_md_and_keeps_slashes() {
        let prefix = Path::new("/base/dir/");
        let path = Path::new("/base/dir/foo/bar.md");
        assert_eq!(derive_relative_id(prefix, path).unwrap(), "foo/bar");
    }

    #[test]
    fn derive_id_flat_file() {
        let prefix = Path::new("/base/");
        let path = Path::new("/base/file.md");
        assert_eq!(derive_relative_id(prefix, path).unwrap(), "file");
    }

    #[test]
    fn derive_id_rejects_when_path_outside_prefix() {
        let prefix = Path::new("/base/dir/");
        let path = Path::new("/elsewhere/file.md");
        assert!(derive_relative_id(prefix, path).is_err());
    }

    // ── expand_skill_globs ───────────────────────────────────────────

    fn write_fixture(dir: &Path, rel: &str, contents: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn expand_skills_basic_nested() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "my-skills/a.md", "# A\n");
        write_fixture(tmp.path(), "my-skills/sub/b.md", "# B\n");
        write_fixture(tmp.path(), "my-skills/sub/deep/c.md", "# C\n");

        let (skills, skipped) = expand_skill_globs(tmp.path(), &["my-skills/**/*.md".to_string()]);
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");
        let ids: Vec<_> = skills.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "sub/b", "sub/deep/c"]);
    }

    #[test]
    fn expand_skills_rejects_invalid_id_segments() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "my-skills/Bad-Case.md", "# bad\n");
        write_fixture(tmp.path(), "my-skills/with space.md", "# space\n");
        write_fixture(tmp.path(), "my-skills/ok.md", "# ok\n");

        let (skills, skipped) = expand_skill_globs(tmp.path(), &["my-skills/**/*.md".to_string()]);
        let ids: Vec<_> = skills.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["ok"]);
        assert_eq!(skipped.len(), 2);
        for s in &skipped {
            assert_eq!(s.kind, SourceKind::Skill);
            assert!(s.reason.contains("invalid id"));
        }
    }

    #[test]
    fn expand_skills_dedupes_when_two_globs_match_same_file() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "my-skills/a.md", "# a\n");

        let (skills, skipped) = expand_skill_globs(
            tmp.path(),
            &[
                "my-skills/*.md".to_string(),
                "my-skills/**/*.md".to_string(),
            ],
        );
        assert_eq!(skills.len(), 1, "same file should not duplicate");
        assert!(skipped.is_empty(), "no skip warning for same-file dedupe");
    }

    #[test]
    fn expand_skills_intra_fs_collision_is_skipped() {
        // Two distinct files that derive the same id (`shared`) — the
        // second is recorded as a SkipReason.
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "first/shared.md", "# from first\n");
        write_fixture(tmp.path(), "second/shared.md", "# from second\n");

        let (skills, skipped) = expand_skill_globs(
            tmp.path(),
            &["first/*.md".to_string(), "second/*.md".to_string()],
        );
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].id, "shared");
        assert_eq!(skipped.len(), 1);
        assert!(
            skipped[0].reason.contains("duplicate id"),
            "got: {:?}",
            skipped[0].reason
        );
    }

    #[test]
    fn expand_skills_empty_when_no_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let (skills, skipped) =
            expand_skill_globs(tmp.path(), &["does-not-exist/**/*.md".to_string()]);
        assert!(skills.is_empty());
        assert!(skipped.is_empty());
    }

    #[test]
    fn expand_skills_invalid_glob_pattern_records_skip() {
        let tmp = tempfile::tempdir().unwrap();
        // `[` without closing `]` is an invalid pattern.
        let (skills, skipped) = expand_skill_globs(tmp.path(), &["[unclosed".to_string()]);
        assert!(skills.is_empty());
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].kind, SourceKind::Skill);
    }

    // ── expand_prompt_globs ──────────────────────────────────────────

    #[test]
    fn expand_prompts_reads_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "p/open-pr.md",
            "---\nname: open-pr\ndescription: Open a PR.\n---\nBody here.\n",
        );

        let (prompts, skipped) = expand_prompt_globs(tmp.path(), &["p/*.md".to_string()]);
        assert!(skipped.is_empty(), "unexpected skips: {skipped:?}");
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].name, "open-pr");
        assert_eq!(prompts[0].description, "Open a PR.");
    }

    #[test]
    fn expand_prompts_falls_back_to_basename_for_name() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "p/foo.md",
            "---\ndescription: Just a description.\n---\nBody.\n",
        );

        let (prompts, _skipped) = expand_prompt_globs(tmp.path(), &["p/*.md".to_string()]);
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].name, "foo");
    }

    #[test]
    fn expand_prompts_rejects_missing_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "p/no-fm.md", "# header\nbody\n");

        let (prompts, skipped) = expand_prompt_globs(tmp.path(), &["p/*.md".to_string()]);
        assert!(prompts.is_empty());
        assert_eq!(skipped.len(), 1);
        assert!(
            skipped[0].reason.contains("missing YAML frontmatter"),
            "got: {:?}",
            skipped[0].reason
        );
    }

    #[test]
    fn expand_prompts_rejects_missing_description() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path(), "p/no-desc.md", "---\nname: foo\n---\nBody.\n");
        let (prompts, skipped) = expand_prompt_globs(tmp.path(), &["p/*.md".to_string()]);
        assert!(prompts.is_empty());
        assert_eq!(skipped.len(), 1);
        assert!(
            skipped[0].reason.contains("description"),
            "got: {:?}",
            skipped[0].reason
        );
    }

    #[test]
    fn expand_prompts_rejects_invalid_name() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "p/bad.md",
            "---\nname: Has Spaces\ndescription: x\n---\nBody.\n",
        );
        let (prompts, skipped) = expand_prompt_globs(tmp.path(), &["p/*.md".to_string()]);
        assert!(prompts.is_empty());
        assert_eq!(skipped.len(), 1);
        assert!(
            skipped[0].reason.contains("invalid prompt name"),
            "got: {:?}",
            skipped[0].reason
        );
    }

    #[test]
    fn expand_prompts_collision_skips_second() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(
            tmp.path(),
            "p/a.md",
            "---\nname: shared\ndescription: from a\n---\nBody A.\n",
        );
        write_fixture(
            tmp.path(),
            "p/b.md",
            "---\nname: shared\ndescription: from b\n---\nBody B.\n",
        );
        let (prompts, skipped) = expand_prompt_globs(tmp.path(), &["p/*.md".to_string()]);
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
