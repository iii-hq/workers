//! Workers registry source: HTTP GET to
//! `{registry_url}/w/{worker}/skills?version=… | ?tag=…`, parses the
//! response shape documented below, and writes accepted skill-tree entries
//! into `<skills_folder>/<worker>/`.
//!
//! Response shape (from the workers registry):
//!
//! ```json
//! {
//!   "name": "resend",
//!   "version": "1.2.3",
//!   "skills": [
//!     { "path": "contacts.md", "content": "…" },
//!     { "path": "emails/index.md", "content": "…" }
//!   ]
//! }
//! ```
//!
//! Skills are written verbatim under the worker namespace. A structured
//! registry `prompts` field is ignored.

use std::path::Path;

use serde::Deserialize;

use super::{build_http_client, validate_relative_path, write_file_atomic, DownloadResult};

/// Specifier for which version of a worker's skills to pull.
///
/// Both arms serialise as `?version=…` on the wire — per the
/// OpenAPI contract, `/w/{slug}/skills` accepts either a tag
/// (`latest`, `beta`) or an exact semver under the same `version`
/// query param. The enum variants are preserved purely so the
/// user-facing input on `directory::skills::download` (which still
/// distinguishes `version:` from `tag:`) round-trips through the
/// download path unchanged for diagnostics and logging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionSpec {
    Version(String),
    Tag(String),
}

impl VersionSpec {
    /// The `?version=…` value to send to the registry. Both variants
    /// share the same wire key; `("version", value)` is returned to
    /// keep the `(key, value)` tuple shape callers already use.
    pub fn as_query_param(&self) -> (&'static str, &str) {
        match self {
            VersionSpec::Version(v) => ("version", v.as_str()),
            VersionSpec::Tag(t) => ("version", t.as_str()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct SkillEntry {
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct WorkerSkillsResponse {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    version: Option<String>,
    #[serde(default)]
    skills: Vec<SkillEntry>,
}

/// Validate the worker name. Mirrors the `^[a-z0-9][a-z0-9_-]*$`
/// invariant the workers registry enforces server-side.
pub fn validate_worker_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("worker name must be non-empty".into());
    }
    let first = name.chars().next().unwrap();
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err(format!(
            "worker name must start with a lowercase ASCII letter or digit: {name:?}"
        ));
    }
    for c in name.chars() {
        let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_');
        if !ok {
            return Err(format!(
                "worker name may only contain lowercase letters, digits, '-', '_': {name:?}"
            ));
        }
    }
    Ok(())
}

/// Outcome of a registry download attempt. Distinguishes between a
/// successful download, a 404 (worker has no skills bundle — benign),
/// and a real error (5xx, timeout, malformed response).
pub enum RegistryDownloadOutcome {
    /// Skills successfully downloaded and written.
    Ok(DownloadResult),
    /// HTTP 404 — the worker has no skills bundle. Benign no-op.
    NotFound,
}

/// HTTP GET the worker's skills bundle, parse the response, and write
/// every entry under `<skills_folder>/<worker>/`. The HTTP request and
/// the file writes are bounded by `timeout_ms` collectively.
pub async fn download(
    registry_base: &str,
    worker: &str,
    spec: &VersionSpec,
    skills_folder: &Path,
    timeout_ms: u64,
) -> Result<DownloadResult, String> {
    match download_typed(registry_base, worker, spec, skills_folder, timeout_ms).await? {
        RegistryDownloadOutcome::Ok(result) => Ok(result),
        RegistryDownloadOutcome::NotFound => Err(format!(
            "D310 not_found: registry worker {worker:?} has no published skills bundle. \
             Next: call directory::registry::workers::list to browse worker names."
        )),
    }
}

/// Like [`download`] but returns a typed outcome distinguishing 404
/// (benign) from real errors (5xx, timeout, malformed). Used by
/// auto-download paths that need to treat 404 as a no-op.
pub async fn download_typed(
    registry_base: &str,
    worker: &str,
    spec: &VersionSpec,
    skills_folder: &Path,
    timeout_ms: u64,
) -> Result<RegistryDownloadOutcome, String> {
    validate_worker_name(worker)?;

    let url = format!(
        "{}/w/{}/skills",
        registry_base.trim_end_matches('/'),
        worker
    );
    let (key, value) = spec.as_query_param();

    let client = build_http_client(timeout_ms)?;

    let response = client
        .get(&url)
        .query(&[(key, value)])
        .send()
        .await
        .map_err(|_| {
            "D320 registry_error: could not reach the registry. Next: retry shortly.".to_string()
        })?;

    let status = response.status();

    // 404 is benign — the worker simply has no skills bundle.
    if status.as_u16() == 404 {
        return Ok(RegistryDownloadOutcome::NotFound);
    }

    if !status.is_success() {
        // Clean error only — never leak the internal registry URL or the
        // raw response body into a handler error an agent has to read.
        return Err(format!(
            "D320 registry_error: registry returned HTTP {}. Next: retry shortly.",
            status.as_u16()
        ));
    }
    let parsed: WorkerSkillsResponse = response
        .json()
        .await
        .map_err(|_| "D320 registry_error: could not decode the registry response.".to_string())?;

    if let Some(name) = parsed.name.as_deref() {
        if name != worker {
            return Err(format!(
                "registry returned worker name {name:?}, expected {worker:?}"
            ));
        }
    }

    let result = write_response(worker, parsed, skills_folder)?;
    Ok(RegistryDownloadOutcome::Ok(result))
}

/// Registry bundles store skill files under a `skills/` directory in the source
/// repo (`skills/exec.md`, `skills/SKILL.md`). That prefix is redundant once
/// materialised under `<skills_folder>/<worker>/`, so strip a single leading
/// `skills/` segment — files land at `<worker>/exec.md`, `<worker>/SKILL.md`
/// rather than nesting a second `skills/` folder. A bundle-root file like
/// `SKILL.md` or `index.md` (no prefix) is returned unchanged.
fn strip_leading_skills_segment(path: &str) -> &str {
    path.strip_prefix("skills/").unwrap_or(path)
}

/// Stale registry snapshots may ship both legacy `index.md` and canonical
/// `SKILL.md` overviews. Prefer `SKILL.md` on disk and drop redundant
/// `index.md` entries before writing.
fn dedupe_stale_overview(skills: &mut Vec<SkillEntry>) {
    let has_skill_md = skills
        .iter()
        .any(|s| strip_leading_skills_segment(&s.path) == "SKILL.md");
    if has_skill_md {
        skills.retain(|s| strip_leading_skills_segment(&s.path) != "index.md");
    }
}

fn write_response(
    worker: &str,
    mut response: WorkerSkillsResponse,
    skills_folder: &Path,
) -> Result<DownloadResult, String> {
    let dest_root = skills_folder.join(worker);
    std::fs::create_dir_all(&dest_root)
        .map_err(|e| format!("create_dir_all {}: {e}", dest_root.display()))?;

    dedupe_stale_overview(&mut response.skills);

    let mut result = DownloadResult::new(worker);

    for skill in response.skills {
        // Drop the redundant leading `skills/` packaging prefix so files land at
        // `<worker>/<path>` instead of a nested `<worker>/skills/<path>`.
        let normalized = strip_leading_skills_segment(&skill.path);
        let rel = validate_relative_path(normalized)
            .map_err(|e| format!("invalid skill path {:?}: {e}", skill.path))?;
        let Some(kind) = crate::fs_source::classify_rel_path(&rel) else {
            continue;
        };
        let dest = dest_root.join(&rel);
        write_file_atomic(&dest, skill.content.as_bytes())?;

        // The registry calls every entry in `skills[]` a "skill", but the shared
        // classifier rejects `prompts/` paths and reports system prompts and agents
        // to their own on-change buckets. Those buckets use the bare file stem;
        // the skills bucket keeps the path.
        match kind {
            crate::fs_source::SourceKind::SystemPrompt
                if rel.extension().is_some_and(|e| e == "md") =>
            {
                if let Some(stem) = super::git::stem_of(&rel) {
                    result.system_prompts_written.push(stem);
                }
            }
            crate::fs_source::SourceKind::Agent if rel.extension().is_some_and(|e| e == "md") => {
                if let Some(stem) = super::git::stem_of(&rel) {
                    result.agents_written.push(stem);
                }
            }
            // Non-md support files under an agents/ dir are written but
            // not reported — they'd fire the wrong on-change and pollute
            // the skills bucket.
            crate::fs_source::SourceKind::Agent => {}
            _ => result
                .skills_written
                .push(rel.to_string_lossy().replace('\\', "/")),
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_worker_name_accepts_simple() {
        assert!(validate_worker_name("resend").is_ok());
        assert!(validate_worker_name("agent-memory").is_ok());
        assert!(validate_worker_name("foo_bar").is_ok());
        assert!(validate_worker_name("v2-worker").is_ok());
    }

    #[test]
    fn validate_worker_name_rejects_uppercase_and_separators() {
        assert!(validate_worker_name("").is_err());
        assert!(validate_worker_name("Resend").is_err());
        assert!(validate_worker_name("a/b").is_err());
        assert!(validate_worker_name("-leading").is_err());
        assert!(validate_worker_name("with space").is_err());
    }

    #[test]
    fn version_spec_query_param_always_uses_version_key() {
        let v = VersionSpec::Version("1.2.3".into());
        assert_eq!(v.as_query_param(), ("version", "1.2.3"));
        let t = VersionSpec::Tag("latest".into());
        // Per the OpenAPI contract, both tags and exact semvers go on
        // the wire as `?version=…`.
        assert_eq!(t.as_query_param(), ("version", "latest"));
    }

    #[test]
    fn write_response_ignores_structured_command_prompts() {
        let tmp = tempfile::tempdir().unwrap();
        let response: WorkerSkillsResponse = serde_json::from_str(
            r##"{
                "name": "resend",
                "version": "1.2.3",
                "skills": [
                    { "path": "index.md", "content": "# resend\n" },
                    { "path": "emails/send-email.md", "content": "# send-email\n" }
                ],
                "prompts": [{ "name": "send-email", "content": "Body." }]
            }"##,
        )
        .unwrap();
        let result = write_response("resend", response, tmp.path()).unwrap();
        assert_eq!(result.namespace, "resend");
        assert_eq!(result.skills_written.len(), 2);
        assert!(!tmp.path().join("resend/prompts/send-email.md").exists());

        let index = std::fs::read_to_string(tmp.path().join("resend/index.md")).unwrap();
        assert_eq!(index, "# resend\n");
    }

    #[test]
    fn write_response_classifies_system_prompt_path_in_skills_array() {
        // The registry response only has ONE array for markdown files —
        // `skills[]` — so a bundle that ships a system prompt lists it there
        // too, at a path under `system-prompts/`. It must still land in
        // `system_prompts_written` (and fire `system-prompts::on-change`,
        // not `skills::on-change`), exactly like `git.rs::copy_recursive`
        // classifies a cloned repo's tree.
        let tmp = tempfile::tempdir().unwrap();
        let response = WorkerSkillsResponse {
            name: Some("resend".into()),
            version: Some("1.2.3".into()),
            skills: vec![
                SkillEntry {
                    path: "index.md".into(),
                    content: "# resend\n".into(),
                },
                SkillEntry {
                    path: "system-prompts/reviewer.md".into(),
                    content: "---\ndescription: reviews PRs\n---\nBody.\n".into(),
                },
            ],
        };
        let result = write_response("resend", response, tmp.path()).unwrap();
        assert_eq!(result.skills_written, vec!["index.md".to_string()]);
        assert_eq!(result.system_prompts_written, vec!["reviewer".to_string()]);

        // Still lands on disk at the path the registry sent — only the
        // REPORTED bucket (and thus which on-change trigger fires) changes.
        let sys =
            std::fs::read_to_string(tmp.path().join("resend/system-prompts/reviewer.md")).unwrap();
        assert!(sys.contains("Body."));
    }

    #[test]
    fn strip_leading_skills_segment_drops_one_prefix() {
        assert_eq!(strip_leading_skills_segment("skills/SKILL.md"), "SKILL.md");
        assert_eq!(strip_leading_skills_segment("skills/exec.md"), "exec.md");
        assert_eq!(
            strip_leading_skills_segment("skills/iii-database/query.md"),
            "iii-database/query.md"
        );
        // Only ONE leading segment is stripped (bundle's own nested skills/ kept).
        assert_eq!(
            strip_leading_skills_segment("skills/skills/http/x.md"),
            "skills/http/x.md"
        );
        // Bundle-root files and non-prefixed paths are untouched.
        assert_eq!(strip_leading_skills_segment("index.md"), "index.md");
        assert_eq!(strip_leading_skills_segment("SKILL.md"), "SKILL.md");
        assert_eq!(strip_leading_skills_segment("a/b.md"), "a/b.md");
    }

    #[test]
    fn write_response_flattens_skills_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let response = WorkerSkillsResponse {
            name: Some("iii".into()),
            version: None,
            skills: vec![SkillEntry {
                path: "skills/SKILL.md".into(),
                content: "# skill\n".into(),
            }],
        };
        let result = write_response("iii", response, tmp.path()).unwrap();
        // Lands at iii/SKILL.md, NOT iii/skills/SKILL.md.
        assert!(tmp.path().join("iii/SKILL.md").is_file());
        assert!(!tmp.path().join("iii/skills/SKILL.md").exists());
        assert!(!tmp.path().join("iii/index.md").exists());
        assert_eq!(result.skills_written, vec!["SKILL.md"]);
    }

    #[test]
    fn write_response_dedupes_stale_index_when_skill_md_present() {
        let tmp = tempfile::tempdir().unwrap();
        let response = WorkerSkillsResponse {
            name: Some("iii-directory".into()),
            version: None,
            skills: vec![
                SkillEntry {
                    path: "index.md".into(),
                    content: "# stale index\n".into(),
                },
                SkillEntry {
                    path: "SKILL.md".into(),
                    content: "# canonical\n".into(),
                },
            ],
        };
        let result = write_response("iii-directory", response, tmp.path()).unwrap();
        assert!(tmp.path().join("iii-directory/SKILL.md").is_file());
        assert!(!tmp.path().join("iii-directory/index.md").exists());
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("iii-directory/SKILL.md")).unwrap(),
            "# canonical\n"
        );
        assert_eq!(result.skills_written, vec!["SKILL.md"]);
    }

    #[test]
    fn write_response_rejects_path_traversal_in_skill_path() {
        let tmp = tempfile::tempdir().unwrap();
        let response = WorkerSkillsResponse {
            name: None,
            version: None,
            skills: vec![SkillEntry {
                path: "../../../etc/passwd".into(),
                content: "x".into(),
            }],
        };
        let err = write_response("resend", response, tmp.path()).unwrap_err();
        assert!(err.contains("invalid skill path"), "got: {err}");
    }
}
