//! `directory::skills::download` — pull markdown into `skills_folder`
//! from either the workers registry (`worker=` source) or a GitHub
//! repo (`repo=` source).
//!
//! The function is the only write path in the worker. It validates the
//! incoming arguments, dispatches to the matching source module under
//! [`crate::sources`], and fires the `directory::skills::on-change` /
//! `directory::prompts::on-change` triggers on success so that
//! subscribers (the `mcp` worker today) can forward MCP
//! `notifications/*_list_changed` to their clients.

use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, III};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::{SharedConfig, SkillsConfig};
use crate::sources::{self, registry::VersionSpec, DownloadResult};
use crate::trigger_types::{self, SubscriberSet};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct DownloadInput {
    /// Source A: GitHub repo URL. Pair with `skill`.
    #[serde(default)]
    pub repo: Option<String>,
    /// Source A: subfolder under `skills/` inside the repo. Doubles as
    /// the destination namespace inside `skills_folder`.
    #[serde(default)]
    pub skill: Option<String>,
    /// Source A: branch to clone. Defaults to `"main"`. Pass
    /// `"master"` (or any other branch name) for repos whose default
    /// branch is not `main`.
    #[serde(default)]
    pub branch: Option<String>,

    /// Source B: workers registry name. Pair with exactly one of
    /// `version` / `tag`.
    #[serde(default)]
    pub worker: Option<String>,
    /// Source B: explicit semver to pull. Mutually exclusive with `tag`.
    #[serde(default)]
    pub version: Option<String>,
    /// Source B: registry tag to pull (e.g. `latest`). Mutually
    /// exclusive with `version`. Defaults to `"latest"` when neither
    /// `version` nor `tag` is provided.
    #[serde(default)]
    pub tag: Option<String>,
}

/// Input for `directory::skills::download_from_registry`. The required
/// `worker` field is what makes this function's source unambiguous at
/// the schema level.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct RegistryDownloadInput {
    /// Worker name in the registry (e.g. `"shell"`).
    pub worker: String,
    /// Explicit semver to pull. Mutually exclusive with `tag`.
    #[serde(default)]
    pub version: Option<String>,
    /// Registry tag to pull (e.g. `"latest"`). Mutually exclusive with
    /// `version`. Defaults to `"latest"` when neither is provided.
    #[serde(default)]
    pub tag: Option<String>,
}

/// Input for `directory::skills::download_from_repo`. The required
/// `repo` + `skill` fields make this function's source unambiguous at
/// the schema level.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct RepoDownloadInput {
    /// GitHub repo URL (validated: https / ssh / git@ only).
    pub repo: String,
    /// Subfolder under `skills/` inside the repo. Doubles as the
    /// destination namespace inside `skills_folder`.
    pub skill: String,
    /// Branch to clone. Defaults to `"main"`.
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct DownloadOutput {
    namespace: String,
    skills_written: Vec<String>,
    prompts_written: Vec<String>,
    source: Value,
}

/// Disambiguated input shape produced by [`classify_input`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassifiedInput {
    Repo {
        repo: String,
        skill: String,
        branch: String,
    },
    Registry {
        worker: String,
        spec: VersionSpec,
    },
}

/// Default branch passed to `git clone` when the caller doesn't
/// specify one. GitHub flipped the default-branch convention to `main`
/// in 2020 and the registry's source-of-truth repos all use it; users
/// with `master`-default repos can override via the `branch` field.
pub const DEFAULT_REPO_BRANCH: &str = "main";

pub fn register(iii: &Arc<III>, cfg: &SharedConfig, subscribers: &super::Subscribers) {
    register_download(iii, cfg, subscribers);
    register_download_from_registry(iii, cfg, subscribers);
    register_download_from_repo(iii, cfg, subscribers);
}

/// Shared pipeline for all three download functions: validate + classify
/// the source, pull it, fan out the change notification, build the response.
async fn run_and_fan_out(
    iii: &III,
    cfg: &SkillsConfig,
    skills_subs: &SubscriberSet,
    prompts_subs: &SubscriberSet,
    input: DownloadInput,
) -> Result<DownloadOutput, IIIError> {
    let classified = classify_input(input).map_err(IIIError::Handler)?;
    let result = run_download(cfg, &classified)
        .await
        .map_err(IIIError::Handler)?;
    fan_out(iii, skills_subs, prompts_subs, &classified, &result).await;
    Ok(build_output(&classified, result))
}

/// `directory::skills::download` — flexible alias that accepts either
/// source set. Kept for back-compat; new callers should prefer the
/// explicit `download_from_registry` / `download_from_repo`, whose
/// schemas make the source unambiguous.
fn register_download(iii: &Arc<III>, cfg: &SharedConfig, subscribers: &super::Subscribers) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    let skills_subs = subscribers.skills.clone();
    let prompts_subs = subscribers.prompts.clone();
    iii.register_function(
        "directory::skills::download",
        RegisterFunction::new_async(move |req: DownloadInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.load_full();
            let skills_subs = skills_subs.clone();
            let prompts_subs = prompts_subs.clone();
            async move { run_and_fan_out(&iii, &cfg, &skills_subs, &prompts_subs, req).await }
        })
        .description(
            "Download skills + prompts into skills_folder from EITHER source. Prefer the \
             explicit directory::skills::download_from_registry / \
             directory::skills::download_from_repo, whose schemas can't be mixed up. \
             Pass {repo, skill, branch?} to clone one skill folder from a GitHub repo \
             (branch defaults to \"main\"), or {worker, version?|tag?} to pull from the \
             workers registry (tag defaults to \"latest\"). Specify exactly ONE source \
             set. Files in the destination namespace are overwritten file-by-file.",
        )
        .metadata(json!({"tool": {"label": "Download skills"}})),
    );
}

/// `directory::skills::download_from_registry` — registry source only.
/// The required `worker` field makes the source unambiguous at the
/// schema level (no "specify exactly one of two groups" guesswork).
fn register_download_from_registry(
    iii: &Arc<III>,
    cfg: &SharedConfig,
    subscribers: &super::Subscribers,
) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    let skills_subs = subscribers.skills.clone();
    let prompts_subs = subscribers.prompts.clone();
    iii.register_function(
        "directory::skills::download_from_registry",
        RegisterFunction::new_async(move |req: RegistryDownloadInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.load_full();
            let skills_subs = skills_subs.clone();
            let prompts_subs = prompts_subs.clone();
            async move {
                let input = DownloadInput {
                    worker: Some(req.worker),
                    version: req.version,
                    tag: req.tag,
                    ..Default::default()
                };
                run_and_fan_out(&iii, &cfg, &skills_subs, &prompts_subs, input).await
            }
        })
        .description(
            "Download one worker's skills + prompts from the workers registry into \
             skills_folder. `worker` is required; pass either `version` (exact semver) \
             OR `tag` (e.g. \"latest\", the default when both are omitted), not both. \
             Files in the destination namespace are overwritten file-by-file. A missing \
             worker returns a `D310 not_found` naming the next function to call. To pull \
             from a GitHub repo instead, use directory::skills::download_from_repo.",
        )
        .metadata(json!({"tool": {"label": "Download skills (registry)"}})),
    );
}

/// `directory::skills::download_from_repo` — GitHub repo source only.
/// The required `repo` + `skill` fields make the source unambiguous at
/// the schema level.
fn register_download_from_repo(
    iii: &Arc<III>,
    cfg: &SharedConfig,
    subscribers: &super::Subscribers,
) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    let skills_subs = subscribers.skills.clone();
    let prompts_subs = subscribers.prompts.clone();
    iii.register_function(
        "directory::skills::download_from_repo",
        RegisterFunction::new_async(move |req: RepoDownloadInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.load_full();
            let skills_subs = skills_subs.clone();
            let prompts_subs = prompts_subs.clone();
            async move {
                let input = DownloadInput {
                    repo: Some(req.repo),
                    skill: Some(req.skill),
                    branch: req.branch,
                    ..Default::default()
                };
                run_and_fan_out(&iii, &cfg, &skills_subs, &prompts_subs, input).await
            }
        })
        .description(
            "Download one skill folder from a GitHub repo into skills_folder. `repo` (the \
             repo URL) and `skill` (the subfolder under `skills/`, which also names the \
             destination namespace) are required; `branch` defaults to \"main\". The repo \
             URL is validated (https / ssh / git@ only). To pull a published worker \
             instead, use directory::skills::download_from_registry.",
        )
        .metadata(json!({"tool": {"label": "Download skills (repo)"}})),
    );
}

/// Validate the incoming arguments and pick exactly one source. Public
/// so the pure validation can be unit-tested without the engine.
pub fn classify_input(input: DownloadInput) -> Result<ClassifiedInput, String> {
    let DownloadInput {
        repo,
        skill,
        branch,
        worker,
        version,
        tag,
    } = input;

    let repo = repo.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let skill = skill
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let branch = branch
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let worker = worker
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let version = version
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let tag = tag.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());

    let has_repo = repo.is_some() || skill.is_some() || branch.is_some();
    let has_worker = worker.is_some() || version.is_some() || tag.is_some();

    if has_repo && has_worker {
        return Err(
            "specify either {repo, skill, branch?} OR {worker, version?|tag?}, not both".into(),
        );
    }
    if !has_repo && !has_worker {
        return Err("specify either {repo, skill, branch?} OR {worker, version?|tag?}".into());
    }

    if has_repo {
        let repo = repo.ok_or_else(|| "repo is required when skill is set".to_string())?;
        let skill = skill.ok_or_else(|| "skill is required when repo is set".to_string())?;
        let branch = branch.unwrap_or_else(|| DEFAULT_REPO_BRANCH.to_string());
        return Ok(ClassifiedInput::Repo {
            repo,
            skill,
            branch,
        });
    }

    let worker =
        worker.ok_or_else(|| "worker is required when version or tag is set".to_string())?;
    let spec = match (version, tag) {
        (Some(v), None) => VersionSpec::Version(v),
        (None, Some(t)) => VersionSpec::Tag(t),
        (Some(_), Some(_)) => return Err("specify either version OR tag, not both".into()),
        (None, None) => VersionSpec::Tag("latest".into()),
    };
    Ok(ClassifiedInput::Registry { worker, spec })
}

pub(crate) async fn run_download(
    cfg: &SkillsConfig,
    classified: &ClassifiedInput,
) -> Result<DownloadResult, String> {
    let folder = cfg.resolved_skills_folder();
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("create_dir_all {}: {e}", folder.display()))?;

    match classified {
        ClassifiedInput::Repo {
            repo,
            skill,
            branch,
        } => sources::git::download(repo, skill, branch, &folder, cfg.download_timeout_ms).await,
        ClassifiedInput::Registry { worker, spec } => {
            sources::registry::download(
                cfg.registry_base(),
                worker,
                spec,
                &folder,
                cfg.download_timeout_ms,
            )
            .await
        }
    }
}

fn build_output(classified: &ClassifiedInput, result: DownloadResult) -> DownloadOutput {
    let source = match classified {
        ClassifiedInput::Repo {
            repo,
            skill,
            branch,
        } => json!({
            "kind": "repo",
            "repo": repo,
            "skill": skill,
            "branch": branch,
        }),
        ClassifiedInput::Registry { worker, spec } => match spec {
            VersionSpec::Version(v) => json!({
                "kind": "registry",
                "worker": worker,
                "version": v,
            }),
            VersionSpec::Tag(t) => json!({
                "kind": "registry",
                "worker": worker,
                "tag": t,
            }),
        },
    };
    DownloadOutput {
        namespace: result.namespace,
        skills_written: result.skills_written,
        prompts_written: result.prompts_written,
        source,
    }
}

/// Fan out to subscribers. We fire `skills::on-change` only when at
/// least one skill was written (and likewise for prompts) so noisy
/// no-op downloads don't churn MCP `notifications/list_changed`.
async fn fan_out(
    iii: &III,
    skills_subs: &SubscriberSet,
    prompts_subs: &SubscriberSet,
    classified: &ClassifiedInput,
    result: &DownloadResult,
) {
    let payload = json!({
        "op": "download",
        "namespace": result.namespace,
        "source": match classified {
            ClassifiedInput::Repo { .. } => "repo",
            ClassifiedInput::Registry { .. } => "registry",
        },
    });
    if !result.skills_written.is_empty() {
        trigger_types::dispatch(iii, skills_subs, payload.clone()).await;
    }
    if !result.prompts_written.is_empty() {
        trigger_types::dispatch(iii, prompts_subs, payload).await;
    }
}

// ────────────────── completion marker ──────────────────────────────────
//
// After a successful registry download, a `.iii-skill-complete` JSON
// marker is written inside the namespace directory. The reconcile path
// treats a namespace as "present" only if the marker exists — this
// prevents half-downloaded namespaces from hiding a needed re-download.

/// Marker filename written inside a namespace after a complete download.
const COMPLETION_MARKER: &str = ".iii-skill-complete";

/// Marker payload shape: `{ worker, source, tag_or_version, schema }`.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct CompletionMarker {
    worker: String,
    source: String,
    tag_or_version: String,
    schema: u32,
}

/// Write the completion marker under `<skills_folder>/<worker>/`.
fn write_completion_marker(
    skills_folder: &std::path::Path,
    worker: &str,
    spec: &VersionSpec,
) -> Result<(), String> {
    let marker = CompletionMarker {
        worker: worker.to_string(),
        source: "registry".to_string(),
        tag_or_version: match spec {
            VersionSpec::Version(v) => v.clone(),
            VersionSpec::Tag(t) => t.clone(),
        },
        schema: 1,
    };
    let json = serde_json::to_string_pretty(&marker).map_err(|e| format!("encode marker: {e}"))?;
    let dest = skills_folder.join(worker).join(COMPLETION_MARKER);
    sources::write_file_atomic(&dest, json.as_bytes())
}

/// Check if a completion marker exists for `worker` under `skills_folder`.
pub fn has_completion_marker(skills_folder: &std::path::Path, worker: &str) -> bool {
    skills_folder.join(worker).join(COMPLETION_MARKER).exists()
}

// ────────────────── auto-download helper ──────────────────────────────
//
// auto-download flow (ASCII):
//
//   event: worker::add(Done)  ─┐
//                               ├─► download_worker_skills(name, tag=latest)
//   boot reconcile: for each   ─┘       │
//     installed worker w/o marker        ▼
//                              validate name
//                                    │
//                              registry::download_typed
//                                    │
//                    ┌───────────────┼───────────────┐
//                    ▼               ▼               ▼
//               Ok(result)    NotFound(404)    Err(5xx/timeout)
//                    │           (no-op)         (logged warn)
//               write marker
//               invalidate cache

/// Download skills for a single worker from the registry. Validates the
/// name, calls `registry::download_typed`, writes the completion marker
/// on success, and treats 404 as a benign no-op.
///
/// Returns `true` if skills were successfully downloaded.
pub async fn download_worker_skills(
    cfg: &SkillsConfig,
    worker: &str,
    spec: &VersionSpec,
) -> Result<bool, String> {
    use crate::sources::registry;

    registry::validate_worker_name(worker)?;

    let folder = cfg.resolved_skills_folder();
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("create_dir_all {}: {e}", folder.display()))?;

    match registry::download_typed(
        cfg.registry_base(),
        worker,
        spec,
        &folder,
        cfg.download_timeout_ms,
    )
    .await?
    {
        registry::RegistryDownloadOutcome::Ok(result) => {
            tracing::info!(
                worker,
                skills = result.skills_written.len(),
                prompts = result.prompts_written.len(),
                "auto-downloaded worker skills"
            );
            write_completion_marker(&folder, worker, spec)?;
            Ok(true)
        }
        registry::RegistryDownloadOutcome::NotFound => {
            tracing::debug!(worker, "registry 404 — no skills bundle; benign skip");
            Ok(false)
        }
    }
}

// ────────────────── in-flight guard ──────────────────────────────────

use std::collections::HashSet;
use std::sync::Mutex;

/// Per-worker in-flight guard shared between the event handler and the
/// reconciler. Prevents concurrent downloads of the same worker.
pub struct InFlightGuard {
    inner: Mutex<HashSet<String>>,
}

impl Default for InFlightGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl InFlightGuard {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashSet::new()),
        }
    }

    /// Attempt to claim a worker for download. Returns `true` if the
    /// worker was not already in-flight and is now claimed.
    pub fn try_claim(&self, worker: &str) -> bool {
        let mut set = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        set.insert(worker.to_string())
    }

    /// Release a previously claimed worker.
    pub fn release(&self, worker: &str) {
        let mut set = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        set.remove(worker);
    }

    /// Claim a worker for download, returning an RAII guard that
    /// releases the claim on drop (including on panic / early return).
    /// Returns `None` if the worker is already in-flight.
    pub fn claim(self: &Arc<Self>, worker: &str) -> Option<InFlightClaim> {
        if self.try_claim(worker) {
            Some(InFlightClaim {
                guard: Arc::clone(self),
                worker: worker.to_string(),
            })
        } else {
            None
        }
    }
}

/// RAII guard that releases a claimed worker on drop.
pub struct InFlightClaim {
    guard: Arc<InFlightGuard>,
    worker: String,
}

impl Drop for InFlightClaim {
    fn drop(&mut self) {
        self.guard.release(&self.worker);
    }
}

// ────────────────── reconcile decision helper ──────────────────────
//
// Pure logic extracted from `spawn_boot_reconcile` so it can be
// unit-tested without an engine or async runtime.

use std::path::Path;

/// Decide whether a worker from `worker::list` needs a skills download
/// during boot reconcile. Returns `None` to skip, `Some(spec)` to
/// download.
///
/// Skip guards (in order):
///   1. Name doesn't validate → skip.
///   2. Local override directory exists → skip.
///   3. Completion marker already present in global root → skip.
///
/// When not skipped, `version` from the worker info determines the
/// spec: `Some(v)` (non-empty) → `VersionSpec::Version(v)`, else
/// `VersionSpec::Tag("latest")`.
pub fn reconcile_decision(
    name: &str,
    version: Option<&str>,
    local_root: &Path,
    global_root: &Path,
) -> Option<VersionSpec> {
    // Guard 1: invalid name.
    if crate::sources::registry::validate_worker_name(name).is_err() {
        return None;
    }
    // Guard 2: local override exists.
    if local_root.join(name).is_dir() {
        return None;
    }
    // Guard 3: completion marker already present.
    if has_completion_marker(global_root, name) {
        return None;
    }
    // Determine version spec.
    match version {
        Some(v) if !v.is_empty() => Some(VersionSpec::Version(v.to_string())),
        _ => Some(VersionSpec::Tag("latest".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_rejects_empty_input() {
        let err = classify_input(DownloadInput::default()).unwrap_err();
        assert!(err.contains("specify either"), "got: {err}");
    }

    #[test]
    fn classify_rejects_both_sources() {
        let err = classify_input(DownloadInput {
            repo: Some("https://github.com/x/y".into()),
            skill: Some("foo".into()),
            worker: Some("z".into()),
            tag: Some("latest".into()),
            ..DownloadInput::default()
        })
        .unwrap_err();
        assert!(err.contains("not both"), "got: {err}");
    }

    #[test]
    fn classify_rejects_repo_without_skill() {
        let err = classify_input(DownloadInput {
            repo: Some("https://github.com/x/y".into()),
            ..DownloadInput::default()
        })
        .unwrap_err();
        assert!(err.contains("skill"), "got: {err}");
    }

    #[test]
    fn classify_rejects_skill_without_repo() {
        let err = classify_input(DownloadInput {
            skill: Some("foo".into()),
            ..DownloadInput::default()
        })
        .unwrap_err();
        assert!(err.contains("repo"), "got: {err}");
    }

    #[test]
    fn classify_worker_without_version_or_tag_defaults_to_latest() {
        let c = classify_input(DownloadInput {
            worker: Some("resend".into()),
            ..DownloadInput::default()
        })
        .unwrap();
        assert_eq!(
            c,
            ClassifiedInput::Registry {
                worker: "resend".into(),
                spec: VersionSpec::Tag("latest".into()),
            }
        );
    }

    #[test]
    fn classify_rejects_both_version_and_tag() {
        let err = classify_input(DownloadInput {
            worker: Some("resend".into()),
            version: Some("1.2.3".into()),
            tag: Some("latest".into()),
            ..DownloadInput::default()
        })
        .unwrap_err();
        assert!(err.contains("either version OR tag"), "got: {err}");
    }

    #[test]
    fn classify_accepts_repo_form_with_default_branch() {
        let c = classify_input(DownloadInput {
            repo: Some("https://github.com/anthropics/skills".into()),
            skill: Some("frontend-design".into()),
            ..DownloadInput::default()
        })
        .unwrap();
        assert_eq!(
            c,
            ClassifiedInput::Repo {
                repo: "https://github.com/anthropics/skills".into(),
                skill: "frontend-design".into(),
                branch: "main".into(),
            }
        );
    }

    #[test]
    fn classify_accepts_repo_form_with_explicit_branch() {
        let c = classify_input(DownloadInput {
            repo: Some("https://github.com/x/y".into()),
            skill: Some("foo".into()),
            branch: Some("master".into()),
            ..DownloadInput::default()
        })
        .unwrap();
        assert_eq!(
            c,
            ClassifiedInput::Repo {
                repo: "https://github.com/x/y".into(),
                skill: "foo".into(),
                branch: "master".into(),
            }
        );
    }

    #[test]
    fn classify_accepts_registry_with_version() {
        let c = classify_input(DownloadInput {
            worker: Some("resend".into()),
            version: Some("1.2.3".into()),
            ..DownloadInput::default()
        })
        .unwrap();
        assert_eq!(
            c,
            ClassifiedInput::Registry {
                worker: "resend".into(),
                spec: VersionSpec::Version("1.2.3".into()),
            }
        );
    }

    #[test]
    fn classify_accepts_registry_with_tag() {
        let c = classify_input(DownloadInput {
            worker: Some("agent-memory".into()),
            tag: Some("latest".into()),
            ..DownloadInput::default()
        })
        .unwrap();
        assert_eq!(
            c,
            ClassifiedInput::Registry {
                worker: "agent-memory".into(),
                spec: VersionSpec::Tag("latest".into()),
            }
        );
    }

    #[test]
    fn classify_trims_whitespace() {
        let c = classify_input(DownloadInput {
            worker: Some("  resend  ".into()),
            tag: Some("latest\n".into()),
            ..DownloadInput::default()
        })
        .unwrap();
        assert_eq!(
            c,
            ClassifiedInput::Registry {
                worker: "resend".into(),
                spec: VersionSpec::Tag("latest".into()),
            }
        );
    }

    #[test]
    fn build_output_includes_repo_source() {
        let mut result = DownloadResult::new("foo");
        result.skills_written.push("foo/bar.md".into());
        let classified = ClassifiedInput::Repo {
            repo: "https://github.com/x/y".into(),
            skill: "foo".into(),
            branch: "main".into(),
        };
        let out = build_output(&classified, result);
        assert_eq!(out.namespace, "foo");
        assert_eq!(out.source["kind"], "repo");
        assert_eq!(out.source["repo"], "https://github.com/x/y");
        assert_eq!(out.source["skill"], "foo");
        assert_eq!(out.source["branch"], "main");
    }

    #[test]
    fn build_output_includes_registry_source_with_tag() {
        let result = DownloadResult::new("resend");
        let classified = ClassifiedInput::Registry {
            worker: "resend".into(),
            spec: VersionSpec::Tag("latest".into()),
        };
        let out = build_output(&classified, result);
        assert_eq!(out.source["kind"], "registry");
        assert_eq!(out.source["worker"], "resend");
        assert_eq!(out.source["tag"], "latest");
        assert!(out.source.get("version").is_none());
    }

    #[test]
    fn build_output_includes_registry_source_with_version() {
        let result = DownloadResult::new("resend");
        let classified = ClassifiedInput::Registry {
            worker: "resend".into(),
            spec: VersionSpec::Version("1.2.3".into()),
        };
        let out = build_output(&classified, result);
        assert_eq!(out.source["version"], "1.2.3");
        assert!(out.source.get("tag").is_none());
    }

    // ── InFlightGuard / InFlightClaim RAII tests ──────────────────────

    #[test]
    fn in_flight_first_claim_succeeds() {
        let guard = Arc::new(InFlightGuard::new());
        let claim = guard.claim("resend");
        assert!(claim.is_some(), "first claim should succeed");
    }

    #[test]
    fn in_flight_concurrent_claim_blocked() {
        let guard = Arc::new(InFlightGuard::new());
        let _claim = guard.claim("resend").unwrap();
        let second = guard.claim("resend");
        assert!(second.is_none(), "concurrent claim should be blocked");
    }

    #[test]
    fn in_flight_drop_releases_claim() {
        let guard = Arc::new(InFlightGuard::new());
        {
            let _claim = guard.claim("resend").unwrap();
            // _claim drops here
        }
        let re_claim = guard.claim("resend");
        assert!(re_claim.is_some(), "after drop, re-claim should succeed");
    }

    #[test]
    fn in_flight_distinct_workers_both_claim() {
        let guard = Arc::new(InFlightGuard::new());
        let _a = guard.claim("resend").unwrap();
        let b = guard.claim("agent-memory");
        assert!(
            b.is_some(),
            "distinct workers should both claim independently"
        );
    }

    // ── completion marker round-trip ──────────────────────────────────

    #[test]
    fn completion_marker_write_then_read() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path();
        // Create the worker namespace directory so the marker can be written.
        std::fs::create_dir_all(folder.join("resend")).unwrap();
        let spec = VersionSpec::Tag("latest".into());
        write_completion_marker(folder, "resend", &spec).unwrap();
        assert!(
            has_completion_marker(folder, "resend"),
            "marker should be present after write"
        );
        // Verify the JSON content is well-formed and carries expected fields.
        let marker_path = folder.join("resend").join(COMPLETION_MARKER);
        let raw = std::fs::read_to_string(marker_path).unwrap();
        let marker: CompletionMarker = serde_json::from_str(&raw).unwrap();
        assert_eq!(marker.worker, "resend");
        assert_eq!(marker.tag_or_version, "latest");
        assert_eq!(marker.source, "registry");
        assert_eq!(marker.schema, 1);
    }

    #[test]
    fn completion_marker_absent_worker_returns_false() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            !has_completion_marker(tmp.path(), "nonexistent"),
            "absent worker should return false"
        );
    }

    #[test]
    fn completion_marker_version_spec() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("myworker")).unwrap();
        let spec = VersionSpec::Version("2.3.4".into());
        write_completion_marker(tmp.path(), "myworker", &spec).unwrap();
        assert!(has_completion_marker(tmp.path(), "myworker"));
        let raw =
            std::fs::read_to_string(tmp.path().join("myworker").join(COMPLETION_MARKER)).unwrap();
        let marker: CompletionMarker = serde_json::from_str(&raw).unwrap();
        assert_eq!(marker.tag_or_version, "2.3.4");
    }

    // ── download_worker_skills (wiremock integration) ──────────────

    #[tokio::test]
    async fn download_worker_skills_200_writes_marker() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let body = serde_json::json!({
            "name": "resend",
            "version": "1.0.0",
            "skills": [{"path": "index.md", "content": "# resend\n"}],
            "prompts": []
        });
        Mock::given(method("GET"))
            .and(path("/w/resend/skills"))
            .and(query_param("version", "latest"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let cfg = SkillsConfig {
            skills_folder: tmp.path().to_string_lossy().into_owned(),
            registry_url: server.uri(),
            ..SkillsConfig::default()
        };
        let spec = VersionSpec::Tag("latest".into());
        let result = download_worker_skills(&cfg, "resend", &spec).await;
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
        assert!(result.unwrap());
        assert!(
            has_completion_marker(&cfg.resolved_skills_folder(), "resend"),
            "completion marker should be written after successful download"
        );
    }

    #[tokio::test]
    async fn download_worker_skills_404_no_marker() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/w/missing/skills"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let cfg = SkillsConfig {
            skills_folder: tmp.path().to_string_lossy().into_owned(),
            registry_url: server.uri(),
            ..SkillsConfig::default()
        };
        let spec = VersionSpec::Tag("latest".into());
        let result = download_worker_skills(&cfg, "missing", &spec).await;
        assert!(!result.unwrap());
        assert!(
            !has_completion_marker(&cfg.resolved_skills_folder(), "missing"),
            "no marker should be written on 404"
        );
    }

    #[tokio::test]
    async fn download_worker_skills_invalid_name_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = SkillsConfig {
            skills_folder: tmp.path().to_string_lossy().into_owned(),
            ..SkillsConfig::default()
        };
        let spec = VersionSpec::Tag("latest".into());
        let result = download_worker_skills(&cfg, "INVALID", &spec).await;
        assert!(
            result.is_err(),
            "invalid worker name should error before HTTP"
        );
    }

    // ── RegisteredWorkersCache::invalidate ────────────────────────────

    #[tokio::test]
    async fn registered_workers_cache_invalidate_clears_state() {
        use crate::functions::skills::RegisteredWorkersCache;

        let cache = RegisteredWorkersCache::new(60_000);
        // Manually populate through the inner mutex.
        {
            let mut lock = cache.inner.lock().await;
            *lock = Some(crate::functions::skills::CacheEntry {
                workers: HashSet::from(["test".to_string()]),
                fetched_at: std::time::Instant::now(),
            });
        }
        cache.invalidate().await;
        {
            let lock = cache.inner.lock().await;
            assert!(lock.is_none(), "invalidate should clear the cache entry");
        }
    }

    // ── reconcile_decision ────────────────────────────────────────────

    #[test]
    fn reconcile_skips_invalid_name() {
        let tmp = tempfile::tempdir().unwrap();
        let result = reconcile_decision("INVALID", None, tmp.path(), tmp.path());
        assert!(result.is_none(), "invalid name should be skipped");
    }

    #[test]
    fn reconcile_skips_local_override() {
        let tmp = tempfile::tempdir().unwrap();
        let local_root = tmp.path().join("local");
        let global_root = tmp.path().join("global");
        // Create a local override directory.
        std::fs::create_dir_all(local_root.join("resend")).unwrap();
        std::fs::create_dir_all(&global_root).unwrap();
        let result = reconcile_decision("resend", None, &local_root, &global_root);
        assert!(result.is_none(), "local override should skip download");
    }

    #[test]
    fn reconcile_skips_existing_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let global_root = tmp.path().join("global");
        let local_root = tmp.path().join("local");
        std::fs::create_dir_all(global_root.join("resend")).unwrap();
        std::fs::create_dir_all(&local_root).unwrap();
        // Write a completion marker.
        write_completion_marker(&global_root, "resend", &VersionSpec::Tag("latest".into()))
            .unwrap();
        let result = reconcile_decision("resend", None, &local_root, &global_root);
        assert!(result.is_none(), "existing marker should skip download");
    }

    #[test]
    fn reconcile_returns_version_spec_when_version_present() {
        let tmp = tempfile::tempdir().unwrap();
        let result = reconcile_decision("resend", Some("2.0.0"), tmp.path(), tmp.path());
        assert_eq!(
            result,
            Some(VersionSpec::Version("2.0.0".to_string())),
            "should return Version spec when version is provided"
        );
    }

    #[test]
    fn reconcile_returns_latest_tag_when_no_version() {
        let tmp = tempfile::tempdir().unwrap();
        let result = reconcile_decision("resend", None, tmp.path(), tmp.path());
        assert_eq!(
            result,
            Some(VersionSpec::Tag("latest".to_string())),
            "should return latest tag when no version"
        );
    }

    #[test]
    fn reconcile_returns_latest_tag_when_empty_version() {
        let tmp = tempfile::tempdir().unwrap();
        let result = reconcile_decision("resend", Some(""), tmp.path(), tmp.path());
        assert_eq!(
            result,
            Some(VersionSpec::Tag("latest".to_string())),
            "empty version string should fall back to latest"
        );
    }
}
