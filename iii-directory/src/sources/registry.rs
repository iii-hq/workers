//! Workers registry source: HTTP GET to
//! `{registry_url}/w/{worker}/skills?version=… | ?tag=…`, parses the
//! response shape documented below, and writes accepted skills into
//! `<skills_folder>/<worker>/` and agent profiles into `agents_folder`.
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
//! Skills are written verbatim under the worker namespace. Exact
//! `agents/<id>.md` entries are routed to the agent profile root. A
//! structured registry `prompts` field is ignored.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::functions::download::read_worker_markers;
use crate::functions::prompts::validate_name;
use crate::kits::lock::sha256_hex;

use super::{
    DownloadResult, SkippedAgent, back_up, build_http_client, read_existing,
    validate_relative_path, write_file_atomic,
};

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
pub struct WorkerSkillsResponse {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
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

/// Outcome of a registry request for a bundle. Distinguishes between a
/// bundle, a 404 (worker has no skills bundle — benign), a registry that
/// could not answer now, and (as `Err`) any other error.
pub enum RegistryFetchOutcome {
    /// The bundle, not written yet.
    Ok(WorkerSkillsResponse),
    /// HTTP 404 — the worker has no skills bundle. Benign no-op.
    NotFound,
    /// Unreachable, a 5xx or a 429: worth asking again later. Carries the
    /// error to report.
    Unavailable(String),
}

/// HTTP GET the worker's directory bundle (bounded by `timeout_ms`), parse
/// the response, and route skills and agent profiles to their configured
/// roots.
///
/// `kit_agents` maps agent ids owned by an installed kit to that kit: those
/// profiles are never overwritten by a worker bundle and are reported in
/// `agents_skipped` instead, like the ones another worker wrote and the ones
/// made or edited in the project.
pub async fn download(
    registry_base: &str,
    worker: &str,
    spec: &VersionSpec,
    skills_folder: &Path,
    agents_folder: &Path,
    timeout_ms: u64,
    kit_agents: &BTreeMap<String, String>,
) -> Result<DownloadResult, String> {
    match fetch(registry_base, worker, spec, timeout_ms).await? {
        RegistryFetchOutcome::Ok(response) => {
            write_response(worker, response, skills_folder, agents_folder, kit_agents)
        }
        RegistryFetchOutcome::NotFound => Err(format!(
            "D310 not_found: registry worker {worker:?} has no published skills bundle. \
             Next: call directory::registry::workers::list to browse worker names."
        )),
        RegistryFetchOutcome::Unavailable(error) => Err(error),
    }
}

/// The request half of [`download`]. The typed outcome tells a 404 (benign)
/// and an unavailable registry (unreachable, 5xx, 429) from other errors:
/// the auto-download treats a 404 as a no-op, retries an unavailable
/// registry, and writes the bundle only when its version is new.
pub async fn fetch(
    registry_base: &str,
    worker: &str,
    spec: &VersionSpec,
    timeout_ms: u64,
) -> Result<RegistryFetchOutcome, String> {
    validate_worker_name(worker)?;

    let url = format!(
        "{}/w/{}/skills",
        registry_base.trim_end_matches('/'),
        worker
    );
    let (key, value) = spec.as_query_param();

    let client = build_http_client(timeout_ms)?;

    let Ok(response) = client.get(&url).query(&[(key, value)]).send().await else {
        return Ok(RegistryFetchOutcome::Unavailable(
            "D320 registry_error: could not reach the registry. Next: retry shortly.".to_string(),
        ));
    };

    let status = response.status();

    // 404 is benign — the worker simply has no skills bundle.
    if status.as_u16() == 404 {
        return Ok(RegistryFetchOutcome::NotFound);
    }

    if !status.is_success() {
        // Clean error only — never leak the internal registry URL or the
        // raw response body into a handler error an agent has to read.
        let error = format!(
            "D320 registry_error: registry returned HTTP {}. Next: retry shortly.",
            status.as_u16()
        );
        if status.is_server_error() || status.as_u16() == 429 {
            return Ok(RegistryFetchOutcome::Unavailable(error));
        }
        return Err(error);
    }
    let parsed: WorkerSkillsResponse = response
        .json()
        .await
        .map_err(|_| "D320 registry_error: could not decode the registry response.".to_string())?;

    if let Some(name) = parsed.name.as_deref()
        && name != worker
    {
        return Err(format!(
            "registry returned worker name {name:?}, expected {worker:?}"
        ));
    }

    Ok(RegistryFetchOutcome::Ok(parsed))
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

/// Write a bundle over what is on disk. The worker's previous completion
/// marker tells its untouched files, which are replaced, from the rest: any
/// other skill or system prompt is backed up first, any other agent profile
/// is kept.
pub fn write_response(
    worker: &str,
    mut response: WorkerSkillsResponse,
    skills_folder: &Path,
    agents_folder: &Path,
    kit_agents: &BTreeMap<String, String>,
) -> Result<DownloadResult, String> {
    let dest_root = skills_folder.join(worker);
    std::fs::create_dir_all(&dest_root)
        .map_err(|e| format!("create_dir_all {}: {e}", dest_root.display()))?;

    dedupe_stale_overview(&mut response.skills);

    let markers = read_worker_markers(skills_folder);
    let previous = markers.iter().find(|m| m.worker == worker);
    let mut result = DownloadResult::new(worker);
    result.version = response.version.clone();

    for skill in response.skills {
        let body = skill.content.as_bytes();
        // Drop the redundant leading `skills/` packaging prefix so files land at
        // `<worker>/<path>` instead of a nested `<worker>/skills/<path>`.
        let normalized = strip_leading_skills_segment(&skill.path);
        let rel = validate_relative_path(normalized)
            .map_err(|e| format!("invalid skill path {:?}: {e}", skill.path))?;
        if let Some(id) = bundle_agent_id(&rel) {
            if validate_name(id).is_err() {
                continue;
            }
            // Kit profiles take precedence over the ones workers ship.
            if let Some(kit) = kit_agents.get(id) {
                result.agents_skipped.push(SkippedAgent {
                    id: id.to_string(),
                    owned_by: kit.clone(),
                });
                continue;
            }
            let dest = agents_folder.join(format!("{id}.md"));
            let current = read_existing(&dest)?;
            if current.as_deref() != Some(body) {
                let recorded = previous.and_then(|m| m.agents.get(id));
                if let Some(current) = &current
                    && recorded != Some(&sha256_hex(current))
                {
                    let owner = match recorded {
                        // Ours, edited here: still attributed to this worker.
                        Some(sha) => {
                            result.agent_shas.insert(id.to_string(), sha.clone());
                            None
                        }
                        None => markers.iter().find(|m| m.agents.contains_key(id)),
                    };
                    result.agents_skipped.push(SkippedAgent {
                        id: id.to_string(),
                        owned_by: owner
                            .map_or_else(|| "project".into(), |m| format!("worker {}", m.worker)),
                    });
                    continue;
                }
                write_file_atomic(&dest, body)?;
            }
            result.agents_written.push(id.to_string());
            result.agent_shas.insert(id.to_string(), sha256_hex(body));
            continue;
        }
        let Some(kind) = crate::fs_source::classify_rel_path(&rel) else {
            continue;
        };
        if kind == crate::fs_source::SourceKind::Agent {
            continue;
        }
        let dest = dest_root.join(&rel);
        let path = rel.to_string_lossy().replace('\\', "/");
        let current = read_existing(&dest)?;
        if current.as_deref() != Some(body) {
            let recorded = previous.and_then(|m| m.files.get(&path));
            if let Some(current) = &current
                && recorded != Some(&sha256_hex(current))
            {
                let backup = back_up(&dest, current)?;
                tracing::warn!(
                    worker,
                    file = %dest.display(),
                    backup = %backup.display(),
                    "replacing a file that differs from what the last download wrote; kept a copy"
                );
            }
            write_file_atomic(&dest, body)?;
        }
        result.file_shas.insert(path.clone(), sha256_hex(body));

        // The registry calls every entry in `skills[]` a "skill", but the shared
        // classifier rejects `prompts/` paths and reports system prompts to their
        // own on-change bucket. That bucket uses the bare file stem; the skills
        // bucket keeps the path.
        match kind {
            crate::fs_source::SourceKind::SystemPrompt
                if rel.extension().is_some_and(|e| e == "md") =>
            {
                if let Some(stem) = super::git::stem_of(&rel) {
                    result.system_prompts_written.push(stem);
                }
            }
            _ => result.skills_written.push(path),
        }
    }

    Ok(result)
}

fn bundle_agent_id(rel: &Path) -> Option<&str> {
    if rel.parent()? != Path::new(crate::fs_source::AGENTS_SEGMENT) || rel.extension()? != "md" {
        return None;
    }
    rel.file_stem()?.to_str().filter(|id| !id.is_empty())
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
        let result =
            write_response("resend", response, tmp.path(), tmp.path(), &BTreeMap::new()).unwrap();
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
        let result =
            write_response("resend", response, tmp.path(), tmp.path(), &BTreeMap::new()).unwrap();
        assert_eq!(result.skills_written, vec!["index.md".to_string()]);
        assert_eq!(result.system_prompts_written, vec!["reviewer".to_string()]);

        // Still lands on disk at the path the registry sent — only the
        // REPORTED bucket (and thus which on-change is called) changes.
        let sys =
            std::fs::read_to_string(tmp.path().join("resend/system-prompts/reviewer.md")).unwrap();
        assert!(sys.contains("Body."));
    }

    #[test]
    fn write_response_routes_agent_to_dedicated_root() {
        let skills = tempfile::tempdir().unwrap();
        let agents = tempfile::tempdir().unwrap();
        let response = WorkerSkillsResponse {
            name: Some("harness".into()),
            version: Some("1.2.3".into()),
            skills: vec![SkillEntry {
                path: "agents/reviewer.md".into(),
                content: "---\nname: Reviewer\n---\nReview changes.\n".into(),
            }],
        };

        let result = write_response(
            "harness",
            response,
            skills.path(),
            agents.path(),
            &BTreeMap::new(),
        )
        .unwrap();

        assert_eq!(result.agents_written, vec!["reviewer"]);
        assert!(agents.path().join("reviewer.md").is_file());
        assert!(!skills.path().join("harness/agents/reviewer.md").exists());
    }

    #[test]
    fn write_response_skips_kit_owned_agents_and_records_shas() {
        let skills = tempfile::tempdir().unwrap();
        let agents = tempfile::tempdir().unwrap();
        std::fs::write(agents.path().join("reviewer.md"), "kit version").unwrap();
        let response = WorkerSkillsResponse {
            name: Some("kanban".into()),
            version: Some("1.6.1".into()),
            skills: vec![
                SkillEntry {
                    path: "agents/reviewer.md".into(),
                    content: "---\nname: Reviewer\n---\nWorker body.\n".into(),
                },
                SkillEntry {
                    path: "agents/triager.md".into(),
                    content: "---\nname: Triager\n---\nBody.\n".into(),
                },
            ],
        };
        let kit_agents = BTreeMap::from([("reviewer".to_string(), "acme/team".to_string())]);

        let result = write_response(
            "kanban",
            response,
            skills.path(),
            agents.path(),
            &kit_agents,
        )
        .unwrap();

        assert_eq!(result.agents_written, vec!["triager"]);
        assert_eq!(
            result.agents_skipped,
            vec![SkippedAgent {
                id: "reviewer".into(),
                owned_by: "acme/team".into()
            }]
        );
        assert_eq!(
            std::fs::read_to_string(agents.path().join("reviewer.md")).unwrap(),
            "kit version"
        );
        assert_eq!(result.version.as_deref(), Some("1.6.1"));
        assert_eq!(
            result.agent_shas.get("triager").map(String::as_str),
            Some(sha256_hex(b"---\nname: Triager\n---\nBody.\n").as_str())
        );
    }

    #[test]
    fn write_response_ignores_invalid_agent_id() {
        let skills = tempfile::tempdir().unwrap();
        let agents = tempfile::tempdir().unwrap();
        let response = WorkerSkillsResponse {
            name: Some("harness".into()),
            version: None,
            skills: vec![SkillEntry {
                path: "agents/Bad-Id.md".into(),
                content: "---\nname: Bad\n---\nBody.\n".into(),
            }],
        };

        let result = write_response(
            "harness",
            response,
            skills.path(),
            agents.path(),
            &BTreeMap::new(),
        )
        .unwrap();

        assert!(result.agents_written.is_empty());
        assert!(!agents.path().join("Bad-Id.md").exists());
    }

    #[test]
    fn write_response_ignores_nested_agent_path() {
        let skills = tempfile::tempdir().unwrap();
        let agents = tempfile::tempdir().unwrap();
        let response = WorkerSkillsResponse {
            name: Some("harness".into()),
            version: None,
            skills: vec![SkillEntry {
                path: "agents/review/reviewer.md".into(),
                content: "---\nname: Reviewer\n---\nBody.\n".into(),
            }],
        };

        let result = write_response(
            "harness",
            response,
            skills.path(),
            agents.path(),
            &BTreeMap::new(),
        )
        .unwrap();

        assert_eq!(result.total_files(), 0);
        assert!(!agents.path().join("review/reviewer.md").exists());
        assert!(
            !skills
                .path()
                .join("harness/agents/review/reviewer.md")
                .exists()
        );
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
        let result =
            write_response("iii", response, tmp.path(), tmp.path(), &BTreeMap::new()).unwrap();
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
        let result = write_response(
            "iii-directory",
            response,
            tmp.path(),
            tmp.path(),
            &BTreeMap::new(),
        )
        .unwrap();
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
        let err = write_response("resend", response, tmp.path(), tmp.path(), &BTreeMap::new())
            .unwrap_err();
        assert!(err.contains("invalid skill path"), "got: {err}");
    }

    // ── files already on disk ─────────────────────────────────────────

    /// Write a bundle and its completion marker, as a download does.
    fn install(
        worker: &str,
        version: &str,
        files: &[(&str, &str)],
        skills: &Path,
        agents: &Path,
    ) -> DownloadResult {
        let response = WorkerSkillsResponse {
            name: Some(worker.into()),
            version: Some(version.into()),
            skills: files
                .iter()
                .map(|(path, content)| SkillEntry {
                    path: (*path).into(),
                    content: (*content).into(),
                })
                .collect(),
        };
        let result = write_response(worker, response, skills, agents, &BTreeMap::new()).unwrap();
        crate::functions::download::write_completion_marker(
            skills,
            worker,
            &VersionSpec::Version(version.into()),
            &result,
        )
        .unwrap();
        result
    }

    /// Contents of the backups in `dir`, sorted.
    fn backups(dir: &Path) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "bak"))
            .map(|e| std::fs::read_to_string(e.path()).unwrap())
            .collect();
        out.sort();
        out
    }

    #[test]
    fn an_untouched_file_is_replaced_without_a_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let (skills, agents) = (tmp.path().join("skills"), tmp.path().join("agents"));
        let file = skills.join("ade/mentions.md");
        install("ade", "1.0.0", &[("mentions.md", "one")], &skills, &agents);
        install("ade", "1.1.0", &[("mentions.md", "two")], &skills, &agents);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "two");
        assert!(backups(&skills.join("ade")).is_empty());
    }

    #[test]
    fn every_edit_is_backed_up_before_it_is_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let (skills, agents) = (tmp.path().join("skills"), tmp.path().join("agents"));
        let file = skills.join("ade/mentions.md");
        for (version, shipped, edit) in [("1.0.0", "one", "my one"), ("1.1.0", "two", "my two")] {
            install(
                "ade",
                version,
                &[("mentions.md", shipped)],
                &skills,
                &agents,
            );
            assert_eq!(std::fs::read_to_string(&file).unwrap(), shipped);
            std::fs::write(&file, edit).unwrap();
        }
        install(
            "ade",
            "1.2.0",
            &[("mentions.md", "three")],
            &skills,
            &agents,
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "three");
        assert_eq!(backups(&skills.join("ade")), ["my one", "my two"]);
        // A backup is not a skill.
        let ids: Vec<String> = crate::fs_source::scan_skills(&skills)
            .0
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, ["ade/mentions"]);
    }

    #[test]
    fn a_file_the_marker_does_not_record_is_backed_up() {
        let tmp = tempfile::tempdir().unwrap();
        let (skills, agents) = (tmp.path().join("skills"), tmp.path().join("agents"));
        std::fs::create_dir_all(skills.join("iii/system-prompts")).unwrap();
        std::fs::write(skills.join("iii/index.md"), "stale").unwrap();
        std::fs::write(skills.join("iii/system-prompts/p.md"), "stale prompt").unwrap();
        std::fs::write(
            skills.join("iii/.iii-skill-complete"),
            r#"{"worker":"iii","source":"registry","tag_or_version":"latest","schema":1}"#,
        )
        .unwrap();
        let bundle = [("index.md", "current"), ("system-prompts/p.md", "prompt")];
        install("iii", "0.26.0", &bundle, &skills, &agents);
        assert_eq!(
            std::fs::read_to_string(skills.join("iii/index.md")).unwrap(),
            "current"
        );
        assert_eq!(backups(&skills.join("iii")), ["stale"]);
        assert_eq!(
            backups(&skills.join("iii/system-prompts")),
            ["stale prompt"]
        );
    }

    #[test]
    fn identical_content_is_not_written_again() {
        let tmp = tempfile::tempdir().unwrap();
        let (skills, agents) = (tmp.path().join("skills"), tmp.path().join("agents"));
        let bundle = [("index.md", "same"), ("agents/lead.md", "same")];
        install("ade", "1.0.0", &bundle, &skills, &agents);
        let past = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        let files = [skills.join("ade/index.md"), agents.join("lead.md")];
        for file in &files {
            let handle = std::fs::File::options().write(true).open(file).unwrap();
            handle.set_modified(past).unwrap();
        }
        let result = install("ade", "1.1.0", &bundle, &skills, &agents);
        for file in &files {
            assert_eq!(file.metadata().unwrap().modified().unwrap(), past);
        }
        assert_eq!(result.skills_written, ["index.md"], "still reported");
        assert_eq!(result.agents_written, ["lead"], "still reported");
        assert!(backups(&skills.join("ade")).is_empty());
    }

    #[test]
    fn an_untouched_worker_profile_is_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let (skills, agents) = (tmp.path().join("skills"), tmp.path().join("agents"));
        install(
            "kanban",
            "1.0.0",
            &[("agents/lead.md", "one")],
            &skills,
            &agents,
        );
        let result = install(
            "kanban",
            "1.1.0",
            &[("agents/lead.md", "two")],
            &skills,
            &agents,
        );
        assert_eq!(
            std::fs::read_to_string(agents.join("lead.md")).unwrap(),
            "two"
        );
        assert!(result.agents_skipped.is_empty());
    }

    #[test]
    fn a_profile_made_in_the_project_is_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let (skills, agents) = (tmp.path().join("skills"), tmp.path().join("agents"));
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("lead.md"), "mine").unwrap();
        let result = install(
            "kanban",
            "1.0.0",
            &[("agents/lead.md", "theirs")],
            &skills,
            &agents,
        );
        assert_eq!(
            std::fs::read_to_string(agents.join("lead.md")).unwrap(),
            "mine"
        );
        assert!(result.agents_written.is_empty());
        assert_eq!(
            result.agents_skipped,
            [SkippedAgent {
                id: "lead".into(),
                owned_by: "project".into()
            }]
        );
        assert!(result.agent_shas.is_empty(), "never the worker's");
    }

    #[test]
    fn an_edited_worker_profile_is_kept_and_stays_the_workers() {
        let tmp = tempfile::tempdir().unwrap();
        let (skills, agents) = (tmp.path().join("skills"), tmp.path().join("agents"));
        let file = agents.join("lead.md");
        install(
            "kanban",
            "1.0.0",
            &[("agents/lead.md", "one")],
            &skills,
            &agents,
        );
        std::fs::write(&file, "edited").unwrap();
        let result = install(
            "kanban",
            "1.1.0",
            &[("agents/lead.md", "two")],
            &skills,
            &agents,
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "edited");
        assert_eq!(result.agents_skipped[0].owned_by, "project");

        // Undoing the edit hands the file back to the worker's updates.
        std::fs::write(&file, "one").unwrap();
        install(
            "kanban",
            "1.2.0",
            &[("agents/lead.md", "three")],
            &skills,
            &agents,
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "three");
    }

    #[test]
    fn another_workers_profile_is_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let (skills, agents) = (tmp.path().join("skills"), tmp.path().join("agents"));
        install(
            "kanban",
            "1.0.0",
            &[("agents/lead.md", "kanban")],
            &skills,
            &agents,
        );
        let result = install(
            "zeta",
            "1.0.0",
            &[("agents/lead.md", "zeta")],
            &skills,
            &agents,
        );
        assert_eq!(
            std::fs::read_to_string(agents.join("lead.md")).unwrap(),
            "kanban"
        );
        assert_eq!(result.agents_skipped[0].owned_by, "worker kanban");
    }
}
